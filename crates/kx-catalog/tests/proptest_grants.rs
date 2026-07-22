// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! Property tests for M7.2 grants/RBAC + revocation (D86).
//!
//! Integration-test file: compiled as a separate crate, so it carries its own
//! lint allowances for fixture `unwrap`s and pedantic test idioms.
//!
//! Properties (each over randomized grants/roles/parties):
//! 1. A grant chain's effective runtime warrant NEVER widens past the owner root
//!    (quantitative axes are min-narrowed; a qualitative widen is a typed error).
//! 2. A delegated grant's actions are a SUBSET of the delegator's — laundering
//!    an action the delegator lacked is impossible.
//! 3. The fold is order-independent (append order of facts does not change the
//!    resolution).
//! 4. Revoke-by-new-fact: an authorized revocation drops authority while the
//!    grant fact survives (append-only).
//! 5. Only the grantor or the asset owner can revoke (a stranger's revocation is
//!    recorded-but-inert).
//! 6. Appending a grant is idempotent (N appends ⇒ one stored fact).
//! 7. The effective warrant for an action is ACTION-ALIGNED: it equals the
//!    most-permissive fold among the chains that ACTUALLY convey that action.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use kx_catalog::{
    AssetBinding, AssetPath, AssetRef, CatalogAction, CatalogActionSet, Grant, GrantLedger,
    InMemoryGrantLedger, PartyId, Revocation,
};
use kx_mote::ModelId;
use kx_warrant::{ModelRoute, ResourceCeiling, Role, SecretRef, SecretScope, WarrantSpec};
use proptest::prelude::*;

fn asset() -> AssetRef {
    AssetRef::Path(AssetPath::new("ns", "col", "asset").unwrap())
}

/// A warrant whose only non-default axes are positive quantitative ceilings
/// (qualitative axes are the empty defaults ⇒ ⊆ any parent).
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

fn role_calls(max_calls: u32) -> Role {
    Role {
        name: "r".into(),
        version: 1,
        spec: warrant_calls(max_calls),
        description: String::new(),
    }
}

/// Each catalog action independently present/absent.
fn arb_actions() -> impl Strategy<Value = CatalogActionSet> {
    (any::<bool>(), any::<bool>(), any::<bool>(), any::<bool>()).prop_map(|(r, u, reg, d)| {
        let mut v = Vec::new();
        if r {
            v.push(CatalogAction::Read);
        }
        if u {
            v.push(CatalogAction::Use);
        }
        if reg {
            v.push(CatalogAction::Register);
        }
        if d {
            v.push(CatalogAction::Delegate);
        }
        CatalogActionSet::allow(v)
    })
}

const ACTIONS: [CatalogAction; 4] = [
    CatalogAction::Read,
    CatalogAction::Use,
    CatalogAction::Register,
    CatalogAction::Delegate,
];

proptest! {
    /// (1) quantitative narrowing: the resolved warrant never exceeds owner root.
    #[test]
    fn grant_runtime_warrant_never_widens(parent in 1u32..=1000, child in 1u32..=2000) {
        let ledger = InMemoryGrantLedger::new();
        let a = asset();
        let owner = PartyId::new("owner");
        let mate = PartyId::new("mate");
        let owner_root = warrant_calls(parent);
        ledger.append_binding(AssetBinding::new(a.clone(), owner.clone())).unwrap();
        ledger.append_grant(Grant::root(
            a.clone(), owner, mate.clone(),
            CatalogActionSet::allow([CatalogAction::Use]), role_calls(child),
        )).unwrap();

        let w = ledger
            .resolve_effective_warrant_for(&mate, &a, CatalogAction::Use, &owner_root)
            .unwrap()
            .expect("Use is granted");
        prop_assert!(w.model_route.max_calls <= parent, "never widens past owner root");
        prop_assert_eq!(w.model_route.max_calls, child.min(parent), "min-narrowed");
    }

    /// (1b) a qualitative widen (a secret the owner cannot resolve) is a typed error.
    #[test]
    fn qualitative_widen_is_refused(parent in 1u32..=1000) {
        let ledger = InMemoryGrantLedger::new();
        let a = asset();
        let owner = PartyId::new("owner");
        let mate = PartyId::new("mate");
        let owner_root = warrant_calls(parent); // secret_scope = None (default)
        let mut wide = warrant_calls(parent);
        wide.secret_scope = SecretScope::AllowList([SecretRef("s".into())].into_iter().collect());
        let role = Role { name: "wide".into(), version: 1, spec: wide, description: String::new() };
        ledger.append_binding(AssetBinding::new(a.clone(), owner.clone())).unwrap();
        ledger.append_grant(Grant::root(
            a.clone(), owner, mate.clone(), CatalogActionSet::allow([CatalogAction::Use]), role,
        )).unwrap();

        prop_assert!(ledger
            .resolve_effective_warrant_for(&mate, &a, CatalogAction::Use, &owner_root)
            .is_err());
    }

    /// (2) a delegated grant's effective actions ⊆ the delegator's; no laundering.
    #[test]
    fn delegated_actions_subset_of_delegator(as_a in arb_actions(), as_b in arb_actions()) {
        let ledger = InMemoryGrantLedger::new();
        let a = asset();
        let owner = PartyId::new("owner");
        let lead = PartyId::new("lead");
        let intern = PartyId::new("intern");
        ledger.append_binding(AssetBinding::new(a.clone(), owner.clone())).unwrap();
        let root = Grant::root(a.clone(), owner, lead.clone(), as_a, role_calls(50));
        let root_id = root.grant_id();
        ledger.append_grant(root).unwrap();
        ledger.append_grant(Grant::delegated(
            root_id, a.clone(), lead.clone(), intern.clone(), as_b, role_calls(50),
        )).unwrap();

        let lead_acts = ledger.effective_grants(&lead, &a);
        let intern_acts = ledger.effective_grants(&intern, &a);
        for x in ACTIONS {
            if intern_acts.actions().contains(x) {
                // Anything the intern conveys, the lead conveyed (no laundering).
                prop_assert!(lead_acts.actions().contains(x), "no action laundered via delegation");
            }
        }
    }

    /// (3) the fold is independent of fact append order.
    #[test]
    fn fold_is_order_independent(acts in arb_actions()) {
        let a = asset();
        let owner = PartyId::new("owner");
        let mate = PartyId::new("mate");
        let bind = AssetBinding::new(a.clone(), owner.clone());
        let grant = Grant::root(a.clone(), owner, mate.clone(), acts, role_calls(10));

        let l1 = InMemoryGrantLedger::new();
        l1.append_binding(bind.clone()).unwrap();
        l1.append_grant(grant.clone()).unwrap();

        let l2 = InMemoryGrantLedger::new();
        l2.append_grant(grant).unwrap(); // grant BEFORE the binding
        l2.append_binding(bind).unwrap();

        for x in ACTIONS {
            prop_assert_eq!(
                l1.is_authorized(&mate, &a, x),
                l2.is_authorized(&mate, &a, x),
                "resolution is order-independent"
            );
        }
    }

    /// (4)+(5) revoke-by-new-fact + only grantor/owner can revoke. For a ROOT
    /// grant the grantor IS the owner, so a revocation takes effect iff the
    /// revoker is the owner.
    #[test]
    fn revocation_authority(revoker_pick in 0u8..3) {
        let ledger = InMemoryGrantLedger::new();
        let a = asset();
        let owner = PartyId::new("owner");
        let mate = PartyId::new("mate");
        let stranger = PartyId::new("stranger");
        ledger.append_binding(AssetBinding::new(a.clone(), owner.clone())).unwrap();
        let g = Grant::root(a.clone(), owner.clone(), mate.clone(),
            CatalogActionSet::allow([CatalogAction::Use]), role_calls(10));
        let gid = g.grant_id();
        ledger.append_grant(g).unwrap();
        prop_assert!(ledger.is_authorized(&mate, &a, CatalogAction::Use));

        let facts_before = ledger.list_facts().count();
        let revoker = [owner.clone(), mate.clone(), stranger][revoker_pick as usize].clone();
        let authorized = revoker == owner; // grantor == owner for a root grant
        ledger.append_revocation(Revocation::new(gid, revoker)).unwrap();

        prop_assert_eq!(
            ledger.is_authorized(&mate, &a, CatalogAction::Use),
            !authorized,
            "authority drops iff the revoker is authorized"
        );
        // The grant fact survives regardless (append-only; revoke is a new fact).
        prop_assert!(ledger.list_facts().count() > facts_before, "revoke is a new fact");
        prop_assert!(
            ledger.list_facts().filter(|f| matches!(f, kx_catalog::LedgerFact::Grant(_))).count() == 1,
            "the grant fact is never mutated/removed"
        );
    }

    /// (6) appending a grant is idempotent.
    #[test]
    fn append_grant_is_idempotent(reps in 1usize..6) {
        let ledger = InMemoryGrantLedger::new();
        let a = asset();
        let owner = PartyId::new("owner");
        let mate = PartyId::new("mate");
        ledger.append_binding(AssetBinding::new(a.clone(), owner.clone())).unwrap();
        let g = Grant::root(a.clone(), owner, mate, CatalogActionSet::allow([CatalogAction::Use]), role_calls(10));
        let mut appended = 0;
        for _ in 0..reps {
            if ledger.append_grant(g.clone()).unwrap().is_appended() {
                appended += 1;
            }
        }
        prop_assert_eq!(appended, 1, "only the first append lands");
        prop_assert_eq!(
            ledger.list_facts().filter(|f| matches!(f, kx_catalog::LedgerFact::Grant(_))).count(),
            1
        );
    }

    /// (7) the warrant for an action equals the most-permissive fold among the
    /// chains that ACTUALLY convey it (the multi-grant regression, randomized).
    #[test]
    fn per_grant_warrant_aligned_with_action(
        as1 in arb_actions(), as2 in arb_actions(), c1 in 1u32..=100, c2 in 1u32..=100,
    ) {
        let ledger = InMemoryGrantLedger::new();
        let a = asset();
        let owner = PartyId::new("owner");
        let party = PartyId::new("party");
        let owner_root = warrant_calls(100); // ≥ c1,c2 ⇒ min(ci,100)=ci
        ledger.append_binding(AssetBinding::new(a.clone(), owner.clone())).unwrap();
        ledger.append_grant(Grant::root(a.clone(), owner.clone(), party.clone(), as1.clone(), role_calls(c1))).unwrap();
        ledger.append_grant(Grant::root(a.clone(), owner, party.clone(), as2.clone(), role_calls(c2))).unwrap();

        for x in ACTIONS {
            let resolved = ledger
                .resolve_effective_warrant_for(&party, &a, x, &owner_root)
                .unwrap();
            // The expected max_calls = max over the chains that convey x.
            let mut expected: Option<u32> = None;
            if as1.contains(x) { expected = Some(expected.map_or(c1, |e| e.max(c1))); }
            if as2.contains(x) { expected = Some(expected.map_or(c2, |e| e.max(c2))); }
            match (resolved, expected) {
                (Some(w), Some(e)) => {
                    prop_assert_eq!(w.model_route.max_calls, e, "warrant comes from a conveying chain");
                    prop_assert!(w.model_route.max_calls <= 100, "never widens past owner root");
                }
                (None, None) => {} // action conveyed by neither ⇒ fail-closed None
                (r, e) => prop_assert!(false, "mismatch: resolved={:?} expected_calls={:?}", r.is_some(), e),
            }
        }
    }
}

/// Closed-enum drift guard: adding a `CatalogAction` variant fails to compile
/// here (and `ACTIONS` above would be incomplete), forcing the proptests + the
/// `CatalogAction::all()` set to be revisited.
#[allow(dead_code)]
fn exhaustive_action_match(a: CatalogAction) {
    match a {
        CatalogAction::Read
        | CatalogAction::Use
        | CatalogAction::Register
        | CatalogAction::Delegate => {}
    }
}

#[test]
fn action_vocabulary_is_pinned() {
    assert_eq!(
        CatalogAction::all().len(),
        ACTIONS.len(),
        "closed action vocabulary"
    );
}
