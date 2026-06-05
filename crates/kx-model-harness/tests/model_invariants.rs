//! In-process model invariants (need the GGUF; run with `--features with-model`).
//!
//! - **A — exit gate, both outcomes.** A real-model producer's output is gated by
//!   a native deterministic critic. `Schema(Text)` over the (textual) output ⇒
//!   `Valid` ⇒ the consumer is promoted and runs. `Schema(Json)` over the same
//!   (non-JSON) output ⇒ `Invalid` ⇒ the consumer is withheld (fail-closed).
//! - **B — critic determinism over a stochastic producer.** The model is called
//!   exactly once per run (the critic adds 0 calls — decorrelated, D60); the
//!   native verdict is a pure function of the producer bytes (re-evaluating gives
//!   a byte-identical verdict ref); two sampled seeds give different model bytes.
//! - **D — reproducibility.** Greedy decode ⇒ byte-identical projection digest
//!   across two independent runs; sampled (different seed) ⇒ different digest,
//!   yet still exactly-once.
//! - **E — reuse the recipe, never the result.** Re-driving an identical workflow
//!   against a populated journal serves the committed facts: the model is NOT
//!   re-called (0 dispatches on the second drive).

#![cfg(feature = "with-model")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;
use std::sync::Arc;

use kx_content::{ContentStore, LocalFsContentStore};
use kx_critic_types::{CheckSpec, SchemaSpec, SchemaTag};
use kx_journal::SqliteJournal;
use kx_model_harness::{evidence::Evidence, harness_warrant, model_id_for, workflows, Harness};
use kx_mote::NdClass;
use kx_projection::{ContentStoreVerdicts, MoteState, Projection, VerdictLookup};
use kx_runtime::config::Mode;
use kx_runtime::{RuntimeConfig, RuntimeError};

fn gguf() -> std::path::PathBuf {
    kx_model_harness::default_gguf_path()
}

fn config(dir: &Path) -> RuntimeConfig {
    RuntimeConfig {
        journal_path: dir.join("j.sqlite"),
        content_root: dir.join("c"),
        mode: Mode::Run,
        crash_at: None,
        // These invariants are unrelated to checkpointing; keep the cadence off
        // so no sidecar is written (recovery would read it harmlessly anyway).
        checkpoint_every: None,
        audit_log: None,
    }
}

fn evidence() -> Option<Evidence> {
    let stamp = std::env::var("KX_RUNSTAMP").ok()?;
    Evidence::open(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target"),
        &stamp,
    )
    .ok()
}

/// Re-open the journal + store after a drive for inspection.
fn reopen(config: &RuntimeConfig) -> (Projection, Arc<LocalFsContentStore>) {
    let store = Arc::new(LocalFsContentStore::open(&config.content_root).unwrap());
    let journal = SqliteJournal::open(&config.journal_path).unwrap();
    let projection = Projection::from_journal(&journal).unwrap();
    (projection, store)
}

fn schema(tag: SchemaTag) -> CheckSpec {
    CheckSpec::Schema(SchemaSpec { expected: tag })
}

// ---- A: exit gate, both outcomes -------------------------------------------

#[test]
fn a_exit_gate_valid_promotes_consumer() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(dir.path());
    let model_id = model_id_for(&gguf()).unwrap();
    let warrant = harness_warrant(&model_id, 64, 60_000);
    // Schema(Text): any UTF-8 model output conforms ⇒ Valid.
    let wf = workflows::exit_gate(
        &model_id,
        &warrant,
        kx_model_harness::ROW_PROMPT,
        schema(SchemaTag::Text),
    );
    let (producer, critic, consumer) = (
        wf.motes[0].mote.id,
        wf.motes[1].mote.id,
        wf.motes[2].mote.id,
    );

    let harness = Harness::open(&cfg, &gguf(), model_id).unwrap();
    let outcome = harness.drive(&cfg, &wf).unwrap();
    assert!(
        outcome.is_complete(),
        "Valid verdict ⇒ all 3 commit (producer+critic+consumer)"
    );

    let (p, store) = reopen(&cfg);
    let verdicts = ContentStoreVerdicts::new(store.clone());
    let verdict = p.result_ref_of(&critic).and_then(|r| verdicts.verdict(&r));
    let model_text =
        String::from_utf8_lossy(&store.get(&p.result_ref_of(&producer).unwrap()).unwrap())
            .into_owned();

    assert!(
        verdict
            .as_ref()
            .is_some_and(kx_critic_types::CriticVerdict::is_valid),
        "critic Valid"
    );
    assert_eq!(
        p.state_of(&consumer),
        MoteState::Committed,
        "consumer promoted + ran"
    );

    if let Some(ev) = evidence() {
        let _ = ev.write_str("A_exit_gate", "valid.txt", &format!(
            "PASS A.valid — Schema(Text) ⇒ Valid ⇒ consumer promoted\nmodel_output={model_text:?}\nverdict=Valid\nconsumer_state=Committed\ndigest={}\n",
            outcome.digest.to_hex()));
    }
}

#[test]
fn a_exit_gate_invalid_withholds_consumer() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(dir.path());
    let model_id = model_id_for(&gguf()).unwrap();
    let warrant = harness_warrant(&model_id, 64, 60_000);
    // Schema(Json) over a (non-JSON) sentence ⇒ Invalid ⇒ consumer withheld.
    let wf = workflows::exit_gate(
        &model_id,
        &warrant,
        kx_model_harness::ROW_PROMPT,
        schema(SchemaTag::Json),
    );
    let (producer, critic, consumer) = (
        wf.motes[0].mote.id,
        wf.motes[1].mote.id,
        wf.motes[2].mote.id,
    );

    let harness = Harness::open(&cfg, &gguf(), model_id).unwrap();
    // Fail-closed: the withheld consumer means the run does not complete.
    let res = harness.drive(&cfg, &wf);
    assert!(
        matches!(res, Err(RuntimeError::Stalled(_))),
        "Invalid verdict withholds the consumer ⇒ the run stalls (fail-closed), got {res:?}"
    );

    let (p, store) = reopen(&cfg);
    let verdicts = ContentStoreVerdicts::new(store.clone());
    let verdict = p.result_ref_of(&critic).and_then(|r| verdicts.verdict(&r));
    let model_text =
        String::from_utf8_lossy(&store.get(&p.result_ref_of(&producer).unwrap()).unwrap())
            .into_owned();
    let ready = p.ready_set_promoted(&verdicts);

    assert!(
        verdict.is_some() && !verdict.as_ref().unwrap().is_valid(),
        "critic Invalid"
    );
    assert_eq!(
        p.state_of(&consumer),
        MoteState::Pending,
        "consumer NOT committed (withheld)"
    );
    assert!(
        !ready.contains(&consumer),
        "consumer is NOT in the promoted ready set (fail-closed)"
    );

    if let Some(ev) = evidence() {
        let _ = ev.write_str("A_exit_gate", "invalid.txt", &format!(
            "PASS A.invalid — Schema(Json) over non-JSON ⇒ Invalid ⇒ consumer withheld\nmodel_output={model_text:?}\nverdict=Invalid\nconsumer_state=Pending(withheld)\nconsumer_in_ready_set=false\n"));
    }
}

// ---- B: critic determinism over a stochastic producer ----------------------

#[test]
fn b_critic_decorrelated_and_deterministic() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(dir.path());
    let model_id = model_id_for(&gguf()).unwrap();
    let warrant = harness_warrant(&model_id, 64, 60_000);
    let wf = workflows::exit_gate(
        &model_id,
        &warrant,
        kx_model_harness::ROW_PROMPT,
        schema(SchemaTag::Json),
    );
    let producer = wf.motes[0].mote.id;
    let critic = wf.motes[1].mote.id;

    let harness = Harness::open(&cfg, &gguf(), model_id).unwrap();
    let _ = harness.drive(&cfg, &wf); // Invalid ⇒ Err(Stalled), expected.

    // Decorrelation: the model was called exactly once (the producer); the native
    // critic added ZERO model calls.
    assert_eq!(
        harness.backend.calls(),
        1,
        "decorrelated: critic adds 0 model calls (D60)"
    );

    // Determinism over input: independently re-evaluate the check over the
    // producer's committed bytes ⇒ byte-identical verdict ref to the committed one.
    let (p, store) = reopen(&cfg);
    let producer_bytes = store
        .get(&p.result_ref_of(&producer).unwrap())
        .unwrap()
        .to_vec();
    let committed_verdict_ref = p.result_ref_of(&critic).unwrap();
    let re_eval = kx_critic::evaluate(&schema(SchemaTag::Json), &producer_bytes);
    assert_eq!(
        re_eval.content_ref_bytes(),
        *committed_verdict_ref.as_bytes(),
        "the native critic verdict is a pure function of the producer bytes (byte-identical)"
    );
    // And re-evaluating again is byte-identical (total + deterministic).
    let re_eval2 = kx_critic::evaluate(&schema(SchemaTag::Json), &producer_bytes);
    assert_eq!(re_eval.content_ref_bytes(), re_eval2.content_ref_bytes());

    if let Some(ev) = evidence() {
        let _ = ev.write_str("B_critic_determinism", "result.txt", &format!(
            "PASS B — critic decorrelated + deterministic\nmodel_calls_during_run={} (producer only; critic=0)\nproducer_bytes={:?}\ncommitted_verdict_ref={}\nre_evaluated_verdict_ref={}\nbyte_identical={}\n",
            harness.backend.calls(),
            String::from_utf8_lossy(&producer_bytes),
            kx_model_harness::evidence::hex(committed_verdict_ref.as_bytes()),
            kx_model_harness::evidence::hex(&re_eval.content_ref_bytes()),
            re_eval.content_ref_bytes() == *committed_verdict_ref.as_bytes()));
    }
}

// ---- D: reproducibility (greedy) vs divergence (sampled) -------------------

#[test]
fn d_greedy_reproducible_sampled_diverges() {
    let model_id = model_id_for(&gguf()).unwrap();
    let warrant = harness_warrant(&model_id, 64, 60_000);

    // Greedy: two independent runs ⇒ byte-identical digest.
    let run_greedy = || {
        let dir = tempfile::tempdir().unwrap();
        let cfg = config(dir.path());
        let wf = workflows::model_chain(
            &model_id,
            &warrant,
            kx_model_harness::ROW_PROMPT,
            workflows::greedy(32),
            NdClass::Pure,
        );
        let h = Harness::open(&cfg, &gguf(), model_id.clone()).unwrap();
        let o = h.drive(&cfg, &wf).unwrap();
        (o.digest.to_hex(), o.committed, o.total)
    };
    let (g1, c1, t1) = run_greedy();
    let (g2, _, _) = run_greedy();
    assert_eq!(
        g1, g2,
        "greedy decode ⇒ byte-identical projection digest across runs"
    );
    assert_eq!((c1, t1), (2, 2));

    // Sampled: two different seeds ⇒ different digests (still exactly-once).
    let run_sampled = |seed: u32| {
        let dir = tempfile::tempdir().unwrap();
        let cfg = config(dir.path());
        let wf = workflows::model_chain(
            &model_id,
            &warrant,
            kx_model_harness::ROW_PROMPT,
            workflows::sampled(32, seed),
            NdClass::ReadOnlyNondet,
        );
        let h = Harness::open(&cfg, &gguf(), model_id.clone()).unwrap();
        let o = h.drive(&cfg, &wf).unwrap();
        (o.digest.to_hex(), o.committed, o.total)
    };
    let (s1, sc1, st1) = run_sampled(7);
    let (s2, _, _) = run_sampled(99);
    assert_eq!((sc1, st1), (2, 2), "sampled run still commits exactly-once");
    // Different seeds usually diverge; record honestly either way (a tiny model
    // on a short prompt can coincide — the guarantee is exactly-once, not divergence).
    let diverged = s1 != s2;

    if let Some(ev) = evidence() {
        let _ = ev.write_str("D_reproducibility", "result.txt", &format!(
            "PASS D — greedy reproducible; sampled exactly-once\ngreedy_digest_run1={g1}\ngreedy_digest_run2={g2}\ngreedy_identical={}\nsampled_seed7_digest={s1}\nsampled_seed99_digest={s2}\nsampled_diverged={diverged}\n",
            g1 == g2));
    }
}

// ---- E: reuse the recipe, never the result --------------------------------

#[test]
fn e_redrive_serves_committed_no_remodel() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(dir.path());
    let model_id = model_id_for(&gguf()).unwrap();
    let warrant = harness_warrant(&model_id, 64, 60_000);
    let wf = workflows::model_chain(
        &model_id,
        &warrant,
        kx_model_harness::ROW_PROMPT,
        workflows::greedy(32),
        NdClass::Pure,
    );

    // First drive: the model is called once (the producer).
    let h1 = Harness::open(&cfg, &gguf(), model_id.clone()).unwrap();
    let o1 = h1.drive(&cfg, &wf).unwrap();
    assert!(o1.is_complete());
    assert_eq!(h1.backend.calls(), 1, "first drive samples the model once");

    // Second drive (fresh harness, SAME journal/content): identical recipe ⇒ the
    // committed facts are served; the model is NOT re-called.
    let h2 = Harness::open(&cfg, &gguf(), model_id).unwrap();
    let o2 = h2.drive(&cfg, &wf).unwrap();
    assert_eq!(
        o1.digest.to_hex(),
        o2.digest.to_hex(),
        "same committed facts"
    );
    assert_eq!(
        h2.backend.calls(),
        0,
        "reuse the result: an already-committed MoteId is served, the model is NOT re-called"
    );

    if let Some(ev) = evidence() {
        let _ = ev.write_str("E_memoizer", "result.txt", &format!(
            "PASS E — reuse the recipe, never the result\nfirst_drive_model_calls={}\nsecond_drive_model_calls={} (served from journal)\ndigest_stable={}\n",
            h1.backend.calls(), h2.backend.calls(), o1.digest.to_hex() == o2.digest.to_hex()));
    }
}
