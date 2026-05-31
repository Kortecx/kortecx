//! Crash + recovery rows (need the GGUF; run with `--features with-model`).
//!
//! - **C — serve-not-re-sample (the centerpiece).** A stochastic (ROND) model
//!   producer commits, then the process is killed (`post-commit-vtc`). A fresh
//!   `replay` must SERVE the committed `result_ref` — never re-sample (model
//!   dispatch count = 0 in the recovery process) — and the committed `result_ref`
//!   is byte-identical before and after. Exactly-once: one `Committed` per Mote.
//! - **G — tool idempotency.** A WM `StageThenCommit` tool Mote is killed after
//!   the effect is staged (`pre-commit-stc`); `replay` re-dispatches and the
//!   content-addressed dedup makes the effect exactly-once (one `Committed`, and
//!   its idempotency key == the Mote id, D38 §1).
//!
//! These spawn the `kx-model-harness` binary as a subprocess because the crash is
//! a real `process::abort` — it must not kill the test process.

#![cfg(feature = "with-model")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::{Command, Output};

use kx_content::LocalFsContentStore;
use kx_journal::{Journal, JournalEntry, SqliteJournal};
use kx_mote::MoteId;
use kx_model_harness::{evidence::Evidence, harness_warrant, model_id_for, workflow_for_row};
use kx_projection::{MoteState, Projection};

const SEED: u32 = 7;

fn gguf() -> String {
    kx_model_harness::default_gguf_path().to_string_lossy().into_owned()
}

fn invoke(mode: &str, journal: &str, content: &str, row: &str, crash_at: Option<&str>) -> Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_kx-model-harness"));
    c.args([
        mode, "--journal", journal, "--content", content, "--gguf", &gguf(), "--row", row,
        "--seed", "7",
    ]);
    if let Some(ca) = crash_at {
        c.args(["--crash-at", ca]);
    }
    c.output().unwrap()
}

fn kv(out: &Output, key: &str) -> Option<String> {
    let s = String::from_utf8_lossy(&out.stdout);
    s.lines()
        .find_map(|l| l.strip_prefix(&format!("{key}=")).map(str::to_string))
}

fn fold(journal: &str, content: &str) -> (Projection, SqliteJournal) {
    let _store = LocalFsContentStore::open(Path::new(content)).unwrap();
    let j = SqliteJournal::open(Path::new(journal)).unwrap();
    let p = Projection::from_journal(&j).unwrap();
    (p, j)
}

/// Count `Committed` journal entries for `mote` (exactly-once evidence).
fn committed_count(j: &SqliteJournal, mote: &MoteId) -> usize {
    let current = j.current_seq().unwrap();
    j.read_entries_by_seq(0..(current + 1))
        .unwrap()
        .filter(|e| matches!(e, JournalEntry::Committed { mote_id, .. } if mote_id == mote))
        .count()
}

/// The committed entry's idempotency key for `mote`, if present.
fn committed_idempotency_key(j: &SqliteJournal, mote: &MoteId) -> Option<[u8; 32]> {
    let current = j.current_seq().unwrap();
    j.read_entries_by_seq(0..(current + 1))
        .unwrap()
        .find_map(|e| match e {
            JournalEntry::Committed {
                mote_id,
                idempotency_key,
                ..
            } if mote_id == *mote => Some(idempotency_key),
            _ => None,
        })
}

fn evidence() -> Option<Evidence> {
    let stamp = std::env::var("KX_RUNSTAMP").ok()?;
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target");
    Evidence::open(&base, &stamp).ok()
}

#[test]
fn c_serve_not_resample_under_crash() {
    let dir = tempfile::tempdir().unwrap();
    let journal = dir.path().join("c.sqlite");
    let content = dir.path().join("c.store");
    let (j, c) = (journal.to_str().unwrap(), content.to_str().unwrap());

    // The producer (the model Mote that must not be re-sampled) is the serve
    // workflow's vtc_crash_target.
    let model_id = model_id_for(Path::new(&gguf())).unwrap();
    let warrant = harness_warrant(&model_id, 64, 60_000);
    let workflow = workflow_for_row("serve", &model_id, &warrant, SEED).unwrap();
    let producer = workflow.vtc_crash_target;

    // [1] Run with a crash the instant the producer's Committed is durable.
    let run = invoke("run", j, c, "serve", Some("post-commit-vtc"));
    assert!(!run.status.success(), "run must abort at the crash point");
    assert_eq!(
        run.status.signal(),
        Some(6),
        "crash is a SIGABRT (process::abort)"
    );

    // After the crash: producer committed exactly once; capture its result_ref.
    let (p_pre, j_pre) = fold(j, c);
    assert_eq!(
        p_pre.state_of(&producer),
        MoteState::Committed,
        "producer committed before the crash"
    );
    let ref_pre = p_pre.result_ref_of(&producer).expect("producer result_ref");
    assert_eq!(
        committed_count(&j_pre, &producer),
        1,
        "exactly-once: one Committed for the producer"
    );

    // [2] Replay in a fresh process: must SERVE the committed result, never
    //     re-sample. The recovery process's model dispatch count is 0.
    let replay = invoke("replay", j, c, "serve", None);
    assert!(replay.status.success(), "replay recovers cleanly");
    assert_eq!(kv(&replay, "CALLS").as_deref(), Some("0"),
        "THE PROOF: the committed non-deterministic result was SERVED, not re-sampled (0 model calls on recovery)");
    assert_eq!(kv(&replay, "COMMITTED").as_deref(), Some("2/2"));

    // After replay: producer's committed result_ref is byte-identical, still once.
    let (p_post, j_post) = fold(j, c);
    let ref_post = p_post.result_ref_of(&producer).expect("producer result_ref");
    assert_eq!(
        ref_pre, ref_post,
        "the served result_ref is byte-identical across the crash (not re-sampled)"
    );
    assert_eq!(committed_count(&j_post, &producer), 1, "still exactly-once");

    if let Some(ev) = evidence() {
        let _ = ev.write_str(
            "C_crash_replay",
            "result.txt",
            &format!(
                "PASS C — serve-not-re-sample\nrun_exit_signal=SIGABRT(6)\nproducer={:?}\nresult_ref_pre={}\nresult_ref_post={}\nresult_ref_identical={}\nreplay_model_calls={}\ncommitted_count={}\nreplay_stdout:\n{}\n",
                producer,
                kx_model_harness::evidence::hex(ref_pre.as_bytes()),
                kx_model_harness::evidence::hex(ref_post.as_bytes()),
                ref_pre == ref_post,
                kv(&replay, "CALLS").unwrap_or_default(),
                committed_count(&j_post, &producer),
                String::from_utf8_lossy(&replay.stdout),
            ),
        );
    }
}

#[test]
fn g_tool_idempotent_no_double_fire() {
    let dir = tempfile::tempdir().unwrap();
    let journal = dir.path().join("g.sqlite");
    let content = dir.path().join("g.store");
    let (j, c) = (journal.to_str().unwrap(), content.to_str().unwrap());

    let model_id = model_id_for(Path::new(&gguf())).unwrap();
    let warrant = harness_warrant(&model_id, 64, 60_000);
    let workflow = workflow_for_row("tool", &model_id, &warrant, SEED).unwrap();
    let tool = workflow.stc_crash_target;

    // [1] Crash after the effect is staged, before Committed.
    let run = invoke("run", j, c, "tool", Some("pre-commit-stc"));
    assert!(!run.status.success(), "run aborts after staging");
    assert_eq!(run.status.signal(), Some(6), "SIGABRT");

    // [2] Replay re-dispatches; content-addressed dedup ⇒ exactly-once.
    let replay = invoke("replay", j, c, "tool", None);
    assert!(replay.status.success(), "replay recovers");
    assert_eq!(kv(&replay, "COMMITTED").as_deref(), Some("1/1"));

    let (p, jj) = fold(j, c);
    assert_eq!(p.state_of(&tool), MoteState::Committed);
    assert_eq!(
        committed_count(&jj, &tool),
        1,
        "exactly-once: one Committed despite the re-dispatch (no double-fire)"
    );
    // D38 §1: the idempotency key IS the Mote id.
    assert_eq!(
        committed_idempotency_key(&jj, &tool),
        Some(*tool.as_bytes()),
        "idempotency key == Mote id (D38 §1)"
    );

    if let Some(ev) = evidence() {
        let _ = ev.write_str(
            "G_tool",
            "result.txt",
            &format!(
                "PASS G — tool idempotency (no double-fire)\ntool={:?}\ncommitted_count={}\nidempotency_key_eq_mote_id={}\nreplay_dispatches={}\nreplay_stdout:\n{}\n",
                tool,
                committed_count(&jj, &tool),
                committed_idempotency_key(&jj, &tool) == Some(*tool.as_bytes()),
                kv(&replay, "DISPATCHES").unwrap_or_default(),
                String::from_utf8_lossy(&replay.stdout),
            ),
        );
    }
}
