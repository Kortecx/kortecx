// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! Property tests for M7.2 content-versioning + provenance/lineage (D82/D88).
//!
//! Integration-test file: compiled as a separate crate, so it carries its own
//! lint allowances for fixture `unwrap`s and pedantic test idioms.
//!
//! Properties (each over randomized chains/orders/actions):
//! 1. Publishing a version is idempotent (N publishes ⇒ one stored fact).
//! 2. A handle resolves to its max-rank version, and `history` is identical
//!    regardless of the order the versions were published (the handle move + the
//!    fold are pure functions of the fact set, not of append order).
//! 3. Lineage walks the full chain when under the depth cap.
//! 4. `descendants(root)` covers every forward node exactly once.
//! 5. The governed publish succeeds IFF the publisher holds `Register`.
//! 6. A foreign-`prior` graft (a different handle's version) truncates lineage —
//!    forged provenance conveys no further ancestry (fail-closed).
//! 7. A missing-`prior` (phantom) reference truncates lineage (fail-closed).
//! 8. A published version round-trips through `get_version` by its exact id.
//!
//! NOTE (structural unreachability): a `VersionId` is `blake3` over the WHOLE
//! `AssetVersion` INCLUDING `prior`, so (a) two versions with the same id are
//! byte-identical (the `ImmutabilityConflict` branch is a cryptographic tripwire,
//! not a reachable state), and (b) a `prior` cycle is impossible to construct (a
//! version's id depends on its prior's id). The `seen`/immutability guards are
//! defense-in-depth for a corrupt backend; the reachable fail-closed paths
//! (foreign/missing `prior`) are what these properties exercise.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use kx_catalog::{
    AssetBinding, AssetPath, AssetRef, AssetVersion, CatalogAction, CatalogActionSet,
    GovernedCatalog, GovernedError, Grant, GrantLedger, InMemoryGrantLedger, InMemoryVersionLedger,
    PartyId, Provenance, TaskSignatureHash, VersionLedger, VersionLedgerError, VersionedContent,
    MAX_VERSION_CHAIN_DEPTH,
};
use kx_mote::ModelId;
use kx_warrant::{ModelRoute, ResourceCeiling, Role, WarrantSpec};
use proptest::prelude::*;

fn apath(name: &str) -> AssetPath {
    AssetPath::new("acme", "recipes", name).unwrap()
}

fn recipe(byte: u8) -> VersionedContent {
    VersionedContent::Recipe(TaskSignatureHash::from_bytes([byte; 32]))
}

fn prov(byte: u8) -> Provenance {
    Provenance::from_recipe([byte; 32])
}

fn alice() -> PartyId {
    PartyId::new("alice@acme")
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

fn role_calls(max_calls: u32) -> Role {
    Role {
        name: "r".into(),
        version: 1,
        spec: warrant_calls(max_calls),
        description: String::new(),
    }
}

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

/// Build the linear version chain `v0 → v1 → … → v(n-1)` of one handle.
fn build_chain(handle: &AssetPath, n: usize) -> Vec<AssetVersion> {
    let mut chain = Vec::with_capacity(n);
    let v0 = AssetVersion::root(handle.clone(), recipe(0), alice(), prov(0));
    let mut prev_id = v0.version_id();
    let mut prev_rev = v0.revision();
    chain.push(v0);
    for k in 1..n {
        let v = AssetVersion::successor(
            prev_id,
            prev_rev,
            handle.clone(),
            recipe(u8::try_from(k % 250).unwrap()),
            alice(),
            prov(u8::try_from((k * 3) % 250).unwrap()),
        );
        prev_id = v.version_id();
        prev_rev = v.revision();
        chain.push(v);
    }
    chain
}

/// A count of independent roots + a random publish order over their indices
/// (roots have no prior, so any publish order is valid — genuine order-independence).
fn roots_and_order() -> impl Strategy<Value = (usize, Vec<usize>)> {
    (2usize..10).prop_flat_map(|n| (Just(n), Just((0..n).collect::<Vec<usize>>()).prop_shuffle()))
}

proptest! {
    /// (1) publish is idempotent.
    #[test]
    fn publish_is_idempotent(reps in 1usize..6) {
        let ledger = InMemoryVersionLedger::new();
        let v = AssetVersion::root(apath("x"), recipe(1), alice(), prov(1));
        let mut published = 0;
        for _ in 0..reps {
            if ledger.publish(v.clone()).unwrap().is_published() {
                published += 1;
            }
        }
        prop_assert_eq!(published, 1, "only the first publish lands");
        prop_assert_eq!(ledger.len(), 1);
    }

    /// (2a) a chain published in causal order resolves to its latest; history is
    /// the full latest → oldest walk.
    #[test]
    fn chain_resolves_to_latest(n in 2usize..10) {
        let handle = apath("x");
        let chain = build_chain(&handle, n);
        let ledger = InMemoryVersionLedger::new();
        for v in &chain {
            ledger.publish(v.clone()).unwrap();
        }
        let latest = chain[n - 1].version_id();
        prop_assert_eq!(ledger.resolve(&handle).unwrap().1, latest);
        let got: Vec<_> = ledger.history(&handle).iter().map(AssetVersion::version_id).collect();
        let want: Vec<_> = (0..n).rev().map(|k| chain[k].version_id()).collect();
        prop_assert_eq!(got, want);
    }

    /// (2b) independent roots (distinct handles, no prior) publish in ANY order and
    /// each handle resolves to its own root — genuine order-independence across
    /// independent facts (the per-chain order is fixed by causality).
    #[test]
    fn independent_roots_order_independent((n, order) in roots_and_order()) {
        let ledger = InMemoryVersionLedger::new();
        let roots: Vec<AssetVersion> = (0..n)
            .map(|i| AssetVersion::root(apath(&format!("h{i}")), recipe(u8::try_from(i).unwrap()), alice(), prov(1)))
            .collect();
        for &k in &order {
            ledger.publish(roots[k].clone()).unwrap();
        }
        for (i, r) in roots.iter().enumerate() {
            prop_assert_eq!(ledger.resolve(&apath(&format!("h{i}"))).unwrap().1, r.version_id());
        }
    }

    /// (3) lineage walks the full chain when under the depth cap.
    #[test]
    fn lineage_length_equals_chain_depth(n in 1usize..50) {
        let handle = apath("x");
        let chain = build_chain(&handle, n);
        let ledger = InMemoryVersionLedger::new();
        for v in &chain {
            ledger.publish(v.clone()).unwrap();
        }
        let leaf = chain[n - 1].version_id();
        let lin = ledger.lineage(&leaf);
        prop_assert_eq!(lin.len(), n.min(MAX_VERSION_CHAIN_DEPTH));
        prop_assert_eq!(lin.first().unwrap().version_id(), leaf);
    }

    /// (4) descendants(root) covers every forward node exactly once.
    #[test]
    fn descendants_cover_forward_chain_once(n in 1usize..50) {
        let handle = apath("x");
        let chain = build_chain(&handle, n);
        let ledger = InMemoryVersionLedger::new();
        for v in &chain {
            ledger.publish(v.clone()).unwrap();
        }
        let root = chain[0].version_id();
        let desc = ledger.descendants(&root);
        prop_assert_eq!(desc.len(), n - 1, "every non-root node, once");
        // unique
        let mut sorted = desc.clone();
        sorted.sort_unstable_by_key(|v| *v.as_bytes());
        sorted.dedup();
        prop_assert_eq!(sorted.len(), desc.len(), "no duplicates");
    }

    /// (5) governed publish succeeds IFF the publisher holds Register.
    #[test]
    fn governed_publish_iff_register(acts in arb_actions()) {
        let asset = AssetRef::Path(apath("x"));
        let owner = PartyId::new("owner");
        let grants = InMemoryGrantLedger::new();
        grants.append_binding(AssetBinding::new(asset.clone(), owner.clone())).unwrap();
        grants.append_grant(Grant::root(
            asset, owner, alice(), acts.clone(), role_calls(10),
        )).unwrap();

        let catalog = GovernedCatalog::new(grants, InMemoryVersionLedger::new());
        let v = AssetVersion::root(apath("x"), recipe(1), alice(), prov(1));
        let result = catalog.publish(v);

        if acts.contains(CatalogAction::Register) {
            prop_assert!(result.is_ok(), "Register-holder may publish");
        } else {
            prop_assert!(
                matches!(result, Err(GovernedError::Unauthorized { action: CatalogAction::Register, .. })),
                "without Register the publish is refused"
            );
            prop_assert_eq!(catalog.versions().len(), 0, "nothing appended on refusal");
        }
    }

    /// (6) a foreign-`prior` graft is refused at publish (cross-handle chain).
    #[test]
    fn foreign_prior_publish_is_refused(b in 0u8..200) {
        let ledger = InMemoryVersionLedger::new();
        // A real version on a DIFFERENT handle Y.
        let vy = AssetVersion::root(apath("y"), recipe(b), alice(), prov(b));
        let vy_id = ledger.publish(vy).unwrap().version_id();
        // A "successor" on handle X grafting Y's version as its prior.
        let forged = AssetVersion::successor(
            vy_id, 0, apath("x"), recipe(b.wrapping_add(1)), alice(), prov(b),
        );
        let is_invalid = matches!(
            ledger.publish(forged).unwrap_err(),
            VersionLedgerError::InvalidLineage { .. }
        );
        prop_assert!(is_invalid);
        prop_assert_eq!(ledger.len(), 1);
    }

    /// (7) a missing-`prior` (phantom) reference is refused at publish.
    #[test]
    fn missing_prior_publish_is_refused(b in 0u8..200) {
        let ledger = InMemoryVersionLedger::new();
        let phantom = AssetVersion::root(apath("ghost"), recipe(b), alice(), prov(b)).version_id();
        let orphan = AssetVersion::successor(
            phantom, 0, apath("x"), recipe(b.wrapping_add(7)), alice(), prov(b),
        );
        prop_assert!(matches!(
            ledger.publish(orphan).unwrap_err(),
            VersionLedgerError::PriorNotFound(_)
        ));
        prop_assert_eq!(ledger.len(), 0);
    }

    /// (7b) an inflated `prior_revision` (the handle-grief vector) is refused — the
    /// revision must be exactly real_prior.revision + 1, so the rank can't be gamed.
    #[test]
    fn inflated_prior_revision_publish_is_refused(inflate in 1u32..u32::MAX) {
        let ledger = InMemoryVersionLedger::new();
        let v1 = AssetVersion::root(apath("x"), recipe(1), alice(), prov(1));
        let v1_id = ledger.publish(v1).unwrap().version_id(); // real revision 0
        // Lie that prior_revision = inflate (>0) ⇒ declared revision = inflate+1 ≠ 1.
        let forged = AssetVersion::successor(v1_id, inflate, apath("x"), recipe(2), alice(), prov(2));
        let is_invalid = matches!(
            ledger.publish(forged).unwrap_err(),
            VersionLedgerError::InvalidLineage { .. }
        );
        prop_assert!(is_invalid);
        // The handle stays at the legitimate v1.
        prop_assert_eq!(ledger.resolve(&apath("x")).unwrap().1, v1_id);
    }

    /// (8) a published version round-trips through get_version by its exact id.
    #[test]
    fn get_version_roundtrips(n in 1usize..20) {
        let handle = apath("x");
        let chain = build_chain(&handle, n);
        let ledger = InMemoryVersionLedger::new();
        for v in &chain {
            ledger.publish(v.clone()).unwrap();
        }
        for v in &chain {
            let got = ledger.get_version(&v.version_id()).expect("present");
            prop_assert_eq!(&got, v, "exact-id pin returns the same version");
        }
    }
}
