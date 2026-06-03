// SPDX-License-Identifier: Apache-2.0
//! Integration + security + exit-gate tests for M7.2 governance (D86).
//!
//! Drives the real-life enterprise use cases end to end (an org admin grants a
//! recipe to a teammate under a narrowed role, then revokes; a consulting
//! delegation chain narrows each hop and a parent revocation cascades; a data
//! scientist holds two grants on one model under different warrants), proves the
//! security invariants (no widening, no laundering, no revocation bypass, no
//! confused deputy, deep-chain fail-closed, fail-closed default), and asserts the
//! milestone exit gate — including the compiler-enforced wall keeping catalog
//! governance OFF the guarantee path (no guarantee-path crate may import
//! `kx-catalog`).
//!
//! `Kind 4 (chaos)` scoping (D95, honest): an in-memory ledger has no process
//! kill/replay — durable crash-recovery is owned by a future persistent backend
//! (D94). The analogue exercised here is concurrency/poison-safety +
//! idempotent-replay-of-appends (`concurrent_appends_and_queries_are_safe`).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::sync::Arc;

use kx_catalog::{
    AssetBinding, AssetPath, AssetRef, AssetVersion, CatalogAction, CatalogActionSet,
    GovernedCatalog, GovernedError, Grant, GrantId, GrantLedger, InMemoryGrantLedger,
    InMemoryVersionLedger, LedgerFact, PartyId, Provenance, Revocation, TaskSignatureHash,
    VersionLedger, VersionLedgerError, VersionedContent,
};
use kx_dataset::DatasetId;
use kx_mote::{ModelId, MoteId};
use kx_warrant::{ModelRoute, ResourceCeiling, Role, SecretRef, SecretScope, WarrantSpec};

// ---- fixtures ---------------------------------------------------------------

fn path(ns: &str, col: &str, name: &str) -> AssetRef {
    AssetRef::Path(AssetPath::new(ns, col, name).unwrap())
}

fn warrant_calls(max_calls: u32) -> WarrantSpec {
    WarrantSpec {
        model_route: ModelRoute {
            model_id: ModelId("m".into()),
            max_input_tokens: 1_000,
            max_output_tokens: 1_000,
            max_calls,
        },
        resource_ceiling: ResourceCeiling {
            cpu_milli: 1_000,
            mem_bytes: 1 << 20,
            wall_clock_ms: 1_000,
            fd_count: 16,
            disk_bytes: 1 << 20,
        },
        ..Default::default()
    }
}

fn role(name: &str, max_calls: u32) -> Role {
    Role {
        name: name.into(),
        version: 1,
        spec: warrant_calls(max_calls),
        description: String::new(),
    }
}

// ---- Integration: real-life enterprise scenarios ----------------------------

/// Scenario 1 — Acme Research: the org admin grants a teammate {Read,Use} on a
/// recipe under a narrowed role, then revokes.
#[test]
fn org_admin_grants_then_revokes_end_to_end() {
    let ledger = InMemoryGrantLedger::new();
    let recipe = path("acme", "research", "lit-review");
    let admin = PartyId::new("admin@acme");
    let mate = PartyId::new("teammate@acme");
    let owner_root = warrant_calls(100);

    ledger
        .append_binding(AssetBinding::new(recipe.clone(), admin.clone()))
        .unwrap();
    let g = Grant::root(
        recipe.clone(),
        admin.clone(),
        mate.clone(),
        CatalogActionSet::allow([CatalogAction::Read, CatalogAction::Use]),
        role("read-only-research", 10),
    );
    let gid = g.grant_id();
    ledger.append_grant(g).unwrap();

    assert!(ledger.is_authorized(&mate, &recipe, CatalogAction::Read));
    assert!(ledger.is_authorized(&mate, &recipe, CatalogAction::Use));
    assert!(!ledger.is_authorized(&mate, &recipe, CatalogAction::Delegate));
    let w = ledger
        .resolve_effective_warrant_for(&mate, &recipe, CatalogAction::Use, &owner_root)
        .unwrap()
        .expect("Use granted");
    assert_eq!(w.model_route.max_calls, 10, "narrowed to min(10,100)");

    ledger
        .append_revocation(Revocation::new(gid, admin))
        .unwrap();
    assert!(!ledger.is_authorized(&mate, &recipe, CatalogAction::Use));
}

/// Scenario 2 — consulting delegation: owner → lead {Use,Delegate}@8 → intern
/// {Use}@3. The runtime warrant narrows min(10,8,3)=3 at the leaf; revoking the
/// parent cascades to the intern.
#[test]
fn delegation_chain_narrows_each_hop_and_parent_revoke_cascades() {
    let ledger = InMemoryGrantLedger::new();
    let asset = path("contoso", "tools", "summarize");
    let owner = PartyId::new("owner");
    let lead = PartyId::new("lead");
    let intern = PartyId::new("intern");
    let owner_root = warrant_calls(10);

    ledger
        .append_binding(AssetBinding::new(asset.clone(), owner.clone()))
        .unwrap();
    let root = Grant::root(
        asset.clone(),
        owner.clone(),
        lead.clone(),
        CatalogActionSet::allow([CatalogAction::Use, CatalogAction::Delegate]),
        role("lead", 8),
    );
    let root_id = root.grant_id();
    ledger.append_grant(root).unwrap();
    ledger
        .append_grant(Grant::delegated(
            root_id,
            asset.clone(),
            lead.clone(),
            intern.clone(),
            CatalogActionSet::allow([CatalogAction::Use]),
            role("intern", 3),
        ))
        .unwrap();

    assert!(ledger.is_authorized(&intern, &asset, CatalogAction::Use));
    assert!(!ledger.is_authorized(&intern, &asset, CatalogAction::Delegate));
    let w = ledger
        .resolve_effective_warrant_for(&intern, &asset, CatalogAction::Use, &owner_root)
        .unwrap()
        .expect("intern may Use");
    assert_eq!(w.model_route.max_calls, 3, "min(10,8,3) narrowed each hop");

    // Owner revokes the ROOT (lead's) grant → cascades to the intern's chain.
    ledger
        .append_revocation(Revocation::new(root_id, owner))
        .unwrap();
    assert!(
        !ledger.is_authorized(&intern, &asset, CatalogAction::Use),
        "cascade"
    );
    assert!(!ledger.is_authorized(&lead, &asset, CatalogAction::Use));
}

/// Scenario 3 — multi-grant aligned warrant: a data scientist holds Use under a
/// tight warrant and Read under a wide one on the SAME model; each action runs
/// under the warrant of a chain that conveys it (the bug-fix regression).
#[test]
fn multi_grant_warrant_is_action_aligned() {
    let ledger = InMemoryGrantLedger::new();
    let model = path("acme", "models", "summarizer");
    let owner = PartyId::new("owner");
    let ds = PartyId::new("data-scientist");
    let owner_root = warrant_calls(100);

    ledger
        .append_binding(AssetBinding::new(model.clone(), owner.clone()))
        .unwrap();
    ledger
        .append_grant(Grant::root(
            model.clone(),
            owner.clone(),
            ds.clone(),
            CatalogActionSet::allow([CatalogAction::Use]),
            role("tight-use", 2),
        ))
        .unwrap();
    ledger
        .append_grant(Grant::root(
            model.clone(),
            owner,
            ds.clone(),
            CatalogActionSet::allow([CatalogAction::Read]),
            role("wide-read", 80),
        ))
        .unwrap();

    let use_calls = ledger
        .resolve_effective_warrant_for(&ds, &model, CatalogAction::Use, &owner_root)
        .unwrap()
        .unwrap()
        .model_route
        .max_calls;
    let read_calls = ledger
        .resolve_effective_warrant_for(&ds, &model, CatalogAction::Read, &owner_root)
        .unwrap()
        .unwrap()
        .model_route
        .max_calls;
    assert_eq!(use_calls, 2, "Use runs under the tight warrant");
    assert_eq!(read_calls, 80, "Read runs under the wide warrant");
}

// ---- Security invariants ----------------------------------------------------

#[test]
fn widen_attempt_is_refused() {
    let ledger = InMemoryGrantLedger::new();
    let asset = path("ns", "c", "n");
    let owner = PartyId::new("owner");
    let mate = PartyId::new("mate");
    let owner_root = warrant_calls(10); // secret_scope = None

    let mut wide = warrant_calls(10);
    wide.secret_scope =
        SecretScope::AllowList([SecretRef("db-password".into())].into_iter().collect());
    ledger
        .append_binding(AssetBinding::new(asset.clone(), owner.clone()))
        .unwrap();
    ledger
        .append_grant(Grant::root(
            asset.clone(),
            owner,
            mate.clone(),
            CatalogActionSet::allow([CatalogAction::Use]),
            Role {
                name: "wide".into(),
                version: 1,
                spec: wide,
                description: String::new(),
            },
        ))
        .unwrap();

    // The widen surfaces loudly as a typed error — never a silently-wider warrant.
    assert!(ledger
        .resolve_effective_warrant_for(&mate, &asset, CatalogAction::Use, &owner_root)
        .is_err());
}

#[test]
fn delegation_laundering_is_refused() {
    let ledger = InMemoryGrantLedger::new();
    let asset = path("ns", "c", "n");
    let owner = PartyId::new("owner");
    let lead = PartyId::new("lead");
    let intern = PartyId::new("intern");

    ledger
        .append_binding(AssetBinding::new(asset.clone(), owner.clone()))
        .unwrap();
    // Lead holds Use + Delegate but NOT Register.
    let root = Grant::root(
        asset.clone(),
        owner,
        lead.clone(),
        CatalogActionSet::allow([CatalogAction::Use, CatalogAction::Delegate]),
        role("lead", 10),
    );
    let root_id = root.grant_id();
    ledger.append_grant(root).unwrap();
    // Lead tries to delegate Register (which it never held) to the intern.
    ledger
        .append_grant(Grant::delegated(
            root_id,
            asset.clone(),
            lead,
            intern.clone(),
            CatalogActionSet::allow([CatalogAction::Register]),
            role("intern", 10),
        ))
        .unwrap();

    // Register was never the lead's to give: the intern gets nothing for it.
    assert!(!ledger.is_authorized(&intern, &asset, CatalogAction::Register));
    assert!(!ledger.is_authorized(&intern, &asset, CatalogAction::Use));
}

#[test]
fn delegation_without_delegate_conveys_nothing() {
    let ledger = InMemoryGrantLedger::new();
    let asset = path("ns", "c", "n");
    let owner = PartyId::new("owner");
    let lead = PartyId::new("lead");
    let intern = PartyId::new("intern");

    ledger
        .append_binding(AssetBinding::new(asset.clone(), owner.clone()))
        .unwrap();
    // Lead holds Use but NOT Delegate.
    let root = Grant::root(
        asset.clone(),
        owner,
        lead.clone(),
        CatalogActionSet::allow([CatalogAction::Use]),
        role("lead", 10),
    );
    let root_id = root.grant_id();
    ledger.append_grant(root).unwrap();
    ledger
        .append_grant(Grant::delegated(
            root_id,
            asset.clone(),
            lead,
            intern.clone(),
            CatalogActionSet::allow([CatalogAction::Use]),
            role("intern", 10),
        ))
        .unwrap();

    // A delegator without Delegate mints nothing.
    assert!(!ledger.is_authorized(&intern, &asset, CatalogAction::Use));
}

#[test]
fn revocation_bypass_is_impossible() {
    let ledger = InMemoryGrantLedger::new();
    let asset = path("ns", "c", "n");
    let owner = PartyId::new("owner");
    let mate = PartyId::new("mate");

    ledger
        .append_binding(AssetBinding::new(asset.clone(), owner.clone()))
        .unwrap();
    let g = Grant::root(
        asset.clone(),
        owner.clone(),
        mate.clone(),
        CatalogActionSet::allow([CatalogAction::Use]),
        role("r", 10),
    );
    let gid = g.grant_id();
    ledger.append_grant(g.clone()).unwrap();
    ledger
        .append_revocation(Revocation::new(gid, owner))
        .unwrap();
    assert!(!ledger.is_authorized(&mate, &asset, CatalogAction::Use));

    // Re-appending the identical grant is an idempotent no-op — it does NOT
    // resurrect authority (the revocation fact stands; the grant id is unchanged).
    let outcome = ledger.append_grant(g).unwrap();
    assert!(!outcome.is_appended(), "re-append is idempotent");
    assert!(
        !ledger.is_authorized(&mate, &asset, CatalogAction::Use),
        "a revoked grant cannot be un-revoked by re-appending it"
    );
}

#[test]
fn confused_deputy_across_assets_is_refused() {
    let ledger = InMemoryGrantLedger::new();
    let asset_a = path("acme", "research", "lit-review");
    let asset_b = path("acme", "finance", "payroll");
    let owner = PartyId::new("owner");
    let mate = PartyId::new("mate");

    ledger
        .append_binding(AssetBinding::new(asset_a.clone(), owner.clone()))
        .unwrap();
    ledger
        .append_binding(AssetBinding::new(asset_b.clone(), owner.clone()))
        .unwrap();
    ledger
        .append_grant(Grant::root(
            asset_a.clone(),
            owner,
            mate.clone(),
            CatalogActionSet::all(),
            role("r", 10),
        ))
        .unwrap();

    // Full authority on A conveys NOTHING on B.
    assert!(ledger.is_authorized(&mate, &asset_a, CatalogAction::Use));
    assert!(!ledger.is_authorized(&mate, &asset_b, CatalogAction::Read));
    assert!(!ledger.is_authorized(&mate, &asset_b, CatalogAction::Use));
}

#[test]
fn only_grantor_or_owner_can_revoke() {
    let ledger = InMemoryGrantLedger::new();
    let asset = path("ns", "c", "n");
    let owner = PartyId::new("owner");
    let mate = PartyId::new("mate");
    let stranger = PartyId::new("stranger");

    ledger
        .append_binding(AssetBinding::new(asset.clone(), owner.clone()))
        .unwrap();
    let g = Grant::root(
        asset.clone(),
        owner.clone(),
        mate.clone(),
        CatalogActionSet::allow([CatalogAction::Use]),
        role("r", 10),
    );
    let gid = g.grant_id();
    ledger.append_grant(g).unwrap();

    // A stranger's revocation is recorded-but-inert.
    ledger
        .append_revocation(Revocation::new(gid, stranger))
        .unwrap();
    assert!(
        ledger.is_authorized(&mate, &asset, CatalogAction::Use),
        "stranger cannot revoke"
    );

    // The owner (== grantor of the root) can.
    ledger
        .append_revocation(Revocation::new(gid, owner))
        .unwrap();
    assert!(!ledger.is_authorized(&mate, &asset, CatalogAction::Use));
}

#[test]
fn deep_delegation_chain_is_bounded_and_fail_closed() {
    let ledger = InMemoryGrantLedger::new();
    let asset = path("ns", "c", "n");
    let owner = PartyId::new("owner");
    ledger
        .append_binding(AssetBinding::new(asset.clone(), owner.clone()))
        .unwrap();

    // Build a chain of 66 grants: owner → p1 → p2 → … → p66, each conveying
    // {Use, Delegate}. The fold caps at MAX_DELEGATION_DEPTH (64).
    let parties: Vec<PartyId> = (1..=66).map(|i| PartyId::new(format!("p{i}"))).collect();
    let acts = CatalogActionSet::allow([CatalogAction::Use, CatalogAction::Delegate]);

    let root = Grant::root(
        asset.clone(),
        owner.clone(),
        parties[0].clone(),
        acts.clone(),
        role("r", 50),
    );
    let mut prev_id = root.grant_id();
    let mut prev_party = parties[0].clone();
    ledger.append_grant(root).unwrap();
    for p in &parties[1..] {
        let g = Grant::delegated(
            prev_id,
            asset.clone(),
            prev_party.clone(),
            p.clone(),
            acts.clone(),
            role("r", 50),
        );
        prev_id = g.grant_id();
        prev_party = p.clone();
        ledger.append_grant(g).unwrap();
    }

    // Depth 64 still resolves; depth 65+ fails closed (work-bounded, no overflow).
    assert!(
        ledger.is_authorized(&parties[63], &asset, CatalogAction::Use),
        "depth 64 resolves"
    );
    assert!(
        !ledger.is_authorized(&parties[64], &asset, CatalogAction::Use),
        "depth 65 fails closed"
    );
    assert!(
        !ledger.is_authorized(&parties[65], &asset, CatalogAction::Use),
        "depth 66 fails closed"
    );
}

#[test]
fn grant_on_unbound_asset_fails_closed() {
    let ledger = InMemoryGrantLedger::new();
    let asset = path("ns", "c", "n");
    let owner = PartyId::new("owner"); // never bound as the asset's owner
    let mate = PartyId::new("mate");
    // A root grant whose grantor is not the (absent) owner conveys nothing.
    ledger
        .append_grant(Grant::root(
            asset.clone(),
            owner,
            mate.clone(),
            CatalogActionSet::all(),
            role("r", 10),
        ))
        .unwrap();
    assert!(ledger.owner_of(&asset).is_none());
    assert!(!ledger.is_authorized(&mate, &asset, CatalogAction::Use));
}

#[test]
fn fail_closed_default_denies_all() {
    let ledger = InMemoryGrantLedger::new();
    let asset = path("ns", "c", "n");
    let nobody = PartyId::new("nobody");
    assert!(ledger.is_empty());
    assert!(!ledger.is_authorized(&nobody, &asset, CatalogAction::Read));
    assert!(ledger.effective_grants(&nobody, &asset).is_empty());
    assert!(ledger
        .resolve_effective_warrant_for(&nobody, &asset, CatalogAction::Use, &warrant_calls(10))
        .unwrap()
        .is_none());
}

// ---- Kind 4 analogue: concurrency / poison-safety + idempotent replay --------

#[test]
fn concurrent_appends_and_queries_are_safe() {
    let ledger = Arc::new(InMemoryGrantLedger::new());
    let asset = path("ns", "c", "n");
    let owner = PartyId::new("owner");
    ledger
        .append_binding(AssetBinding::new(asset.clone(), owner.clone()))
        .unwrap();

    // 8 threads each append a DISTINCT grant + query concurrently.
    let mut handles = Vec::new();
    for i in 0..8u32 {
        let l = Arc::clone(&ledger);
        let a = asset.clone();
        let o = owner.clone();
        handles.push(std::thread::spawn(move || {
            let mate = PartyId::new(format!("mate{i}"));
            l.append_grant(Grant::root(
                a.clone(),
                o,
                mate.clone(),
                CatalogActionSet::allow([CatalogAction::Use]),
                role("r", 10),
            ))
            .unwrap();
            assert!(l.is_authorized(&mate, &a, CatalogAction::Use));
        }));
    }
    for h in handles {
        h.join().unwrap();
    }

    // Idempotent replay: a burst of the SAME grant from many threads lands once.
    let shared = Grant::root(
        asset.clone(),
        owner,
        PartyId::new("shared"),
        CatalogActionSet::allow([CatalogAction::Use]),
        role("r", 10),
    );
    let mut bursts = Vec::new();
    for _ in 0..8 {
        let l = Arc::clone(&ledger);
        let g = shared.clone();
        bursts.push(std::thread::spawn(move || {
            l.append_grant(g).unwrap().is_appended()
        }));
    }
    let appended_count = bursts
        .into_iter()
        .map(|h| h.join().unwrap())
        .filter(|&b| b)
        .count();
    assert_eq!(
        appended_count, 1,
        "exactly one of the identical-grant bursts lands"
    );
}

// ---- Exit gate --------------------------------------------------------------

/// The structural wall: NO guarantee-path crate may depend on `kx-catalog`, so
/// the compiler can never wire catalog governance onto the identity / commit /
/// selection path (SN-8 / D70 / D87). Read the manifests directly — a future
/// `kx-catalog` edge into any of these is a compile-independent regression this
/// test catches.
#[test]
fn guarantee_path_does_not_depend_on_catalog() {
    let crates = [
        "kx-scheduler",
        "kx-executor",
        "kx-projection",
        "kx-inference",
        // The journal/identity core must also stay off kx-catalog (the new
        // versioning/lineage modules add no edge back into the frozen spine).
        "kx-mote",
        "kx-journal",
    ];
    for c in crates {
        let manifest = format!("{}/../{c}/Cargo.toml", env!("CARGO_MANIFEST_DIR"));
        let toml =
            std::fs::read_to_string(&manifest).unwrap_or_else(|e| panic!("read {manifest}: {e}"));
        assert!(
            !toml.contains("kx-catalog"),
            "{c} must NOT depend on kx-catalog (the SN-8 governance wall)"
        );
    }
}

/// The M7.2 milestone exit gate, composite: grant-never-widens + revoke-by-new-
/// fact + multi-grant action-aligned warrant + the closed action vocabulary +
/// the guarantee-path wall.
#[test]
fn m7_2_exit_gate() {
    // (a) closed action vocabulary.
    assert_eq!(CatalogAction::all().len(), 4);

    let ledger = InMemoryGrantLedger::new();
    let asset = path("acme", "models", "summarizer");
    let owner = PartyId::new("owner");
    let ds = PartyId::new("ds");
    let owner_root = warrant_calls(50);
    ledger
        .append_binding(AssetBinding::new(asset.clone(), owner.clone()))
        .unwrap();

    // (b) a grant never widens (resolved ≤ owner root) + multi-grant alignment.
    ledger
        .append_grant(Grant::root(
            asset.clone(),
            owner.clone(),
            ds.clone(),
            CatalogActionSet::allow([CatalogAction::Use]),
            role("use", 5),
        ))
        .unwrap();
    ledger
        .append_grant(Grant::root(
            asset.clone(),
            owner.clone(),
            ds.clone(),
            CatalogActionSet::allow([CatalogAction::Read]),
            role("read", 40),
        ))
        .unwrap();
    let use_w = ledger
        .resolve_effective_warrant_for(&ds, &asset, CatalogAction::Use, &owner_root)
        .unwrap()
        .unwrap();
    assert!(use_w.model_route.max_calls <= 50, "never widens");
    assert_eq!(
        use_w.model_route.max_calls, 5,
        "Use is action-aligned to its chain"
    );

    // (c) revoke-by-new-fact: the grant fact survives a revocation.
    let use_gid: GrantId = ledger
        .effective_grants(&ds, &asset)
        .grants_conveying(CatalogAction::Use)
        .next()
        .unwrap();
    let facts_before = ledger.list_facts().count();
    ledger
        .append_revocation(Revocation::new(use_gid, owner))
        .unwrap();
    assert!(
        ledger.list_facts().count() > facts_before,
        "revoke is a NEW fact"
    );
    assert_eq!(
        ledger
            .list_facts()
            .filter(|f| matches!(f, LedgerFact::Grant(_)))
            .count(),
        2,
        "no grant fact is mutated or removed"
    );
    assert!(
        !ledger.is_authorized(&ds, &asset, CatalogAction::Use),
        "revoked"
    );
    assert!(
        ledger.is_authorized(&ds, &asset, CatalogAction::Read),
        "unaffected action stands"
    );
}

// ============================================================================
// M7.2 — content-versioning + provenance/lineage (D82/D88 + D-LOCK-4)
// ============================================================================

fn vpath(name: &str) -> AssetPath {
    AssetPath::new("acme", "recipes", name).unwrap()
}

fn recipe(byte: u8) -> VersionedContent {
    VersionedContent::Recipe(TaskSignatureHash::from_bytes([byte; 32]))
}

/// A governed catalog where `owner` owns `handle` and `publisher` holds the given
/// actions on it.
fn governed_with(
    handle: &AssetPath,
    owner: &PartyId,
    publisher: &PartyId,
    actions: CatalogActionSet,
) -> GovernedCatalog<InMemoryGrantLedger, InMemoryVersionLedger> {
    let grants = InMemoryGrantLedger::new();
    let asset = AssetRef::Path(handle.clone());
    grants
        .append_binding(AssetBinding::new(asset.clone(), owner.clone()))
        .unwrap();
    grants
        .append_grant(Grant::root(
            asset,
            owner.clone(),
            publisher.clone(),
            actions,
            role("publisher", 10),
        ))
        .unwrap();
    GovernedCatalog::new(grants, InMemoryVersionLedger::new())
}

// ---- governance (publish gated by Register; reads gated by Read) -------------

#[test]
fn unauthorized_publish_is_refused() {
    let handle = vpath("summarize");
    let owner = PartyId::new("owner@acme");
    let mate = PartyId::new("mate@acme");
    // mate holds Read+Use but NOT Register.
    let catalog = governed_with(
        &handle,
        &owner,
        &mate,
        CatalogActionSet::allow([CatalogAction::Read, CatalogAction::Use]),
    );
    let v = AssetVersion::root(handle, recipe(1), mate, Provenance::from_recipe([1u8; 32]));
    let err = catalog.publish(v).unwrap_err();
    assert!(matches!(
        err,
        GovernedError::Unauthorized {
            action: CatalogAction::Register,
            ..
        }
    ));
    assert_eq!(catalog.versions().len(), 0, "nothing appended on refusal");
}

#[test]
fn register_granted_party_can_publish() {
    let handle = vpath("summarize");
    let owner = PartyId::new("owner@acme");
    let lead = PartyId::new("lead@acme");
    let catalog = governed_with(
        &handle,
        &owner,
        &lead,
        CatalogActionSet::allow([CatalogAction::Read, CatalogAction::Register]),
    );
    let v = AssetVersion::root(
        handle.clone(),
        recipe(1),
        lead.clone(),
        Provenance::from_recipe([1u8; 32]),
    );
    assert!(catalog.publish(v).unwrap().is_published());
    // lead also holds Read → can resolve.
    assert!(catalog.resolve(&lead, &handle).unwrap().is_some());
}

#[test]
fn confused_deputy_publish_across_handles_is_refused() {
    let handle_a = vpath("recipe-a");
    let owner = PartyId::new("owner@acme");
    let lead = PartyId::new("lead@acme");
    // lead holds Register on handle A only.
    let catalog = governed_with(
        &handle_a,
        &owner,
        &lead,
        CatalogActionSet::allow([CatalogAction::Register]),
    );
    // Publishing to a DIFFERENT handle B is refused (authority is asset-keyed).
    let handle_b = vpath("recipe-b");
    let v = AssetVersion::root(
        handle_b,
        recipe(2),
        lead,
        Provenance::from_recipe([2u8; 32]),
    );
    assert!(matches!(
        catalog.publish(v).unwrap_err(),
        GovernedError::Unauthorized { .. }
    ));
}

#[test]
fn revoked_register_cannot_publish_but_prior_versions_retained() {
    let handle = vpath("summarize");
    let owner = PartyId::new("owner@acme");
    let lead = PartyId::new("lead@acme");
    let grants = InMemoryGrantLedger::new();
    let asset = AssetRef::Path(handle.clone());
    grants
        .append_binding(AssetBinding::new(asset.clone(), owner.clone()))
        .unwrap();
    let reg_grant = Grant::root(
        asset.clone(),
        owner.clone(),
        lead.clone(),
        CatalogActionSet::allow([CatalogAction::Read, CatalogAction::Register]),
        role("publisher", 10),
    );
    let reg_gid = reg_grant.grant_id();
    grants.append_grant(reg_grant).unwrap();
    let catalog = GovernedCatalog::new(grants, InMemoryVersionLedger::new());

    // v1 publishes fine.
    let v1 = AssetVersion::root(
        handle.clone(),
        recipe(1),
        lead.clone(),
        Provenance::from_recipe([1u8; 32]),
    );
    let v1_id = catalog.publish(v1).unwrap().version_id();

    // Owner revokes Register (a NEW fact). v2 is now refused.
    catalog
        .grants()
        .append_revocation(Revocation::new(reg_gid, owner))
        .unwrap();
    let v2 = AssetVersion::successor(
        v1_id,
        0,
        handle.clone(),
        recipe(2),
        lead.clone(),
        Provenance::from_recipe([2u8; 32]),
    );
    assert!(matches!(
        catalog.publish(v2).unwrap_err(),
        GovernedError::Unauthorized { .. }
    ));
    // v1 is retained and still resolves (revocation stops FUTURE publishes only).
    assert_eq!(catalog.versions().resolve(&handle).unwrap().1, v1_id);
    assert!(catalog.versions().get_version(&v1_id).is_some());
}

#[test]
fn governed_read_requires_read() {
    let handle = vpath("summarize");
    let owner = PartyId::new("owner@acme");
    let lead = PartyId::new("lead@acme");
    let stranger = PartyId::new("stranger@acme");
    let catalog = governed_with(
        &handle,
        &owner,
        &lead,
        CatalogActionSet::allow([CatalogAction::Read, CatalogAction::Register]),
    );
    let v = AssetVersion::root(
        handle.clone(),
        recipe(1),
        lead.clone(),
        Provenance::from_recipe([1u8; 32]),
    );
    catalog.publish(v).unwrap();
    // lead has Read → ok; stranger has nothing → refused.
    assert!(catalog.resolve(&lead, &handle).unwrap().is_some());
    assert!(matches!(
        catalog.resolve(&stranger, &handle).unwrap_err(),
        GovernedError::Unauthorized {
            action: CatalogAction::Read,
            ..
        }
    ));
}

// ---- lineage integrity + advisory wall ---------------------------------------

#[test]
fn provenance_forgery_via_fake_prior_is_refused_at_publish() {
    // A forged "successor" grafting a FOREIGN handle's version as its prior is
    // refused fail-closed at publish — stronger than truncate-on-read: the forged
    // edge never lands, so the forward (descendants) and backward (lineage) folds
    // can never disagree about it (the review's consistency finding).
    let ledger = InMemoryVersionLedger::new();
    let foreign = AssetVersion::root(
        vpath("other"),
        recipe(9),
        PartyId::new("x"),
        Provenance::from_recipe([9u8; 32]),
    );
    let foreign_id = ledger.publish(foreign).unwrap().version_id();
    let forged = AssetVersion::successor(
        foreign_id,
        0,
        vpath("summarize"),
        recipe(1),
        PartyId::new("x"),
        Provenance::from_recipe([1u8; 32]),
    );
    assert!(matches!(
        ledger.publish(forged).unwrap_err(),
        VersionLedgerError::InvalidLineage { .. }
    ));
    // The foreign version gained NO descendant (the graft never landed), and the
    // target handle was never created.
    assert!(ledger.descendants(&foreign_id).is_empty());
    assert!(ledger.resolve(&vpath("summarize")).is_none());
}

#[test]
fn lineage_never_gates_publish() {
    // A version with empty/garbage provenance still publishes (D84: provenance is
    // advisory; only the Register grant gates). Build it via the governed surface.
    let handle = vpath("summarize");
    let owner = PartyId::new("owner@acme");
    let lead = PartyId::new("lead@acme");
    let catalog = governed_with(
        &handle,
        &owner,
        &lead,
        CatalogActionSet::allow([CatalogAction::Register]),
    );
    // Provenance points at a recipe that was never run / does not exist — advisory.
    let v = AssetVersion::root(handle, recipe(1), lead, Provenance::from_recipe([0xAB; 32]));
    assert!(
        catalog.publish(v).unwrap().is_published(),
        "advisory provenance never blocks a Register-authorized publish"
    );
}

#[test]
fn version_fold_is_deterministic_on_refold() {
    let ledger = InMemoryVersionLedger::new();
    let handle = vpath("summarize");
    let v1 = AssetVersion::root(
        handle.clone(),
        recipe(1),
        PartyId::new("p"),
        Provenance::from_recipe([1u8; 32]),
    );
    let v1_id = ledger.publish(v1).unwrap().version_id();
    let v2 = AssetVersion::successor(
        v1_id,
        0,
        handle,
        recipe(2),
        PartyId::new("p"),
        Provenance::from_recipe([2u8; 32]),
    );
    let v2_id = ledger.publish(v2).unwrap().version_id();
    let a: Vec<_> = ledger
        .lineage(&v2_id)
        .iter()
        .map(AssetVersion::version_id)
        .collect();
    let b: Vec<_> = ledger
        .lineage(&v2_id)
        .iter()
        .map(AssetVersion::version_id)
        .collect();
    assert_eq!(a, b, "lineage re-fold is byte-identical");
    assert_eq!(a, vec![v2_id, v1_id]);
}

// ---- Kind 4 analogue: concurrency / poison-safety + idempotent replay --------

#[test]
fn concurrent_version_publishes_and_resolves_are_safe() {
    let ledger = Arc::new(InMemoryVersionLedger::new());

    // 8 threads each publish a DISTINCT handle + resolve concurrently.
    let mut handles = Vec::new();
    for i in 0..8u32 {
        let l = Arc::clone(&ledger);
        handles.push(std::thread::spawn(move || {
            let h = AssetPath::new("acme", "recipes", format!("r{i}")).unwrap();
            let v = AssetVersion::root(
                h.clone(),
                recipe(u8::try_from(i).unwrap()),
                PartyId::new("p"),
                Provenance::from_recipe([1u8; 32]),
            );
            l.publish(v).unwrap();
            assert!(l.resolve(&h).is_some());
        }));
    }
    for h in handles {
        h.join().unwrap();
    }

    // Idempotent replay: a burst of the SAME version from many threads lands once.
    let shared = AssetVersion::root(
        vpath("shared"),
        recipe(42),
        PartyId::new("p"),
        Provenance::from_recipe([2u8; 32]),
    );
    let mut bursts = Vec::new();
    for _ in 0..8 {
        let l = Arc::clone(&ledger);
        let v = shared.clone();
        bursts.push(std::thread::spawn(move || {
            l.publish(v).unwrap().is_published()
        }));
    }
    let published = bursts
        .into_iter()
        .map(|h| h.join().unwrap())
        .filter(|&b| b)
        .count();
    assert_eq!(published, 1, "exactly one of the identical bursts lands");
}

// ---- Integration: real-life enterprise scenarios -----------------------------

/// Scenario A — a platform team publishes a recipe v1, improves it, publishes v2;
/// the handle resolves v2, v1 stays exact-pinnable (D87), lineage traces v2 → v1.
#[test]
fn platform_team_publishes_recipe_v1_then_v2() {
    let handle = vpath("summarize");
    let owner = PartyId::new("platform-team@acme");
    let catalog = governed_with(
        &handle,
        &owner,
        &owner, // the team owns and publishes
        CatalogActionSet::allow([CatalogAction::Read, CatalogAction::Register]),
    );

    let v1 = AssetVersion::root(
        handle.clone(),
        recipe(0x11),
        owner.clone(),
        Provenance::from_recipe([0x11; 32]),
    );
    let v1_id = catalog.publish(v1).unwrap().version_id();

    // Improve the recipe → publish v2 (new fingerprint, prior = v1).
    let v2 = AssetVersion::successor(
        v1_id,
        0,
        handle.clone(),
        recipe(0x22),
        owner.clone(),
        Provenance::from_recipe([0x22; 32]),
    );
    let v2_id = catalog.publish(v2).unwrap().version_id();

    // The handle now resolves v2; v1 is still pinnable by its exact id (D87).
    assert_eq!(catalog.resolve(&owner, &handle).unwrap().unwrap().1, v2_id);
    assert!(catalog.versions().get_version(&v1_id).is_some());
    // Lineage traces v2 → v1.
    let lin: Vec<_> = catalog
        .lineage(&owner, &v2_id)
        .unwrap()
        .iter()
        .map(AssetVersion::version_id)
        .collect();
    assert_eq!(lin, vec![v2_id, v1_id]);
    assert_eq!(catalog.history(&owner, &handle).unwrap().len(), 2);
}

/// Scenario B — governed publishing: a lead with `Register` may publish a new
/// version; an intern with only `Use` is refused.
#[test]
fn governed_publishing_intern_with_use_is_refused() {
    let handle = vpath("summarize");
    let owner = PartyId::new("owner@acme");
    let lead = PartyId::new("lead@acme");
    let intern = PartyId::new("intern@acme");
    let asset = AssetRef::Path(handle.clone());

    let grants = InMemoryGrantLedger::new();
    grants
        .append_binding(AssetBinding::new(asset.clone(), owner.clone()))
        .unwrap();
    grants
        .append_grant(Grant::root(
            asset.clone(),
            owner.clone(),
            lead.clone(),
            CatalogActionSet::allow([CatalogAction::Read, CatalogAction::Register]),
            role("lead", 10),
        ))
        .unwrap();
    grants
        .append_grant(Grant::root(
            asset,
            owner,
            intern.clone(),
            CatalogActionSet::allow([CatalogAction::Read, CatalogAction::Use]),
            role("intern", 10),
        ))
        .unwrap();
    let catalog = GovernedCatalog::new(grants, InMemoryVersionLedger::new());

    // Lead publishes.
    let v1 = AssetVersion::root(
        handle.clone(),
        recipe(1),
        lead,
        Provenance::from_recipe([1u8; 32]),
    );
    let v1_id = catalog.publish(v1).unwrap().version_id();

    // Intern (Use, not Register) tries to publish a new version → refused.
    let v2 = AssetVersion::successor(
        v1_id,
        0,
        handle.clone(),
        recipe(2),
        intern,
        Provenance::from_recipe([2u8; 32]),
    );
    assert!(matches!(
        catalog.publish(v2).unwrap_err(),
        GovernedError::Unauthorized {
            action: CatalogAction::Register,
            ..
        }
    ));
    // The shared recipe is unchanged (still v1).
    assert_eq!(catalog.versions().resolve(&handle).unwrap().1, v1_id);
}

/// Scenario C — provenance audit / M12 flywheel: a curated model is published as
/// a new version carrying full provenance + a `prior` to the previous corpus
/// version; an auditor traces the lineage and confirms it gated nothing.
#[test]
fn provenance_audit_m12_flywheel_traces_chain() {
    let handle = vpath("curated-summarizer");
    let owner = PartyId::new("ml-platform@acme");
    let catalog = governed_with(
        &handle,
        &owner,
        &owner,
        CatalogActionSet::allow([CatalogAction::Read, CatalogAction::Register]),
    );

    // v1 — the first curated corpus (a Dataset content).
    let v1 = AssetVersion::root(
        handle.clone(),
        VersionedContent::Dataset(DatasetId([0x10; 32])),
        owner.clone(),
        Provenance::from_recipe([0x10; 32]),
    );
    let v1_id = catalog.publish(v1).unwrap().version_id();

    // v2 — a re-curated corpus produced by a known run, derived from v1.
    let prov_v2 = Provenance::from_recipe([0x20; 32])
        .with_run([0x21; 16])
        .with_dataset(DatasetId([0x22; 32]))
        .with_corpus_lineage([
            MoteId::from_bytes([0x23; 32]),
            MoteId::from_bytes([0x24; 32]),
        ])
        .unwrap();
    let v2 = AssetVersion::successor(
        v1_id,
        0,
        handle.clone(),
        VersionedContent::Dataset(DatasetId([0x25; 32])),
        owner.clone(),
        prov_v2,
    );
    let v2_id = catalog.publish(v2).unwrap().version_id();

    // The auditor walks the lineage and reads each version's advisory provenance.
    let lin = catalog.lineage(&owner, &v2_id).unwrap();
    assert_eq!(lin.len(), 2, "v2 → v1");
    let head = &lin[0];
    assert_eq!(head.version_id(), v2_id);
    assert_eq!(head.provenance().generating_run(), Some([0x21; 16]));
    assert_eq!(head.provenance().corpus_lineage().len(), 2);
    assert_eq!(head.provenance().recipe_fingerprint(), &[0x20; 32]);
    // The advisory provenance gated nothing — the Register grant did.
    assert_eq!(catalog.resolve(&owner, &handle).unwrap().unwrap().1, v2_id);
}

// ---- Exit gate (versioning) --------------------------------------------------

/// The M7.2 versioning exit gate, composite: publish → resolve → move-handle →
/// rollback round-trip; governed publish refused without `Register`; lineage is
/// advisory (a forged-provenance publish still succeeds); the schema version is
/// pinned; and the guarantee-path wall holds (asserted by
/// `guarantee_path_does_not_depend_on_catalog`).
#[test]
fn m7_2_versioning_exit_gate() {
    let handle = vpath("recipe");
    let owner = PartyId::new("owner@acme");
    let catalog = governed_with(
        &handle,
        &owner,
        &owner,
        CatalogActionSet::allow([CatalogAction::Read, CatalogAction::Register]),
    );

    // (a) publish → resolve → move handle → rollback.
    let v1 = AssetVersion::root(
        handle.clone(),
        recipe(1),
        owner.clone(),
        Provenance::from_recipe([1u8; 32]),
    );
    assert_eq!(
        v1.schema_version(),
        kx_catalog::CATALOG_VERSION_SCHEMA_VERSION
    );
    let v1_id = catalog.publish(v1).unwrap().version_id();
    let v2 = AssetVersion::successor(
        v1_id,
        0,
        handle.clone(),
        recipe(2),
        owner.clone(),
        Provenance::from_recipe([2u8; 32]),
    );
    let v2_id = catalog.publish(v2).unwrap().version_id();
    assert_eq!(catalog.resolve(&owner, &handle).unwrap().unwrap().1, v2_id);
    // rollback to v1's content (a NEW version).
    let v3 = AssetVersion::successor(
        v2_id,
        1,
        handle.clone(),
        recipe(1),
        owner.clone(),
        Provenance::from_recipe([3u8; 32]),
    );
    let v3_id = catalog.publish(v3).unwrap().version_id();
    assert_eq!(catalog.resolve(&owner, &handle).unwrap().unwrap().1, v3_id);
    assert_eq!(
        catalog.resolve(&owner, &handle).unwrap().unwrap().0,
        recipe(1),
        "rolled back to v1 content"
    );
    assert_eq!(catalog.versions().len(), 3, "all versions retained");

    // (b) without Register, a publish is refused.
    let nobody = PartyId::new("nobody@acme");
    let bad = AssetVersion::root(
        handle,
        recipe(9),
        nobody,
        Provenance::from_recipe([9u8; 32]),
    );
    assert!(matches!(
        catalog.publish(bad).unwrap_err(),
        GovernedError::Unauthorized {
            action: CatalogAction::Register,
            ..
        }
    ));

    // (c) lineage is advisory: it traces the chain but never gated a publish.
    let lin = catalog.lineage(&owner, &v3_id).unwrap();
    assert_eq!(lin.len(), 3, "v3 → v2 → v1");
}
