//! `WorkflowDef` serde is the M8/G1 durable-body unblock — and it must be
//! STRICTLY off the identity path: adding the derives changes no field and no
//! ordering, so `compile()` / `ManifestId` / `MoteId` are byte-invariant (the
//! product digest `a6b5c679…` stays put). This guard round-trips a non-trivial
//! workflow and proves both the round-trip AND the identity invariance.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use kx_mote::{ConfigKey, ConfigVal, EdgeMeta, LogicRef, ModelId, ToolName};
use kx_workflow::{compile, permissive_warrant, transform, Manifest, WorkflowDef};

fn canonical() -> impl bincode::config::Config {
    bincode::config::standard()
        .with_little_endian()
        .with_fixed_int_encoding()
}

/// A non-trivial workflow: a 2-step pure chain A → B where A carries a bound
/// `config_subset` entry (exercises StepDef.config_subset / StepEdge / the seed).
fn sample() -> WorkflowDef {
    let mut wf = WorkflowDef::new(0xC0FFEE);
    let mut a = transform(
        LogicRef::from_bytes([1; 32]),
        ModelId("m".into()),
        permissive_warrant(ModelId("m".into())),
        ToolName("demo".into()),
    );
    a.config_subset
        .insert(ConfigKey("topic".into()), ConfigVal(b"incidents".to_vec()));
    let a_ref = wf.add_step(a);
    let b_ref = wf.add_step(transform(
        LogicRef::from_bytes([2; 32]),
        ModelId("m".into()),
        permissive_warrant(ModelId("m".into())),
        ToolName("demo".into()),
    ));
    wf.add_edge(a_ref, b_ref, EdgeMeta::data()).unwrap();
    wf
}

#[test]
fn workflow_def_round_trips_byte_for_byte() {
    let original = sample();
    let bytes = bincode::serde::encode_to_vec(&original, canonical()).unwrap();
    let (decoded, consumed): (WorkflowDef, usize) =
        bincode::serde::decode_from_slice(&bytes, canonical()).unwrap();
    assert_eq!(consumed, bytes.len(), "no trailing garbage");
    assert_eq!(decoded, original, "serde round-trip is identity");
}

#[test]
fn serde_does_not_move_the_recipe_identity() {
    let original = sample();
    let bytes = bincode::serde::encode_to_vec(&original, canonical()).unwrap();
    let (decoded, _): (WorkflowDef, usize) =
        bincode::serde::decode_from_slice(&bytes, canonical()).unwrap();

    let co = compile(&original).unwrap();
    let cd = compile(&decoded).unwrap();

    // The compiled Mote identities are unchanged — serde is off the identity path.
    let ids_o: Vec<_> = co.motes.iter().map(|m| m.mote.id).collect();
    let ids_d: Vec<_> = cd.motes.iter().map(|m| m.mote.id).collect();
    assert_eq!(ids_o, ids_d, "compile() MoteIds are serde-invariant");

    // And the recipe identity (ManifestId) is unchanged.
    assert_eq!(
        Manifest::recipe(&co, original.seed()).id(),
        Manifest::recipe(&cd, decoded.seed()).id(),
        "ManifestId is serde-invariant (digest a6b5c679 stays put)"
    );
}
