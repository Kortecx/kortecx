//! `BodyLedger` (M8/D121): content-verified, immutable, idempotent recipe-body
//! storage keyed by the recipe's `ManifestId`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use kx_catalog::{
    body_manifest_id, body_manifest_id_v1, BodyLedger, BodyOutcome, InMemoryBodyLedger,
    SqliteBodyLedger,
};
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

/// **The upgrade path of the ledger itself.**
///
/// Folding the step warrant into `ManifestId` moved every recipe id. A durable ledger
/// written by the PREVIOUS binary holds rows keyed under the old `…/v1` scheme, and
/// `SqliteBodyLedger::open` re-derives every stored key to prove the row was not
/// tampered with. Deriving only the current scheme rejects all of those rows and the
/// ledger refuses to open — i.e. the identity fix would have replaced one serve-boot
/// failure with another, on exactly the state dirs it exists to save.
///
/// No fixture in the tree has a pre-existing `bodies.db`, so the whole suite stayed
/// green while this was broken. This test writes the old-scheme row explicitly.
#[test]
fn a_ledger_written_under_the_previous_identity_scheme_still_opens() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bodies.db");
    let wf = body(7, 0xCC);

    let v1_key = body_manifest_id_v1(&wf).unwrap();
    let v2_key = body_manifest_id(&wf).unwrap();
    assert_ne!(
        v1_key, v2_key,
        "the two schemes must actually differ or this test proves nothing"
    );

    // Write the row exactly as the older binary would have: keyed by its v1 identity.
    {
        let ledger = SqliteBodyLedger::open(&path).unwrap();
        drop(ledger); // let `open` create the schema, then write the legacy row by hand
        let conn = rusqlite::Connection::open(&path).unwrap();
        let bytes = bincode::serde::encode_to_vec(
            &wf,
            bincode::config::standard()
                .with_little_endian()
                .with_fixed_int_encoding(),
        )
        .unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO bodies (manifest_id, body_bytes) VALUES (?1, ?2)",
            rusqlite::params![v1_key.0.as_slice(), &bytes[..]],
        )
        .unwrap();
    }

    // The upgraded binary opens it.
    let reopened = SqliteBodyLedger::open(&path)
        .expect("a ledger holding previous-scheme rows must still open on upgrade");
    assert_eq!(
        reopened.get_body(&v1_key),
        Some(wf.clone()),
        "the legacy row stays resolvable under the id an in-flight run pinned"
    );

    // And the same recipe republishes cleanly under its CURRENT identity, beside it.
    let (id, outcome) = reopened.publish_body(wf).unwrap();
    assert_eq!(id, v2_key, "new writes use the current scheme");
    assert!(matches!(outcome, BodyOutcome::Inserted(_)));
}

/// The check above must not have been softened into "accept anything".
#[test]
fn a_tampered_body_is_still_refused_on_open() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bodies.db");
    let honest = body(8, 0xDD);
    let other = body(9, 0xEE);

    {
        let ledger = SqliteBodyLedger::open(&path).unwrap();
        drop(ledger);
        let conn = rusqlite::Connection::open(&path).unwrap();
        // A row claiming one recipe's key while holding a DIFFERENT recipe's bytes —
        // matching neither the v1 nor the v2 derivation of what is stored.
        let bytes = bincode::serde::encode_to_vec(
            &other,
            bincode::config::standard()
                .with_little_endian()
                .with_fixed_int_encoding(),
        )
        .unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO bodies (manifest_id, body_bytes) VALUES (?1, ?2)",
            rusqlite::params![body_manifest_id(&honest).unwrap().0.as_slice(), &bytes[..]],
        )
        .unwrap();
    }

    let Err(err) = SqliteBodyLedger::open(&path) else {
        panic!("a body that matches NEITHER identity scheme is still a tampered row");
    };
    assert!(
        err.to_string().contains("content mismatch"),
        "refused for the tamper reason, not something incidental: {err}"
    );
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
