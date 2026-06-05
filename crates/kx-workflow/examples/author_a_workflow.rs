//! Author a workflow, compile it to a Mote DAG, run it through the real
//! single-node runtime primitives, and inspect the committed result.
//!
//! This is the "how do I actually use it?" example for contributors. It mirrors
//! the path real code takes — author → compile → submit → run → fold — using a
//! small all-PURE chain so it needs no model, no network, and no capability
//! wiring, and is fully deterministic.
//!
//! Run it:
//!
//! ```text
//! cargo run -p kx-workflow --example author_a_workflow
//! ```
//!
//! See the README (How it works) for how these pieces fit and GLOSSARY.md for the terms.

use std::fmt::Write as _;

use kx_executor::{run_pure_mote, LocalResourceManager, TestMoteExecutor};
use kx_journal::SqliteJournal;
use kx_mote::{EdgeMeta, LogicRef, ModelId, ToolName};
use kx_projection::Projection;
use kx_runtime::digest_journal;
use kx_scheduler::{LocalPlacement, Scheduler};
use kx_workflow::{compile, permissive_warrant, transform, WorkflowDef};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // ── 1. Author a workflow ────────────────────────────────────────────────
    // A `WorkflowDef` is the author-side shape: steps + the edges between them.
    // The seed (42) makes the authored shape reproducible. Here we build a
    // 3-step chain  A ──data──▶ B ──data──▶ C : each step's output feeds the
    // next. `transform(..)` is a convenience that builds a PURE step (no world
    // effect); `permissive_warrant` grants it a broad local scope for the demo.
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
    // A Data edge means "B consumes A's output" — and it makes B wait for A to
    // commit. (Repudiating A would cascade to B and C along these edges.)
    wf.add_edge(a, b, EdgeMeta::data())?;
    wf.add_edge(b, c, EdgeMeta::data())?;
    println!("1. authored a 3-step PURE chain (A → B → C)");

    // ── 2. Compile WorkflowDef → Mote DAG ───────────────────────────────────
    // Compilation is pure + deterministic: the same WorkflowDef always produces
    // the same Motes with the same content-addressed `MoteId`s (a precondition
    // for replay). Motes come back in topological order.
    let compiled = compile(&wf)?;
    println!(
        "2. compiled to {} Motes (topologically ordered):",
        compiled.motes.len()
    );
    for (i, cm) in compiled.motes.iter().enumerate() {
        // A MoteId is a 32-byte content address; show the first 4 bytes as hex.
        let mut short = String::new();
        for byte in &cm.mote.id.as_bytes()[..4] {
            write!(short, "{byte:02x}")?;
        }
        println!("     mote[{i}] id={short}…  nd={:?}", cm.mote.def.nd_class);
    }

    // ── 3. Run it through the real single-node primitives ────────────────────
    // Submit every Mote to a fresh scheduler + projection (the same submission
    // surface production uses), then run each PURE Mote to a durable `Committed`
    // fact. The journal (in-memory here) is the single source of truth.
    let journal = SqliteJournal::open_in_memory()?;
    let rm = LocalResourceManager::dev_defaults();
    let executor = TestMoteExecutor::deterministic();

    let mut projection = Projection::new();
    let mut scheduler = Scheduler::new(LocalPlacement);
    for cm in &compiled.motes {
        scheduler.submit(cm.mote.clone(), cm.warrant.clone(), &mut projection)?;
    }
    for cm in &compiled.motes {
        run_pure_mote(&cm.mote, &cm.warrant, &journal, &rm, &executor)?;
    }
    println!("3. submitted + ran all Motes to Committed");

    // ── 4. Inspect the result via a fresh fold of the journal ───────────────
    // Live state is never stored authoritatively — it is re-derived by folding
    // the journal. `digest_journal` folds, then hashes the committed facts;
    // re-running this whole program yields the SAME digest (try it).
    let committed = Projection::from_journal(&journal)?.committed_count();
    let digest = digest_journal(&journal)?;
    println!(
        "4. journal fold: {committed} committed; digest = {}",
        digest.to_hex()
    );
    println!("\n   (re-run: same digest — the run is reproducible. Crash mid-run and");
    println!("    replay instead — see the README 'Try it' demo for exactly-once.)");

    Ok(())
}
