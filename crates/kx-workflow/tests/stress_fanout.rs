//! Recipe-library fan-out scaling stress harness (R6 scale & performance gate).
//!
//! `#[ignore]`d; run explicitly in RELEASE (wired into `just scale-smoke`):
//!
//! ```text
//! cargo test -p kx-workflow --release --test stress_fanout \
//!     -- --ignored --nocapture --test-threads=1
//! ```
//!
//! Builds a wide fan-out recipe (`map_reduce`: N PURE mappers → 1 reduce, N ∈
//! {1_000, 5_000, 10_000}), compiles it TWICE, runs both compiled DAGs through
//! the real single-node runtime surface, and asserts:
//!   - the recipe compiles to exactly `N + 1` Motes (the gather has `N` parents);
//!   - compile is DETERMINISTIC at fan-out scale (two runs → byte-identical
//!     committed journal digest);
//!   - prints compile-ms / run-ms / node count so the curve stays ~linear.
//!
//! The PURE `map_reduce` variant is used so the run is broker-free and fully
//! deterministic; the ROND `fan_out_gather` has the identical fan-in structure.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::time::Instant;

use kx_executor::{run_pure_mote, LocalResourceManager, TestMoteExecutor};
use kx_journal::SqliteJournal;
use kx_mote::{LogicRef, ModelId, ToolName};
use kx_projection::Projection;
use kx_runtime::digest_journal;
use kx_scheduler::{LocalPlacement, Scheduler};
use kx_workflow::{compile, map_reduce, CompiledWorkflow, WorkerKind, WorkflowDef};

const SIZES: &[usize] = &[1_000, 5_000, 10_000];

/// A wide fan-out recipe: `n` distinct PURE mappers → one reduce.
fn fanout_workflow(n: usize) -> WorkflowDef {
    let mappers: Vec<LogicRef> = (0..n)
        .map(|i| {
            let mut logic = [0u8; 32];
            logic[..8].copy_from_slice(&(i as u64).to_le_bytes());
            LogicRef::from_bytes(logic)
        })
        .collect();
    map_reduce(
        42,
        ModelId("local".into()),
        ToolName("demo".into()),
        WorkerKind::Transform,
        &mappers,
        LogicRef::from_bytes([0xFF; 32]),
    )
    .unwrap()
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
fn fanout_recipe_scales_linearly_and_deterministically() {
    let mut all_deterministic = true;
    for &n in SIZES {
        let wf = fanout_workflow(n);

        let c_start = Instant::now();
        let c1 = compile(&wf).unwrap();
        let compile_ms = c_start.elapsed().as_millis();
        let nodes = c1.motes.len();
        assert_eq!(nodes, n + 1, "N mappers + 1 reduce");

        // The reduce gathers every mapper.
        let reduce = c1
            .motes
            .iter()
            .find(|m| m.mote.def.logic_ref == LogicRef::from_bytes([0xFF; 32]))
            .unwrap();
        assert_eq!(
            reduce.mote.parents.len(),
            n,
            "gather has one parent per mapper"
        );

        let c2 = compile(&wf).unwrap();
        let r_start = Instant::now();
        let (committed1, digest1) = run_compiled(&c1);
        let run_ms = r_start.elapsed().as_millis();
        let (committed2, digest2) = run_compiled(&c2);

        assert_eq!(committed1, n + 1, "all motes commit");
        assert_eq!(committed2, n + 1);
        let deterministic = digest1 == digest2;
        all_deterministic &= deterministic;
        let prefix: String = digest1.chars().take(12).collect();

        println!(
            "fanout N={n}: compile_ms={compile_ms} run_ms={run_ms} nodes={nodes} \
             deterministic={deterministic} digest_prefix={prefix}"
        );
    }
    assert!(
        all_deterministic,
        "fan-out recipe compile must be deterministic at every scale"
    );
}
