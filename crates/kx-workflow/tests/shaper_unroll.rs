//! A Morphic-compiled topology shaper unrolls into deterministic children:
//! given the shaper's committed `TopologyDecision`, the production
//! `kx_runtime::topology::derive_child_motes` re-derives byte-identical child
//! `MoteId`s across re-derivation (the replay guarantee), and the children fold
//! in the shaper's committed `result_ref` (so a different decision → different
//! children). This is how loops / branches / fan-out express as a runtime DAG.
#![allow(clippy::unwrap_used)]

use kx_content::ContentRef;
use kx_mote::{LogicRef, ModelId, ToolName};
use kx_runtime::topology::{demo_topology_decision, derive_child_motes, encode_topology_decision};
use kx_workflow::{compile, permissive_warrant, topology_shaper, WorkflowDef};

#[test]
fn compiled_shaper_unrolls_deterministically() {
    let model = ModelId("local".into());
    let cap = ToolName("demo".into());
    let warrant = permissive_warrant(model.clone());

    // Compile a one-step workflow whose only step is a topology shaper.
    let mut wf = WorkflowDef::new(0);
    wf.add_step(topology_shaper(
        LogicRef::from_bytes([9; 32]),
        model,
        warrant.clone(),
        cap.clone(),
    ));
    let compiled = compile(&wf).unwrap();
    let shaper = &compiled.motes[0].mote;
    assert!(shaper.def.is_topology_shaper);

    // The shaper's committed result IS a TopologyDecision; its content-ref is
    // what child identity folds in.
    let decision = demo_topology_decision();
    let result_ref = ContentRef::of(&encode_topology_decision(&decision).unwrap());

    let children_a = derive_child_motes(shaper, result_ref, &decision, &warrant, &cap);
    let children_b = derive_child_motes(shaper, result_ref, &decision, &warrant, &cap);
    let ids_a: Vec<_> = children_a.iter().map(|w| w.mote.id).collect();
    let ids_b: Vec<_> = children_b.iter().map(|w| w.mote.id).collect();

    assert_eq!(ids_a.len(), decision.children.len());
    assert!(ids_a.len() >= 2, "the demo decision fans out to ≥2 workers");
    assert_eq!(
        ids_a, ids_b,
        "re-derivation (replay) must produce byte-identical child MoteIds"
    );

    // Children are PURE workers parented on the shaper (control edge).
    for w in &children_a {
        assert_eq!(w.mote.parents.len(), 1);
        assert_eq!(w.mote.parents[0].parent_id, shaper.id);
    }

    // Sensitivity: a different committed decision result_ref → different child
    // identities (child MoteId folds in the shaper's committed result).
    let other_ref = ContentRef::from_bytes([0xAB; 32]);
    let children_c = derive_child_motes(shaper, other_ref, &decision, &warrant, &cap);
    let ids_c: Vec<_> = children_c.iter().map(|w| w.mote.id).collect();
    assert_ne!(
        ids_a, ids_c,
        "child identity must fold the shaper's committed result_ref"
    );
}
