//! The `deterministic_critic` authoring builder (D60 / P4.2-2) compiles to a
//! well-formed native-critic `MoteDef`: PURE, `critic_for` = the producer's
//! derived `MoteId`, and `critic_check` = the declared `CheckSpec` folded into
//! identity. Reproducible by construction (same recipe → same `MoteId`s).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use kx_critic_types::{CheckSpec, SchemaSpec, SchemaTag};
use kx_mote::{EdgeMeta, LogicRef, ModelId, NdClass, ToolName};
use kx_workflow::{compile, deterministic_critic, permissive_warrant, transform, WorkflowDef};

fn json_check() -> CheckSpec {
    CheckSpec::Schema(SchemaSpec {
        expected: SchemaTag::Json,
    })
}

fn build() -> WorkflowDef {
    let model = ModelId("local".into());
    let cap = ToolName("demo".into());
    let warrant = permissive_warrant(model.clone());
    let mut wf = WorkflowDef::new(7);
    let producer = wf.add_step(transform(
        LogicRef::from_bytes([1; 32]),
        model.clone(),
        warrant.clone(),
        cap.clone(),
    ));
    let critic = wf.add_step(deterministic_critic(
        producer,
        json_check(),
        LogicRef::from_bytes([2; 32]),
        model,
        warrant,
        cap,
    ));
    wf.add_edge(producer, critic, EdgeMeta::data()).unwrap();
    wf
}

#[test]
fn deterministic_critic_compiles_to_native_critic_mote_def() {
    let compiled = compile(&build()).unwrap();
    assert_eq!(compiled.motes.len(), 2);

    // The producer is the mote with no critic_check; the critic carries it.
    let producer = compiled
        .motes
        .iter()
        .find(|m| m.mote.def.critic_check.is_none())
        .expect("producer present");
    let critic = compiled
        .motes
        .iter()
        .find(|m| m.mote.def.critic_check.is_some())
        .expect("critic present");

    // R-15 shape, by construction.
    assert_eq!(critic.mote.def.nd_class, NdClass::Pure);
    assert!(!critic.mote.def.is_topology_shaper);
    assert_eq!(critic.mote.def.critic_for, Some(producer.mote.id));
    assert_eq!(critic.mote.def.critic_check, Some(json_check()));
}

#[test]
fn deterministic_critic_recipe_is_reproducible() {
    let a = compile(&build()).unwrap();
    let b = compile(&build()).unwrap();
    let ids_a: Vec<_> = a.motes.iter().map(|m| m.mote.id).collect();
    let ids_b: Vec<_> = b.motes.iter().map(|m| m.mote.id).collect();
    assert_eq!(
        ids_a, ids_b,
        "same recipe => byte-identical MoteIds (the check folds into identity)"
    );
}

#[test]
fn changing_the_check_changes_the_critic_identity() {
    // The declared check is part of the critic's MoteId — a different check is a
    // different Mote (reproducible-by-construction; SN-8 identity discrimination).
    let model = ModelId("local".into());
    let cap = ToolName("demo".into());
    let warrant = permissive_warrant(model.clone());

    let mk = |check: CheckSpec| {
        let mut wf = WorkflowDef::new(7);
        let p = wf.add_step(transform(
            LogicRef::from_bytes([1; 32]),
            model.clone(),
            warrant.clone(),
            cap.clone(),
        ));
        let c = wf.add_step(deterministic_critic(
            p,
            check,
            LogicRef::from_bytes([2; 32]),
            model.clone(),
            warrant.clone(),
            cap.clone(),
        ));
        wf.add_edge(p, c, EdgeMeta::data()).unwrap();
        let compiled = compile(&wf).unwrap();
        compiled
            .motes
            .iter()
            .find(|m| m.mote.def.critic_check.is_some())
            .unwrap()
            .mote
            .id
    };

    let json = mk(json_check());
    let text = mk(CheckSpec::Schema(SchemaSpec {
        expected: SchemaTag::Text,
    }));
    assert_ne!(json, text, "different check => different critic MoteId");
}
