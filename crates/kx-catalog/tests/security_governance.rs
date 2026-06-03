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
    AssetBinding, AssetPath, AssetRef, CatalogAction, CatalogActionSet, Grant, GrantId,
    GrantLedger, InMemoryGrantLedger, LedgerFact, PartyId, Revocation,
};
use kx_mote::ModelId;
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
