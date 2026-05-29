//! End-to-end: a Morphic-compiled PURE workflow drives through the real
//! single-node runtime primitives (`Scheduler::submit` → `run_pure_mote`) to
//! `Committed`, and two independent runs produce a byte-identical journal
//! digest. This closes the loop: workflow source → Mote DAG → run → reproducible
//! committed state.
//!
//! Integration tests compile as their own crate; this file carries its own lint
//! exemptions. `kx-runtime`/`kx-scheduler`/etc. are DEV-only deps — kx-workflow
//! keeps no production dependency on them (the thesis test).
#![allow(clippy::unwrap_used)]

use kx_executor::{run_pure_mote, LocalResourceManager, TestMoteExecutor};
use kx_journal::SqliteJournal;
use kx_mote::{EdgeMeta, LogicRef, ModelId, ToolName};
use kx_projection::Projection;
use kx_runtime::digest_journal;
use kx_scheduler::{LocalPlacement, Scheduler};
use kx_workflow::{compile, permissive_warrant, transform, CompiledWorkflow, WorkflowDef};

/// A 3-step all-PURE chain: A ──data──> B ──data──> C. PURE so no broker /
/// capability wiring is needed and the committed journal is fully deterministic.
fn pure_chain() -> WorkflowDef {
    let model = ModelId("local".into());
    let cap = ToolName("demo".into());
    let warrant = permissive_warrant(model.clone());
    let mut wf = WorkflowDef::new(42);
    let a = wf.add_step(transform(
        LogicRef::from_bytes([1; 32]),
        model.clone(),
        warrant.clone(),
        cap.clone(),
    ));
    let b = wf.add_step(transform(
        LogicRef::from_bytes([2; 32]),
        model.clone(),
        warrant.clone(),
        cap.clone(),
    ));
    let c = wf.add_step(transform(
        LogicRef::from_bytes([3; 32]),
        model,
        warrant,
        cap,
    ));
    wf.add_edge(a, b, EdgeMeta::data()).unwrap();
    wf.add_edge(b, c, EdgeMeta::data()).unwrap();
    wf
}

/// Submit every compiled mote to a fresh scheduler+projection (exercising the
/// real submission surface), run each PURE mote to commit, then digest the
/// resulting journal. Returns `(digest_hex, committed_count)`.
fn run_pure_workflow(compiled: &CompiledWorkflow) -> (String, usize) {
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
    // Compiled motes are in topological order, so running them in sequence
    // respects dependencies.
    for cm in &compiled.motes {
        run_pure_mote(&cm.mote, &cm.warrant, &journal, &rm, &executor).unwrap();
    }

    let digest = digest_journal(&journal).unwrap();
    let committed = Projection::from_journal(&journal)
        .unwrap()
        .committed_count();
    (digest.to_hex(), committed)
}

#[test]
fn compiled_pure_workflow_runs_and_is_reproducible() {
    let wf = pure_chain();
    let c1 = compile(&wf).unwrap();
    let c2 = compile(&wf).unwrap();

    let (digest_a, committed_a) = run_pure_workflow(&c1);
    let (digest_b, committed_b) = run_pure_workflow(&c2);

    assert_eq!(committed_a, 3, "all three PURE motes must commit");
    assert_eq!(committed_b, 3);
    assert_eq!(
        digest_a, digest_b,
        "two independent runs of the compiled workflow must yield a byte-identical committed digest"
    );
}

#[test]
fn compiled_motes_submit_in_dependency_order() {
    // The scheduler accepts every compiled mote (parents already submitted
    // earlier in topological order), confirming the output plugs into the
    // submission surface without reordering.
    let wf = pure_chain();
    let compiled = compile(&wf).unwrap();
    let mut projection = Projection::new();
    let mut scheduler = Scheduler::new(LocalPlacement);
    for cm in &compiled.motes {
        assert!(scheduler
            .submit(cm.mote.clone(), cm.warrant.clone(), &mut projection)
            .is_ok());
    }
}
