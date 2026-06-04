// SPDX-License-Identifier: Apache-2.0
//! Property tests for the membership ledger + the composed fleet resolution (M7,
//! D112): idempotency, content-id distinctness, fold determinism + order-independence,
//! the **no-widen** safety invariant (a member never exceeds the team), revoke /
//! disband honoring + authority, and the additive-cap model. Mirrors
//! `kx-catalog/tests/proptest_grants.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use kx_catalog::{
    AssetBinding, AssetPath, AssetRef, CatalogAction, CatalogActionSet, Grant, GrantLedger,
    InMemoryGrantLedger, PartyId,
};
use kx_fleet::{
    Admit, Disband, GovernedFleet, InMemoryMembershipLedger, MembershipLedger, Removal, Team,
};
use kx_mote::ModelId;
use kx_warrant::{ModelRoute, ResourceCeiling, Role, WarrantSpec};
use proptest::prelude::*;

// ---- fixtures ---------------------------------------------------------------

fn asset() -> AssetRef {
    AssetRef::Path(AssetPath::new("acme", "runbooks", "restart").unwrap())
}

/// A FIXED positive model route shared by every warrant — `kx_warrant::intersect`
/// rejects a zero model route, and a constant route makes its per-axis `min()` a
/// no-op so the narrowing chain isolates `cpu_milli`.
fn model_route() -> ModelRoute {
    ModelRoute {
        model_id: ModelId("m".into()),
        max_input_tokens: 1_000,
        max_output_tokens: 1_000,
        max_calls: 1_000,
    }
}

/// A warrant whose ONLY varying quantitative axis is `cpu_milli` (so a narrowing chain
/// is exactly a `min()` over `cpu_milli` — qualitative axes stay default-empty, so
/// `intersect` never errors on a widen).
fn warrant_cpu(cpu: u32) -> WarrantSpec {
    WarrantSpec {
        model_route: model_route(),
        resource_ceiling: ResourceCeiling {
            cpu_milli: cpu,
            mem_bytes: 0,
            wall_clock_ms: 0,
            fd_count: 0,
            disk_bytes: 0,
        },
        ..Default::default()
    }
}

fn role_cpu(name: &str, cpu: u32) -> Role {
    Role {
        name: name.into(),
        version: 1,
        spec: warrant_cpu(cpu),
        description: String::new(),
    }
}

fn role(name: &str) -> Role {
    role_cpu(name, 0)
}

// ---- properties -------------------------------------------------------------

proptest! {
    /// Appending the same admit any number of times stores exactly one fact.
    #[test]
    fn admit_append_is_idempotent(reps in 1usize..6) {
        let fleet = InMemoryMembershipLedger::new();
        let team = PartyId::new("t");
        let owner = PartyId::new("o");
        fleet.append_founding(Team::found(team.clone(), owner.clone(), "T")).unwrap();
        let admit = Admit::new(team.clone(), PartyId::new("m"), owner, role("r"),
            CatalogActionSet::allow([CatalogAction::Use]));
        prop_assert!(fleet.append_admit(admit.clone()).unwrap().is_appended());
        for _ in 0..reps {
            prop_assert!(!fleet.append_admit(admit.clone()).unwrap().is_appended());
        }
        // founding + one admit.
        prop_assert_eq!(fleet.len(), 2);
    }

    /// Distinct admits (differing in any field) get distinct content ids.
    #[test]
    fn distinct_admits_have_distinct_ids(a in 0u8..4, b in 0u8..4) {
        prop_assume!(a != b);
        let mk = |i: u8| Admit::new(
            PartyId::new("t"), PartyId::new(format!("m{i}")), PartyId::new("o"),
            role("r"), CatalogActionSet::all()).admit_id();
        prop_assert_ne!(mk(a), mk(b));
    }

    /// The fold is order-independent: appending a member-set in any order yields the
    /// same effective membership.
    #[test]
    fn effective_members_are_order_independent(n in 1usize..8) {
        let team = PartyId::new("t");
        let owner = PartyId::new("o");
        let members: Vec<PartyId> = (0..n).map(|i| PartyId::new(format!("m{i}"))).collect();

        let build = |order: Box<dyn Iterator<Item = usize>>| {
            let fleet = InMemoryMembershipLedger::new();
            fleet.append_founding(Team::found(team.clone(), owner.clone(), "T")).unwrap();
            for i in order {
                fleet.append_admit(Admit::new(team.clone(), members[i].clone(), owner.clone(),
                    role("r"), CatalogActionSet::allow([CatalogAction::Use]))).unwrap();
            }
            fleet.effective_members(&team)
        };
        let forward = build(Box::new(0..n));
        let reverse = build(Box::new((0..n).rev()));
        prop_assert_eq!(forward.len(), n);
        // Effective members are sorted by member id ⇒ identical regardless of order.
        prop_assert!(forward.iter().map(|m| m.member().clone())
            .eq(reverse.iter().map(|m| m.member().clone())));
    }

    /// THE core safety invariant: a member's resolved warrant never exceeds the team
    /// grant. With single-axis (`cpu_milli`) warrants the resolved value is EXACTLY
    /// the chain `min` — and therefore `≤` the team's own effective warrant.
    #[test]
    fn member_warrant_never_exceeds_team(c0 in 0u32..50_000, c1 in 0u32..50_000, c2 in 0u32..50_000) {
        let owner = PartyId::new("asset-owner");
        let team = PartyId::new("team:eng");
        let member = PartyId::new("alice");
        let owner_root = warrant_cpu(c0);

        let grants = InMemoryGrantLedger::new();
        grants.append_binding(AssetBinding::new(asset(), owner.clone())).unwrap();
        grants.append_grant(Grant::root(asset(), owner.clone(), team.clone(),
            CatalogActionSet::allow([CatalogAction::Use]), role_cpu("team", c1))).unwrap();

        let fleet = InMemoryMembershipLedger::new();
        fleet.append_founding(Team::found(team.clone(), owner.clone(), "Eng")).unwrap();
        fleet.append_admit(Admit::new(team.clone(), member.clone(), owner.clone(),
            role_cpu("member", c2), CatalogActionSet::allow([CatalogAction::Use]))).unwrap();

        let team_eff = grants
            .resolve_effective_warrant_for(&team, &asset(), CatalogAction::Use, &owner_root)
            .unwrap().unwrap();
        let gov = GovernedFleet::new(fleet, grants);
        let resolved = gov
            .resolve_member_warrant(&member, &asset(), CatalogAction::Use, &owner_root)
            .unwrap().unwrap();

        prop_assert_eq!(resolved.resource_ceiling.cpu_milli, c0.min(c1).min(c2));
        prop_assert!(resolved.resource_ceiling.cpu_milli <= team_eff.resource_ceiling.cpu_milli);
    }

    /// An authorized removal (by the owner) drops the member; the admit fact survives.
    #[test]
    fn authorized_remove_drops_member(_seed in 0u8..4) {
        let fleet = InMemoryMembershipLedger::new();
        let team = PartyId::new("t");
        let owner = PartyId::new("o");
        let member = PartyId::new("m");
        fleet.append_founding(Team::found(team.clone(), owner.clone(), "T")).unwrap();
        fleet.append_admit(Admit::new(team.clone(), member.clone(), owner.clone(),
            role("r"), CatalogActionSet::allow([CatalogAction::Use]))).unwrap();
        prop_assert!(fleet.is_member(&member, &team));
        fleet.append_remove(Removal::new(team.clone(), member.clone(), owner.clone())).unwrap();
        prop_assert!(!fleet.is_member(&member, &team));
        // The admit fact is retained (append-only) — founding + admit + removal.
        prop_assert_eq!(fleet.len(), 3);
    }

    /// A stranger's removal is recorded-but-inert (the member stays).
    #[test]
    fn stranger_remove_is_inert(_seed in 0u8..4) {
        let fleet = InMemoryMembershipLedger::new();
        let team = PartyId::new("t");
        let owner = PartyId::new("o");
        let member = PartyId::new("m");
        fleet.append_founding(Team::found(team.clone(), owner.clone(), "T")).unwrap();
        fleet.append_admit(Admit::new(team.clone(), member.clone(), owner.clone(),
            role("r"), CatalogActionSet::allow([CatalogAction::Use]))).unwrap();
        fleet.append_remove(Removal::new(team.clone(), member.clone(), PartyId::new("mallory"))).unwrap();
        prop_assert!(fleet.is_member(&member, &team));
    }

    /// An owner disband makes every member inert (cascade).
    #[test]
    fn owner_disband_cascades(n in 1usize..6) {
        let fleet = InMemoryMembershipLedger::new();
        let team = PartyId::new("t");
        let owner = PartyId::new("o");
        fleet.append_founding(Team::found(team.clone(), owner.clone(), "T")).unwrap();
        for i in 0..n {
            fleet.append_admit(Admit::new(team.clone(), PartyId::new(format!("m{i}")),
                owner.clone(), role("r"), CatalogActionSet::all())).unwrap();
        }
        prop_assert_eq!(fleet.effective_members(&team).len(), n);
        fleet.append_disband(Disband::new(team.clone(), owner.clone())).unwrap();
        prop_assert!(fleet.effective_members(&team).is_empty());
    }

    /// An admit by a non-owner without Delegate conveys nothing.
    #[test]
    fn non_admin_admit_is_inert(_seed in 0u8..4) {
        let fleet = InMemoryMembershipLedger::new();
        let team = PartyId::new("t");
        let owner = PartyId::new("o");
        // Dave is a plain member (cap {Use}, no Delegate).
        fleet.append_founding(Team::found(team.clone(), owner.clone(), "T")).unwrap();
        fleet.append_admit(Admit::new(team.clone(), PartyId::new("dave"), owner.clone(),
            role("r"), CatalogActionSet::allow([CatalogAction::Use]))).unwrap();
        // Dave admits Eve — inert (Dave lacks Delegate).
        fleet.append_admit(Admit::new(team.clone(), PartyId::new("eve"), PartyId::new("dave"),
            role("r"), CatalogActionSet::all())).unwrap();
        prop_assert!(!fleet.is_member(&PartyId::new("eve"), &team));
    }

    /// Additive caps: multiple parallel admits of one member union their action caps.
    #[test]
    fn parallel_admits_union_caps(_seed in 0u8..4) {
        let fleet = InMemoryMembershipLedger::new();
        let team = PartyId::new("t");
        let owner = PartyId::new("o");
        let member = PartyId::new("grace");
        fleet.append_founding(Team::found(team.clone(), owner.clone(), "T")).unwrap();
        // Two admits under different roles + disjoint caps.
        fleet.append_admit(Admit::new(team.clone(), member.clone(), owner.clone(),
            role("a"), CatalogActionSet::allow([CatalogAction::Use]))).unwrap();
        fleet.append_admit(Admit::new(team.clone(), member.clone(), owner.clone(),
            role("b"), CatalogActionSet::allow([CatalogAction::Read]))).unwrap();
        let mr = fleet.member_role(&member, &team).unwrap();
        prop_assert!(mr.action_cap().contains(CatalogAction::Use));
        prop_assert!(mr.action_cap().contains(CatalogAction::Read));
    }
}
