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

/// ★ The reason `step_index` exists. Motes are emitted in TOPOLOGICAL order, which is not
/// authoring order — so a caller holding a per-step decision needs the authored index back,
/// and must not have to recompute the ordering rule that already lives in `compile`.
/// Built deliberately "backwards": step 0 depends on step 1, so topological order reverses
/// the authored order and an off-by-index bug cannot pass.
#[test]
fn compiled_motes_carry_their_authored_step_index() {
    let mut wf = WorkflowDef::new(0);
    let late = wf.add_step(transform(logic(1), model(), warrant(), cap()));
    let early = wf.add_step(transform(logic(2), model(), warrant(), cap()));
    wf.add_edge(early, late, EdgeMeta::data()).unwrap();

    let out = compile(&wf).unwrap();
    // Topological order puts the AUTHORED-SECOND step first...
    assert_eq!(
        out.motes
            .iter()
            .map(|m| m.step_index)
            .collect::<Vec<usize>>(),
        vec![early.index(), late.index()],
        "emission order is topological, not authoring order"
    );
    // ...and every index still addresses the step whose logic it was authored with.
    for m in &out.motes {
        assert_eq!(m.mote.def.logic_ref, wf.steps[m.step_index].logic_ref);
    }
}

/// `inject_step_config` targets ONE step. Its sibling `inject_entry_config` targets every
/// DAG ROOT, which on a fan-out is several steps — the difference that matters when a
/// capability belongs to one node rather than to the run.
#[test]
fn inject_step_config_targets_one_step_and_reports_a_bad_index() {
    use kx_mote::ConfigVal;

    let mut wf = WorkflowDef::new(0);
    let a = wf.add_step(transform(logic(1), model(), warrant(), cap()));
    let b = wf.add_step(transform(logic(2), model(), warrant(), cap()));
    let sink = wf.add_step(transform(logic(3), model(), warrant(), cap()));
    wf.add_edge(a, sink, EdgeMeta::data()).unwrap();
    wf.add_edge(b, sink, EdgeMeta::data()).unwrap();

    let val = ConfigVal(b"bound".to_vec());
    assert!(wf.inject_step_config(b.index(), "kx.test.bound", &val));
    let has = |i: usize| {
        wf.steps[i]
            .config_subset
            .contains_key(&kx_mote::ConfigKey("kx.test.bound".to_string()))
    };
    assert!(has(b.index()), "the named step is configured");
    assert!(
        !has(a.index()) && !has(sink.index()),
        "no sibling root and no child is touched — the whole point"
    );
    // Out of range REPORTS rather than silently no-oping (or hitting the wrong step).
    assert!(!wf.inject_step_config(99, "kx.test.bound", &val));
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
