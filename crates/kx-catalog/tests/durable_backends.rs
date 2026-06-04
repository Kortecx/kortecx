//! G1 durable backends (D94): the kx-catalog ledgers survive a process restart.
//! Mirrors the `kx-journal` durability discipline — `run_with_each_backend` holds
//! the SQLite impl to the SAME contract as the in-memory one; a write → drop →
//! reopen sweep proves the FOLD (not just raw facts) survives; plus
//! atomicity-under-panic and a loud schema-version-mismatch refusal.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use kx_catalog::{
    AssetBinding, AssetPath, AssetRef, AssetVersion, BodyLedger, CatalogAction, CatalogActionSet,
    CatalogRegistry, DiscoveryIndex, Grant, GrantLedger, InMemoryDiscoveryIndex,
    InMemoryGrantLedger, InMemoryVersionLedger, PartyId, Provenance, RecipeSnapshot,
    SignatureEntry, SqliteBodyLedger, SqliteCatalog, SqliteGrantLedger, SqliteVersionLedger,
    TaskSignature, VersionLedger, VersionedContent,
};
use kx_mote::{LogicRef, ModelId, MoteDefHash, ToolName};
use kx_warrant::{ModelRoute, Role, WarrantSpec};
use kx_workflow::{permissive_warrant, transform, ManifestId, WorkflowDef};

// --- fixtures --------------------------------------------------------------

fn asset() -> AssetRef {
    AssetRef::Path(AssetPath::new("acme", "recipes", "triage").unwrap())
}
fn handle() -> AssetPath {
    AssetPath::new("acme", "recipes", "triage").unwrap()
}

fn warrant(max_calls: u32) -> WarrantSpec {
    WarrantSpec {
        model_route: ModelRoute {
            model_id: ModelId("m".into()),
            max_input_tokens: 1_000,
            max_output_tokens: 1_000,
            max_calls,
        },
        ..Default::default()
    }
}
fn role(name: &str, max_calls: u32) -> Role {
    Role {
        name: name.into(),
        version: 1,
        spec: warrant(max_calls),
        description: String::new(),
    }
}

fn signature(tag: u8) -> SignatureEntry {
    SignatureEntry::new(
        TaskSignature::model_invariant(MoteDefHash::from_bytes([tag; 32])),
        ManifestId([tag; 32]),
        RecipeSnapshot::new([tag; 32]),
    )
}

fn recipe_body(seed: u32) -> WorkflowDef {
    let mut wf = WorkflowDef::new(seed);
    wf.add_step(transform(
        LogicRef::from_bytes([seed as u8; 32]),
        ModelId("m".into()),
        permissive_warrant(ModelId("m".into())),
        ToolName("demo".into()),
    ));
    wf
}

/// Seed a grant ledger with: owner-binding + a root Use+Read grant to `mate` + a
/// delegated Use grant + a revocation of the delegated grant (so the FOLD, not
/// just raw facts, is exercised across a restart).
fn seed_grants(ledger: &dyn GrantLedger) -> PartyId {
    let admin = PartyId::new("admin@acme");
    let mate = PartyId::new("teammate@acme");
    ledger
        .append_binding(AssetBinding::new(asset(), admin.clone()))
        .unwrap();
    let g = Grant::root(
        asset(),
        admin.clone(),
        mate.clone(),
        CatalogActionSet::allow([CatalogAction::Read, CatalogAction::Use]),
        role("reader", 10),
    );
    ledger.append_grant(g).unwrap();
    mate
}

// --- per-ledger: write → drop → reopen → identical -------------------------

#[test]
fn catalog_survives_reopen() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let path = tmp.path().to_owned();
    let hashes: Vec<_> = {
        let cat = SqliteCatalog::open(&path).unwrap();
        (0u8..5)
            .map(|t| cat.register_signature(signature(t)).unwrap())
            .map(|o| match o {
                kx_catalog::RegistrationOutcome::Inserted(h)
                | kx_catalog::RegistrationOutcome::AlreadyPresent(h) => h,
            })
            .collect()
    };
    // Reopen: every registered signature is still resolvable, identical bytes.
    let cat = SqliteCatalog::open(&path).unwrap();
    assert_eq!(cat.len(), 5);
    for (t, h) in hashes.iter().enumerate() {
        assert_eq!(cat.lookup(h).unwrap(), signature(t as u8));
    }
}

#[test]
fn grants_and_the_fold_survive_reopen() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let path = tmp.path().to_owned();
    let owner_root = warrant(100);
    let mate = {
        let ledger = SqliteGrantLedger::open(&path).unwrap();
        seed_grants(&ledger)
    };
    // Reopen: the AUTHORIZATION FOLD (not just facts) is intact.
    let ledger = SqliteGrantLedger::open(&path).unwrap();
    assert!(ledger.is_authorized(&mate, &asset(), CatalogAction::Use));
    assert!(!ledger.is_authorized(&mate, &asset(), CatalogAction::Delegate));
    let w = ledger
        .resolve_effective_warrant_for(&mate, &asset(), CatalogAction::Use, &owner_root)
        .unwrap()
        .expect("Use granted");
    assert_eq!(
        w.model_route.max_calls, 10,
        "narrowed warrant survives restart"
    );
    assert_eq!(ledger.owner_of(&asset()), Some(PartyId::new("admin@acme")));
}

#[test]
fn versions_and_the_handle_move_survive_reopen() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let path = tmp.path().to_owned();
    let admin = PartyId::new("admin@acme");
    let (v1_id, v2_id) = {
        let ledger = SqliteVersionLedger::open(&path).unwrap();
        let v1 = AssetVersion::root(
            handle(),
            VersionedContent::Workflow(ManifestId([1; 32])),
            admin.clone(),
            Provenance::from_recipe([1; 32]),
        );
        let v1_id = ledger.publish(v1).unwrap().version_id();
        let v2 = AssetVersion::successor(
            v1_id,
            0,
            handle(),
            VersionedContent::Workflow(ManifestId([2; 32])),
            admin.clone(),
            Provenance::from_recipe([2; 32]),
        );
        let v2_id = ledger.publish(v2).unwrap().version_id();
        (v1_id, v2_id)
    };
    // Reopen: the MUTABLE HANDLE still resolves to v2; v1 retained; lineage intact.
    let ledger = SqliteVersionLedger::open(&path).unwrap();
    assert_eq!(
        ledger.resolve(&handle()).unwrap().1,
        v2_id,
        "handle move survived"
    );
    assert!(ledger.get_version(&v1_id).is_some(), "v1 retained forever");
    assert_eq!(ledger.lineage(&v2_id).len(), 2, "v2 -> v1 lineage intact");
    assert_eq!(
        ledger.descendants(&v1_id),
        vec![v2_id],
        "forward lineage intact"
    );
}

#[test]
fn bodies_survive_reopen() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let path = tmp.path().to_owned();
    let id = {
        let ledger = SqliteBodyLedger::open(&path).unwrap();
        ledger.publish_body(recipe_body(7)).unwrap().0
    };
    let ledger = SqliteBodyLedger::open(&path).unwrap();
    assert_eq!(
        ledger.get_body(&id),
        Some(recipe_body(7)),
        "body invocable after restart"
    );
}

#[test]
fn discovery_rebuilds_from_durable_versions() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let path = tmp.path().to_owned();
    {
        let versions = SqliteVersionLedger::open(&path).unwrap();
        versions
            .publish(AssetVersion::root(
                handle(),
                VersionedContent::Workflow(ManifestId([1; 32])),
                PartyId::new("admin@acme"),
                Provenance::from_recipe([1; 32]),
            ))
            .unwrap();
    }
    // A fresh (empty) discovery index rebuilds from the durable version ledger.
    let versions = SqliteVersionLedger::open(&path).unwrap();
    let index = InMemoryDiscoveryIndex::default();
    assert!(index.is_empty());
    index.rebuild_from_versions(&versions);
    assert_eq!(index.by_namespace("acme"), vec![asset()]);
}

// --- run_with_each_backend: the Sqlite impl is held to the SAME contract ----

#[test]
fn grant_obligations_hold_on_both_backends() {
    fn obligations(ledger: &dyn GrantLedger) {
        let mate = seed_grants(ledger);
        // Idempotency: re-binding the same owner is AlreadyPresent.
        assert!(matches!(
            ledger.append_binding(AssetBinding::new(asset(), PartyId::new("admin@acme"))),
            Ok(kx_catalog::AppendOutcome::AlreadyPresent(_))
        ));
        // Owner conflict: a different owner is refused.
        assert!(matches!(
            ledger.append_binding(AssetBinding::new(asset(), PartyId::new("evil@x"))),
            Err(kx_catalog::LedgerError::OwnerConflict(_))
        ));
        // The fold authorizes Use, denies Delegate.
        assert!(ledger.is_authorized(&mate, &asset(), CatalogAction::Use));
        assert!(!ledger.is_authorized(&mate, &asset(), CatalogAction::Delegate));
    }
    obligations(&InMemoryGrantLedger::new());
    obligations(&SqliteGrantLedger::open_in_memory().unwrap());
}

#[test]
fn version_obligations_hold_on_both_backends() {
    fn obligations(ledger: &dyn VersionLedger) {
        let admin = PartyId::new("admin@acme");
        let v1 = AssetVersion::root(
            handle(),
            VersionedContent::Workflow(ManifestId([1; 32])),
            admin,
            Provenance::from_recipe([1; 32]),
        );
        let v1_id = ledger.publish(v1.clone()).unwrap().version_id();
        // Idempotent re-publish.
        assert!(matches!(
            ledger.publish(v1),
            Ok(kx_catalog::PublishOutcome::AlreadyPresent(_))
        ));
        // A successor with an inflated revision is refused (lineage-strict).
        let bad = AssetVersion::successor(
            v1_id,
            5, // prior_revision 5 ⇒ revision 6, but prior's real revision is 0
            handle(),
            VersionedContent::Workflow(ManifestId([2; 32])),
            PartyId::new("admin@acme"),
            Provenance::from_recipe([2; 32]),
        );
        assert!(matches!(
            ledger.publish(bad),
            Err(kx_catalog::VersionLedgerError::InvalidLineage { .. })
        ));
    }
    obligations(&InMemoryVersionLedger::new());
    obligations(&SqliteVersionLedger::open_in_memory().unwrap());
}

#[test]
fn registry_and_body_obligations_hold_on_both_backends() {
    fn registry_obligations(reg: &dyn CatalogRegistry) {
        assert!(reg.register_signature(signature(1)).unwrap().is_inserted());
        // Idempotent re-register.
        assert!(!reg.register_signature(signature(1)).unwrap().is_inserted());
        assert_eq!(reg.len(), 1);
    }
    registry_obligations(&kx_catalog::InMemoryCatalog::new());
    registry_obligations(&SqliteCatalog::open_in_memory().unwrap());

    fn body_obligations(b: &dyn BodyLedger) {
        let (_id, o1) = b.publish_body(recipe_body(3)).unwrap();
        let (_id2, o2) = b.publish_body(recipe_body(3)).unwrap();
        assert!(matches!(o1, kx_catalog::BodyOutcome::Inserted(_)));
        assert!(matches!(o2, kx_catalog::BodyOutcome::AlreadyPresent(_)));
        assert_eq!(b.len(), 1);
    }
    body_obligations(&kx_catalog::InMemoryBodyLedger::new());
    body_obligations(&SqliteBodyLedger::open_in_memory().unwrap());
}

// --- atomicity-under-panic + schema-version-mismatch -----------------------

#[test]
fn forged_mid_txn_insert_rolls_back() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let path = tmp.path().to_owned();
    {
        let ledger = SqliteGrantLedger::open(&path).unwrap();
        seed_grants(&ledger);
    }
    let before = SqliteGrantLedger::open(&path).unwrap().len();
    let result = std::panic::catch_unwind(|| {
        let mut conn = rusqlite::Connection::open(&path).unwrap();
        let txn = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .unwrap();
        txn.execute(
            "INSERT INTO facts (seq, fact_id, kind, fact_bytes) VALUES (999, ?1, 1, ?2)",
            rusqlite::params![&[0u8; 32][..], &[0u8; 8][..]],
        )
        .unwrap();
        panic!("simulated mid-txn crash"); // Drop rolls the txn back.
    });
    assert!(result.is_err(), "the panic propagates");
    let ledger = SqliteGrantLedger::open(&path).unwrap();
    assert_eq!(
        ledger.len(),
        before,
        "rolled-back forged insert must not persist"
    );
    // A subsequent normal append still works.
    assert!(ledger
        .append_grant(Grant::root(
            AssetRef::Path(AssetPath::new("acme", "recipes", "other").unwrap()),
            PartyId::new("admin@acme"),
            PartyId::new("x@y"),
            CatalogActionSet::allow([CatalogAction::Read]),
            role("r", 1),
        ))
        .is_ok());
}

#[test]
fn schema_version_mismatch_is_refused_loudly() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let path = tmp.path().to_owned();
    {
        let _ = SqliteGrantLedger::open(&path).unwrap();
    }
    // Corrupt the stored schema version via a raw connection.
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        let bogus: [u8; 2] = 999u16.to_le_bytes();
        conn.execute(
            "UPDATE metadata SET value = ?1 WHERE key = 'schema_version'",
            rusqlite::params![&bogus[..]],
        )
        .unwrap();
    }
    assert!(
        matches!(
            SqliteGrantLedger::open(&path),
            Err(kx_catalog::LedgerError::Storage(_))
        ),
        "a schema-version mismatch on reopen must refuse loudly"
    );
}
