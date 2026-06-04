//! `BodyLedger` (M8/D121): content-verified, immutable, idempotent recipe-body
//! storage keyed by the recipe's `ManifestId`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use kx_catalog::{body_manifest_id, BodyLedger, BodyOutcome, InMemoryBodyLedger};
use kx_mote::{EdgeMeta, LogicRef, ModelId, ToolName};
use kx_workflow::{compile, permissive_warrant, transform, Manifest, WorkflowDef};

fn body(seed: u32, logic: u8) -> WorkflowDef {
    let mut wf = WorkflowDef::new(seed);
    wf.add_step(transform(
        LogicRef::from_bytes([logic; 32]),
        ModelId("m".into()),
        permissive_warrant(ModelId("m".into())),
        ToolName("demo".into()),
    ));
    wf
}

#[test]
fn publish_keys_by_the_recipe_it_compiles_to() {
    let ledger = InMemoryBodyLedger::new();
    let wf = body(1, 0xAA);
    let expected = Manifest::recipe(&compile(&wf).unwrap(), wf.seed()).id();

    let (id, outcome) = ledger.publish_body(wf.clone()).unwrap();
    assert_eq!(id, expected, "keyed by the recipe identity it compiles to");
    assert_eq!(id, body_manifest_id(&wf).unwrap());
    assert!(matches!(outcome, BodyOutcome::Inserted(_)));
    assert_eq!(ledger.get_body(&id), Some(wf));
}

#[test]
fn republishing_is_idempotent() {
    let ledger = InMemoryBodyLedger::new();
    let wf = body(2, 0xBB);
    let (id1, o1) = ledger.publish_body(wf.clone()).unwrap();
    let (id2, o2) = ledger.publish_body(wf).unwrap();
    assert_eq!(id1, id2);
    assert!(matches!(o1, BodyOutcome::Inserted(_)));
    assert!(matches!(o2, BodyOutcome::AlreadyPresent(_)));
    assert_eq!(ledger.len(), 1);
}

#[test]
fn distinct_recipes_store_separately() {
    let ledger = InMemoryBodyLedger::new();
    let (a, _) = ledger.publish_body(body(3, 0x01)).unwrap();
    let (b, _) = ledger.publish_body(body(4, 0x02)).unwrap();
    assert_ne!(a, b);
    assert_eq!(ledger.len(), 2);
    assert!(ledger.get_body(&a).is_some());
    assert!(ledger.get_body(&b).is_some());
}

#[test]
fn an_uncompilable_body_is_refused() {
    let ledger = InMemoryBodyLedger::new();
    // A 2-step cycle A->B->A does not topologically order, so it cannot compile,
    // so it has no recipe identity to key on — refused at publish.
    let mut wf = WorkflowDef::new(5);
    let a = wf.add_step(transform(
        LogicRef::from_bytes([1; 32]),
        ModelId("m".into()),
        permissive_warrant(ModelId("m".into())),
        ToolName("demo".into()),
    ));
    let b = wf.add_step(transform(
        LogicRef::from_bytes([2; 32]),
        ModelId("m".into()),
        permissive_warrant(ModelId("m".into())),
        ToolName("demo".into()),
    ));
    wf.add_edge(a, b, EdgeMeta::data()).unwrap();
    wf.add_edge(b, a, EdgeMeta::data()).unwrap();
    assert!(ledger.publish_body(wf).is_err());
    assert!(ledger.is_empty());
}
