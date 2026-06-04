//! `WorkflowDef::bind_param` (the M8/D121 binding primitive): injecting a value
//! into a declared config slot changes the compiled Mote identity (so distinct
//! bound values yield distinct runs), and binding an undeclared slot reports 0
//! (so the caller can fail-closed).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use kx_mote::{ConfigKey, ConfigVal, LogicRef, ModelId, ToolName};
use kx_workflow::{compile, permissive_warrant, transform, WorkflowDef};

fn body_with_topic() -> WorkflowDef {
    let mut wf = WorkflowDef::new(9);
    let mut step = transform(
        LogicRef::from_bytes([1; 32]),
        ModelId("m".into()),
        permissive_warrant(ModelId("m".into())),
        ToolName("demo".into()),
    );
    step.config_subset
        .insert(ConfigKey("topic".into()), ConfigVal(Vec::new()));
    wf.add_step(step);
    wf
}

#[test]
fn binding_a_declared_slot_changes_compiled_identity() {
    let mut a = body_with_topic();
    let mut b = body_with_topic();
    assert_eq!(a.bind_param("topic", &ConfigVal(b"incidents".to_vec())), 1);
    assert_eq!(b.bind_param("topic", &ConfigVal(b"outages".to_vec())), 1);

    let id_a = compile(&a).unwrap().motes[0].mote.id;
    let id_b = compile(&b).unwrap().motes[0].mote.id;
    assert_ne!(id_a, id_b, "distinct bound values → distinct Mote identity");
}

#[test]
fn identical_bound_values_yield_identical_identity() {
    let mut a = body_with_topic();
    let mut b = body_with_topic();
    a.bind_param("topic", &ConfigVal(b"same".to_vec()));
    b.bind_param("topic", &ConfigVal(b"same".to_vec()));
    assert_eq!(
        compile(&a).unwrap().motes[0].mote.id,
        compile(&b).unwrap().motes[0].mote.id
    );
}

#[test]
fn binding_an_undeclared_slot_reports_zero() {
    let mut wf = body_with_topic();
    assert_eq!(
        wf.bind_param("nope", &ConfigVal(b"x".to_vec())),
        0,
        "a slot no step declares binds nothing → caller fails closed"
    );
}
