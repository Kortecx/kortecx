//! Author a workflow from the **recipe library**, run it to a reproducible
//! committed digest, then bind a **prompt template** and watch the rendered
//! prompt flow into Mote identity.
//!
//! This is the "how do I use the recipes + prompt-templating?" example. Part 1
//! uses an all-PURE `map_reduce` so it needs no model / network / capability
//! wiring and is fully deterministic; Part 2 shows that a rendered prompt is
//! identity-bearing (same prompt ⇒ same `MoteId`, different prompt ⇒ different).
//!
//! Run it:
//!
//! ```text
//! cargo run -p kx-workflow --example recipe_to_digest
//! ```
//!
//! See ARCHITECTURE.md / GLOSSARY.md for how the pieces fit.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use kx_executor::{run_pure_mote, LocalResourceManager, TestMoteExecutor};
use kx_journal::SqliteJournal;
use kx_mote::{ConfigKey, ConfigVal, LogicRef, ModelId, ToolName, PROMPT_KEY};
use kx_projection::Projection;
use kx_runtime::digest_journal;
use kx_scheduler::{LocalPlacement, Scheduler};
use kx_workflow::{
    compile, map_reduce, permissive_warrant, render_prompts, transform, WorkerKind, WorkflowDef,
    TEMPLATE_KEY,
};

fn short(bytes: &[u8]) -> String {
    let mut s = String::new();
    for b in &bytes[..4] {
        let _ = write!(s, "{b:02x}");
    }
    s
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let model = ModelId("local".into());
    let cap = ToolName("demo".into());

    // ── Part 1. A recipe → a reproducible run ───────────────────────────────
    // `map_reduce` fans out to N PURE mappers and folds them in one reduce step.
    // Authored as data — pin the seed and the logic refs and it re-derives the
    // same Mote DAG every time.
    let mappers = [
        LogicRef::from_bytes([1; 32]),
        LogicRef::from_bytes([2; 32]),
        LogicRef::from_bytes([3; 32]),
    ];
    let wf = map_reduce(
        42,
        model.clone(),
        cap.clone(),
        WorkerKind::Transform,
        &mappers,
        LogicRef::from_bytes([9; 32]),
    )?;
    let compiled = compile(&wf)?;
    println!(
        "1. map_reduce recipe → {} Motes ({} mappers + 1 reduce)",
        compiled.motes.len(),
        mappers.len()
    );

    // Submit + run every Mote through the real single-node primitives, then fold
    // the journal back to a digest. Re-running yields the SAME digest.
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
    let committed = Projection::from_journal(&journal)?.committed_count();
    let digest = digest_journal(&journal)?;
    println!(
        "2. ran to {committed} committed; journal digest = {} (re-run → identical)",
        digest.to_hex()
    );

    // ── Part 2. A prompt template → identity-bearing prompt ─────────────────
    // A step carries an un-rendered template under TEMPLATE_KEY with named
    // {slots}. `render_prompts` substitutes params into the final prompt BEFORE
    // compile, so the rendered prompt folds into the Mote's identity.
    let build_templated = || -> WorkflowDef {
        let mut wf = WorkflowDef::new(7);
        let mut step = transform(
            LogicRef::from_bytes([5; 32]),
            model.clone(),
            permissive_warrant(model.clone()),
            cap.clone(),
        );
        step.config_subset.insert(
            ConfigKey(TEMPLATE_KEY.to_string()),
            ConfigVal(b"summarize {topic} for {audience}".to_vec()),
        );
        wf.add_step(step);
        wf
    };

    let mut a = build_templated();
    let mut b = build_templated();
    let params = |topic: &str, audience: &str| {
        BTreeMap::from([
            ("topic".to_string(), topic.to_string()),
            ("audience".to_string(), audience.to_string()),
        ])
    };
    render_prompts(&mut a, &params("outages", "SREs"))?;
    render_prompts(&mut b, &params("revenue", "execs"))?;

    let ca = compile(&a)?;
    let cb = compile(&b)?;
    let prompt_a = ca.motes[0]
        .mote
        .def
        .config_subset
        .get(&ConfigKey(PROMPT_KEY.to_string()))
        .map(|v| String::from_utf8_lossy(&v.0).into_owned())
        .unwrap_or_default();

    println!("3. rendered prompt   = {prompt_a:?}");
    println!(
        "4. mote id (outages) = {}…   mote id (revenue) = {}…",
        short(ca.motes[0].mote.id.as_bytes()),
        short(cb.motes[0].mote.id.as_bytes())
    );
    assert_ne!(
        ca.motes[0].mote.id, cb.motes[0].mote.id,
        "a different rendered prompt is a different Mote — fresh call, not recipe reuse"
    );
    println!("   → distinct prompts ⇒ distinct Mote identity (SN-8: derived, exact, never fuzzy)");

    Ok(())
}
