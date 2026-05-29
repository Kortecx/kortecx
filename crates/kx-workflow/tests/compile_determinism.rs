//! Property test: random valid DAGs compile deterministically (identical
//! `MoteId`s across two compiles) and always to a well-formed DAG (every parent
//! precedes its child and exists in the graph).
//!
//! Integration tests compile as their own crate, so this file carries its own
//! lint exemptions (the workspace deny on unwrap/expect applies to library code).
#![allow(clippy::unwrap_used, clippy::cast_possible_truncation)]

use kx_mote::{EdgeMeta, LogicRef, ModelId, ToolName};
use kx_workflow::{compile, permissive_warrant, transform, StepRef, WorkflowDef};
use proptest::prelude::*;
use std::collections::BTreeMap;

fn build(seed: u32, n: usize, edges: &[(usize, usize)]) -> WorkflowDef {
    let model = ModelId("local".into());
    let cap = ToolName("demo".into());
    let warrant = permissive_warrant(model.clone());
    let mut wf = WorkflowDef::new(seed);
    let mut refs: Vec<StepRef> = Vec::with_capacity(n);
    for i in 0..n {
        refs.push(wf.add_step(transform(
            LogicRef::from_bytes([i as u8; 32]),
            model.clone(),
            warrant.clone(),
            cap.clone(),
        )));
    }
    for &(a, b) in edges {
        // Only parent<child edges → guaranteed acyclic. Duplicate edges are
        // rejected at declaration; ignore those so the property still holds.
        if a < n && b < n && a < b {
            let _ = wf.add_edge(refs[a], refs[b], EdgeMeta::data());
        }
    }
    wf
}

proptest! {
    #[test]
    fn random_dags_compile_deterministically(
        seed in any::<u32>(),
        n in 1usize..8,
        raw_edges in proptest::collection::vec((0usize..8, 0usize..8), 0..24),
    ) {
        let wf = build(seed, n, &raw_edges);

        let a = compile(&wf).unwrap();
        let b = compile(&wf).unwrap();

        // Determinism: byte-identical MoteId sequences.
        let ids_a: Vec<_> = a.motes.iter().map(|m| m.mote.id).collect();
        let ids_b: Vec<_> = b.motes.iter().map(|m| m.mote.id).collect();
        prop_assert_eq!(ids_a, ids_b);
        prop_assert_eq!(a.motes.len(), n);

        // Well-formed: parents precede children and exist in the graph.
        let pos: BTreeMap<_, _> = a
            .motes
            .iter()
            .enumerate()
            .map(|(i, m)| (m.mote.id, i))
            .collect();
        for (i, m) in a.motes.iter().enumerate() {
            for parent in &m.mote.parents {
                prop_assert!(a.graph.get(&parent.parent_id).is_some());
                prop_assert!(pos[&parent.parent_id] < i);
            }
        }
    }
}
