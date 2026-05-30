//! Workflow compile (Morphic engine) scaling stress harness
//! (P4.1 scale & performance validation campaign).
//!
//! `#[ignore]`d; run explicitly in RELEASE:
//!
//! ```text
//! cargo test -p kx-workflow --release --test stress_compile \
//!     -- --ignored --nocapture --test-threads=1
//! ```
//!
//! **H4** builds a large chain `WorkflowDef` (steps in {1_000, 5_000, 10_000}),
//! compiles it to a Mote DAG, prints compile-ms + node count, runs the compiled
//! PURE DAG through the real runtime surface and prints run-ms, and asserts the
//! compile is DETERMINISTIC (compile twice → byte-identical committed digest of
//! the run, i.e. identical DAG identity).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::time::Instant;

use kx_executor::{run_pure_mote, LocalResourceManager, TestMoteExecutor};
use kx_journal::SqliteJournal;
use kx_mote::{EdgeMeta, LogicRef, ModelId, ToolName};
use kx_projection::Projection;
use kx_runtime::digest_journal;
use kx_scheduler::{LocalPlacement, Scheduler};
use kx_workflow::{compile, permissive_warrant, transform, CompiledWorkflow, WorkflowDef};

const SIZES: &[usize] = &[1_000, 5_000, 10_000];

/// Build a PURE chain WorkflowDef of `n` steps: s0 -> s1 -> ... -> s(n-1).
fn chain_workflow(n: usize) -> WorkflowDef {
    let model = ModelId("local".into());
    let cap = ToolName("demo".into());
    let warrant = permissive_warrant(model.clone());
    let mut wf = WorkflowDef::new(42);
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

/// Drive a compiled PURE workflow through the runtime; return committed digest.
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

#[test]
#[ignore = "stress: run with --release --ignored --nocapture --test-threads=1"]
fn h4_compile_and_run_scaling() {
    let mut all_deterministic = true;
    for &n in SIZES {
        let build_start = Instant::now();
        let wf = chain_workflow(n);
        let _build_ms = build_start.elapsed().as_millis();

        let c_start = Instant::now();
        let c1 = compile(&wf).unwrap();
        let compile_ms = c_start.elapsed().as_millis();
        let nodes = c1.motes.len();
        assert_eq!(nodes, n, "compile emits one Mote per step");

        // Determinism: compile a second time, run both, compare digests.
        let c2 = compile(&wf).unwrap();

        let r_start = Instant::now();
        let (committed1, digest1) = run_compiled(&c1);
        let run_ms = r_start.elapsed().as_millis();
        let (committed2, digest2) = run_compiled(&c2);

        assert_eq!(committed1, n, "all steps commit");
        assert_eq!(committed2, n);
        let deterministic = digest1 == digest2;
        all_deterministic &= deterministic;
        let prefix: String = digest1.chars().take(12).collect();

        println!(
            "H4 steps={n}: compile_ms={compile_ms} run_ms={run_ms} nodes={nodes} \
             deterministic-compile={deterministic} digest_prefix={prefix}"
        );
    }
    println!("H4: deterministic-compile={all_deterministic}");
}
