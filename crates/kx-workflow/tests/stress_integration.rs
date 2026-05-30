//! Integrated Morphic-at-scale stress harness
//! (P4.1 scale & performance validation campaign).
//!
//! `#[ignore]`d; run explicitly in RELEASE:
//!
//! ```text
//! cargo test -p kx-workflow --release --test stress_integration \
//!     -- --ignored --nocapture --test-threads=1
//! ```
//!
//! **H5** drives the ENTIRE P4.1 feature set as one pipeline, at scale, and
//! asserts every invariant still holds under load:
//!
//! * **4.1a compile → DAG** — compile a large recipe twice; the emitted `MoteId`
//!   vectors are byte-identical (pure/deterministic compile at scale).
//! * **4.1b e2e run** — run the compiled PURE DAG through the real runtime twice;
//!   the committed journal digests are byte-identical (reproducible execution).
//! * **4.1c kx-dataset seam** — populate an [`InMemoryDataStore`] +
//!   [`InMemoryRetrievalIndex`] with thousands of typed vectors; top-k retrieval
//!   is deterministic and the corpus's [`DatasetId`] is pure over rows + lineage.
//! * **4.1d graph-RAG retrieval Mote (SN-8)** — at volume, the committed
//!   retrieval *fact* is the ordered ref set ONLY: two hit sets with identical
//!   refs but different similarity scores encode to the SAME fact / `ContentRef`.
//! * **4.1e sharing manifest** — the recipe-as-product [`Manifest`] (with the
//!   produced corpus pinned) has a `ManifestId` reproducible by reference across
//!   the two independent compiles.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::time::Instant;

use kx_dataset::{
    ContentSchema, DataStore, Dataset, DatasetId, Hit, InMemoryDataStore, InMemoryRetrievalIndex,
    RetrievalIndex, TypedRef,
};
use kx_executor::{run_pure_mote, LocalResourceManager, TestMoteExecutor};
use kx_journal::SqliteJournal;
use kx_mote::{EdgeMeta, LogicRef, ModelId, MoteId, ToolName};
use kx_projection::Projection;
use kx_runtime::digest_journal;
use kx_scheduler::{LocalPlacement, Scheduler};
use kx_workflow::{
    compile, encode_retrieval_fact, permissive_warrant, retrieval_result_ref, transform,
    CompiledWorkflow, Manifest, WorkflowDef,
};

/// (recipe steps, indexed vectors) per scaling point.
const POINTS: &[(usize, usize)] = &[(1_000, 10_000), (5_000, 10_000)];
/// Embedding dimensionality for the retrieval corpus.
const DIM: usize = 64;
/// Top-k for the retrieval query.
const K: usize = 16;
/// Workflow seed (folded into entrypoint identity, D50).
const SEED: u32 = 0x4D6F_7270; // "Morp"

/// Build a PURE chain `WorkflowDef` of `n` transform steps (the recipe whose
/// compiled `MoteId`s form the shareable manifest).
fn chain_workflow(n: usize) -> WorkflowDef {
    let model = ModelId("local".into());
    let cap = ToolName("synth".into());
    let warrant = permissive_warrant(model.clone());
    let mut wf = WorkflowDef::new(SEED);
    let mut prev = None;
    for i in 0..n {
        let mut logic = [0u8; 32];
        logic[..8].copy_from_slice(&(i as u64).to_le_bytes());
        let step = wf.add_step(transform(
            LogicRef::from_bytes(logic),
            model.clone(),
            warrant.clone(),
            cap.clone(),
        ));
        if let Some(p) = prev {
            wf.add_edge(p, step, EdgeMeta::data()).unwrap();
        }
        prev = Some(step);
    }
    wf
}

/// Deterministic, RNG-free embedding for row `i` (no clock/host/PID — the whole
/// corpus regenerates byte-identically). The mixing constant is coprime to the
/// prime modulus (65_521 = 2^16 − 15) and the modulus exceeds the corpus size,
/// so component 0 — and therefore every row's vector — is UNIQUE per `i` (no
/// content-ref collisions that the idempotent store would dedup).
fn embedding(i: usize) -> Vec<f32> {
    (0..DIM)
        .map(|j| (((i * 1_000_003 + j * 31) % 65_521) as f32) / 65_521.0)
        .collect()
}

fn embedding_bytes(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }
    out
}

/// Drive a compiled PURE workflow through the real runtime surface; return
/// (committed count, journal digest hex).
fn run_compiled(compiled: &CompiledWorkflow) -> (usize, String) {
    let journal = SqliteJournal::open_in_memory().unwrap();
    let rm = LocalResourceManager::dev_defaults();
    let executor = TestMoteExecutor::deterministic();

    let mut projection = Projection::new();
    let mut scheduler = Scheduler::new(LocalPlacement);
    for cm in &compiled.motes {
        scheduler
            .submit(cm.mote.clone(), cm.warrant.clone(), &mut projection)
            .unwrap();
    }
    for cm in &compiled.motes {
        run_pure_mote(&cm.mote, &cm.warrant, &journal, &rm, &executor).unwrap();
    }
    let committed = Projection::from_journal(&journal)
        .unwrap()
        .committed_count();
    let digest = digest_journal(&journal).unwrap().to_hex();
    (committed, digest)
}

/// Populate a typed store + retrieval index with `n` deterministic embedding
/// vectors. Returns (store rows in insertion order, the index).
fn build_corpus(n: usize) -> (Vec<TypedRef>, InMemoryRetrievalIndex) {
    let store = InMemoryDataStore::new();
    let mut index = InMemoryRetrievalIndex::new();
    let mut rows = Vec::with_capacity(n);
    for i in 0..n {
        let v = embedding(i);
        let row = store
            .put_typed(
                &embedding_bytes(&v),
                ContentSchema::Vector { dim: DIM as u32 },
            )
            .unwrap();
        index.insert(row.content_ref, v);
        rows.push(row);
    }
    (rows, index)
}

#[test]
#[ignore = "stress: run with --release --ignored --nocapture --test-threads=1"]
fn h5_integrated_pipeline_at_scale() {
    let mut all_ok = true;
    for &(steps, vecs) in POINTS {
        // --- 4.1a: compile the recipe twice; identity is byte-stable at scale. ---
        let wf = chain_workflow(steps);
        let c_start = Instant::now();
        let c1 = compile(&wf).unwrap();
        let compile_ms = c_start.elapsed().as_millis();
        let c2 = compile(&wf).unwrap();
        let ids1: Vec<MoteId> = c1.motes.iter().map(|m| m.mote.id).collect();
        let ids2: Vec<MoteId> = c2.motes.iter().map(|m| m.mote.id).collect();
        let compile_deterministic = ids1 == ids2;
        assert!(
            compile_deterministic,
            "compile is deterministic at {steps} steps"
        );
        assert_eq!(ids1.len(), steps, "one Mote per step");

        // --- 4.1b: run the compiled PURE DAG twice; digests match. ---
        let r_start = Instant::now();
        let (committed1, digest1) = run_compiled(&c1);
        let run_ms = r_start.elapsed().as_millis();
        let (committed2, digest2) = run_compiled(&c2);
        assert_eq!(committed1, steps, "all steps commit");
        assert_eq!(committed2, steps);
        let run_deterministic = digest1 == digest2;
        assert!(
            run_deterministic,
            "runtime execution is reproducible at {steps} steps"
        );

        // --- 4.1c: corpus under load; retrieval is deterministic + tie-stable. ---
        let idx_start = Instant::now();
        let (rows, index) = build_corpus(vecs);
        let index_build_ms = idx_start.elapsed().as_millis();
        assert_eq!(index.len(), vecs, "every vector indexed");

        let query = embedding(vecs / 2); // a real corpus point → exact + near hits
        let q_start = Instant::now();
        let hits_a = index.query(&query, K);
        let query_ms = q_start.elapsed().as_millis();
        let hits_b = index.query(&query, K);
        assert_eq!(hits_a.len(), K, "top-k returns k hits");
        let retrieval_deterministic = hits_a == hits_b;
        assert!(
            retrieval_deterministic,
            "retrieval order is deterministic + tie-stable at {vecs} vectors"
        );

        // --- 4.1d: SN-8 — the committed fact is score-INDEPENDENT at volume. ---
        let perturbed: Vec<Hit> = hits_a
            .iter()
            .map(|h| Hit {
                id: h.id,
                score: h.score + 1.0, // different scores, identical neighbour set
            })
            .collect();
        let fact_a = encode_retrieval_fact(&hits_a);
        let fact_b = encode_retrieval_fact(&perturbed);
        let sn8_holds =
            fact_a == fact_b && retrieval_result_ref(&hits_a) == retrieval_result_ref(&perturbed);
        assert!(
            sn8_holds,
            "SN-8: similarity scores never leak into the committed fact at scale"
        );

        // --- 4.1c: DatasetId is pure over rows + lineage (compile-independent). ---
        let corpus1 = Dataset::new(rows.clone(), ids1.clone());
        let corpus2 = Dataset::new(rows.clone(), ids2.clone());
        let dataset_id: DatasetId = corpus1.id();
        assert_eq!(
            dataset_id,
            corpus2.id(),
            "DatasetId is pure over rows + lineage"
        );

        // --- 4.1e: the recipe-as-product manifest is reproducible by reference. ---
        let manifest1 = Manifest::recipe(&c1, SEED).with_dataset(dataset_id);
        let manifest2 = Manifest::recipe(&c2, SEED).with_dataset(corpus2.id());
        let manifest_reproducible = manifest1.id() == manifest2.id();
        assert!(
            manifest_reproducible,
            "ManifestId is reproducible by reference across independent compiles"
        );

        let point_ok = compile_deterministic
            && run_deterministic
            && retrieval_deterministic
            && sn8_holds
            && manifest_reproducible;
        all_ok &= point_ok;

        println!(
            "H5 steps={steps} vecs={vecs}: compile_ms={compile_ms} run_ms={run_ms} \
             index_build_ms={index_build_ms} query_ms={query_ms} \
             dataset={} manifest={} integrated-ok={point_ok}",
            &dataset_id.to_hex()[..12],
            &manifest1.id().to_hex()[..12],
        );
    }
    println!("H5: integrated compile→run→dataset→retrieval→manifest all-ok={all_ok}");
    assert!(
        all_ok,
        "every scaling point holds all P4.1 invariants under load"
    );
}
