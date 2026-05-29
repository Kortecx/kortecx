//! Unit tests for the Morphic compile pass: determinism, seed sensitivity,
//! topological soundness, cycle/critic validation, and role flags.

use kx_mote::{EdgeMeta, LogicRef, ModelId, NdClass, ToolName};

use crate::def::StepRef;
use crate::{
    compile, critic, permissive_warrant, synthesis_pipeline, topology_shaper, transform,
    CompileError, WorkflowDef,
};

fn model() -> ModelId {
    ModelId("local".into())
}
fn cap() -> ToolName {
    ToolName("demo".into())
}
fn warrant() -> kx_warrant::WarrantSpec {
    permissive_warrant(model())
}
fn logic(seed: u8) -> LogicRef {
    LogicRef::from_bytes([seed; 32])
}

#[test]
fn empty_workflow_compiles_to_empty() {
    let wf = WorkflowDef::new(0);
    let out = compile(&wf).unwrap();
    assert!(out.motes.is_empty());
    assert!(out.graph.is_empty());
}

#[test]
fn compile_is_deterministic() {
    let wf = synthesis_pipeline(7, model(), cap(), logic(1), logic(2), logic(3)).unwrap();
    let a = compile(&wf).unwrap();
    let b = compile(&wf).unwrap();
    let ids_a: Vec<_> = a.motes.iter().map(|m| m.mote.id).collect();
    let ids_b: Vec<_> = b.motes.iter().map(|m| m.mote.id).collect();
    assert_eq!(ids_a, ids_b);
    assert_eq!(a.motes.len(), 3);
}

#[test]
fn seed_changes_identity() {
    let wf1 = synthesis_pipeline(1, model(), cap(), logic(1), logic(2), logic(3)).unwrap();
    let wf2 = synthesis_pipeline(2, model(), cap(), logic(1), logic(2), logic(3)).unwrap();
    let a = compile(&wf1).unwrap();
    let b = compile(&wf2).unwrap();
    // The entrypoint generator's identity folds the workflow seed, and the
    // change propagates down the lineage to every descendant.
    for (x, y) in a.motes.iter().zip(b.motes.iter()) {
        assert_ne!(
            x.mote.id, y.mote.id,
            "every MoteId must shift with the seed"
        );
    }
}

#[test]
fn topo_order_places_parents_before_children() {
    let wf = synthesis_pipeline(0, model(), cap(), logic(1), logic(2), logic(3)).unwrap();
    let out = compile(&wf).unwrap();
    let pos: std::collections::BTreeMap<_, _> = out
        .motes
        .iter()
        .enumerate()
        .map(|(i, m)| (m.mote.id, i))
        .collect();
    for (i, m) in out.motes.iter().enumerate() {
        for parent in &m.mote.parents {
            let pi = pos[&parent.parent_id];
            assert!(pi < i, "parent must be submitted before its child");
        }
    }
}

#[test]
fn parents_all_exist_in_graph() {
    let wf = synthesis_pipeline(0, model(), cap(), logic(1), logic(2), logic(3)).unwrap();
    let out = compile(&wf).unwrap();
    for m in &out.motes {
        for parent in &m.mote.parents {
            assert!(
                out.graph.get(&parent.parent_id).is_some(),
                "every declared parent must be a node in the graph"
            );
        }
    }
}

#[test]
fn critic_resolves_to_producer_mote_id() {
    let wf = synthesis_pipeline(0, model(), cap(), logic(1), logic(2), logic(3)).unwrap();
    let out = compile(&wf).unwrap();
    // The transform carries logic_ref [2; 32]; the critic validates it.
    let transform_id = out
        .motes
        .iter()
        .find(|m| m.mote.def.logic_ref == logic(2))
        .map(|m| m.mote.id)
        .unwrap();
    let critic = out
        .motes
        .iter()
        .find(|m| m.mote.def.critic_for.is_some())
        .unwrap();
    assert_eq!(critic.mote.def.critic_for, Some(transform_id));
}

#[test]
fn topology_shaper_is_flagged_and_read_only_nondet() {
    let mut wf = WorkflowDef::new(0);
    wf.add_step(topology_shaper(logic(9), model(), warrant(), cap()));
    let out = compile(&wf).unwrap();
    let shaper = &out.motes[0].mote;
    assert!(shaper.def.is_topology_shaper);
    assert_eq!(shaper.nd_class(), NdClass::ReadOnlyNondet);
}

#[test]
fn cycle_is_rejected() {
    let mut wf = WorkflowDef::new(0);
    let a = wf.add_step(transform(logic(1), model(), warrant(), cap()));
    let b = wf.add_step(transform(logic(2), model(), warrant(), cap()));
    wf.add_edge(a, b, EdgeMeta::data()).unwrap();
    wf.add_edge(b, a, EdgeMeta::data()).unwrap();
    assert!(matches!(compile(&wf), Err(CompileError::Cycle(_))));
}

#[test]
fn duplicate_edge_is_rejected_at_declaration() {
    let mut wf = WorkflowDef::new(0);
    let a = wf.add_step(transform(logic(1), model(), warrant(), cap()));
    let b = wf.add_step(transform(logic(2), model(), warrant(), cap()));
    wf.add_edge(a, b, EdgeMeta::data()).unwrap();
    assert_eq!(
        wf.add_edge(a, b, EdgeMeta::data()),
        Err(CompileError::DuplicateEdge {
            parent: 0,
            child: 1
        })
    );
}

#[test]
fn out_of_range_edge_is_rejected() {
    let mut wf = WorkflowDef::new(0);
    let a = wf.add_step(transform(logic(1), model(), warrant(), cap()));
    // StepRef(5) does not exist.
    assert_eq!(
        wf.add_edge(a, StepRef(5), EdgeMeta::data()),
        Err(CompileError::StepIndexOutOfRange(5))
    );
}

#[test]
fn critic_whose_producer_does_not_precede_is_rejected() {
    // Critic at index 0 references a producer at index 1 with NO ordering edge.
    // The min-index frontier processes the critic first, before the producer's
    // MoteId is known → InvalidCritic.
    let mut wf = WorkflowDef::new(0);
    let producer_ref = StepRef(1);
    wf.add_step(critic(producer_ref, logic(3), model(), warrant(), cap()));
    wf.add_step(transform(logic(2), model(), warrant(), cap()));
    assert_eq!(
        compile(&wf),
        Err(CompileError::InvalidCritic {
            critic: 0,
            producer: 1
        })
    );
}
