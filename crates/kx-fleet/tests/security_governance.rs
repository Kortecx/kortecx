// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! Integration + security + exit-gate tests for fleet/team governance (M7, D112).
//!
//! Drives the real-life enterprise use cases end to end — an org admin founds a team,
//! grants it a recipe, admits members under narrowed roles, a member invokes the
//! recipe under their team's grant narrowed by their role; an SRE team nested inside
//! a platform fleet; removal / disband cascades — and proves the security invariants
//! (no escalation past the team, action-aligned per-team resolution, admit-authority,
//! revoke-by-new-fact, fail-closed default, bounded cycle/depth). Also asserts the
//! compiler-enforced wall keeping fleet governance OFF the guarantee path (no
//! guarantee-path crate, and not even kx-catalog, may import `kx-fleet`).
//!
//! `Kind 4 (chaos)` scoping (honest, mirrors kx-catalog): an in-memory ledger has no
//! process kill/replay — durable crash-recovery is a future persistent backend (D94).
//! The analogue here is idempotent-replay-of-appends + fail-closed folds.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::collections::BTreeSet;

use kx_catalog::{
    AssetBinding, AssetPath, AssetRef, CatalogAction, CatalogActionSet, Grant, GrantLedger,
    InMemoryGrantLedger, PartyId,
};
use kx_fleet::{
    Admit, Disband, GovernedFleet, GovernedFleetError, InMemoryMembershipLedger, MembershipFact,
    MembershipLedger, MembershipLedgerError, MembershipOutcome, Removal, Team,
    MAX_TEAM_MEMBERS_WALK,
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

/// Bind `asset` to `owner` and grant `team` the `actions` under `role`.
fn grant_team(
    grants: &InMemoryGrantLedger,
    asset: &AssetRef,
    owner: &PartyId,
    team: &PartyId,
    actions: CatalogActionSet,
    role: Role,
) {
    // Idempotent binding (one owner per asset).
    let _ = grants.append_binding(AssetBinding::new(asset.clone(), owner.clone()));
    grants
        .append_grant(Grant::root(
            asset.clone(),
            owner.clone(),
            team.clone(),
            actions,
            role,
        ))
        .unwrap();
}

// ---- Scenario 1: the happy path ---------------------------------------------

#[test]
fn org_admin_grants_team_then_member_uses_under_narrowed_role() {
    let admin = PartyId::new("admin@acme");
    let sre = PartyId::new("team:sre@acme");
    let alice = PartyId::new("alice@acme");
    let asset = path("acme", "runbooks", "restart");
    let owner_root = warrant_calls(100);

    let grants = InMemoryGrantLedger::new();
    grant_team(
        &grants,
        &asset,
        &admin,
        &sre,
        CatalogActionSet::allow([CatalogAction::Use]),
        role("team", 50),
    );

    let fleet = InMemoryMembershipLedger::new();
    fleet
        .append_founding(Team::found(sre.clone(), admin.clone(), "SRE"))
        .unwrap();
    fleet
        .append_admit(Admit::new(
            sre.clone(),
            alice.clone(),
            admin.clone(),
            role("oncall", 20),
            CatalogActionSet::allow([CatalogAction::Use]),
        ))
        .unwrap();

    let team_eff = grants
        .resolve_effective_warrant_for(&sre, &asset, CatalogAction::Use, &owner_root)
        .unwrap()
        .unwrap();
    let gov = GovernedFleet::new(fleet, grants);

    assert!(gov.is_member_authorized(&alice, &asset, CatalogAction::Use));
    let w = gov
        .resolve_member_warrant(&alice, &asset, CatalogAction::Use, &owner_root)
        .unwrap()
        .unwrap();
    // min(owner 100, team 50, member 20) = 20, and ≤ the team's own 50 (no escalation).
    assert_eq!(w.model_route.max_calls, 20);
    assert!(w.model_route.max_calls <= team_eff.model_route.max_calls);
}

// ---- Scenario 2: removal is immediate ---------------------------------------

#[test]
fn removed_member_loses_access_immediately() {
    let admin = PartyId::new("admin");
    let sre = PartyId::new("team:sre");
    let alice = PartyId::new("alice");
    let asset = path("acme", "runbooks", "restart");
    let owner_root = warrant_calls(100);

    let grants = InMemoryGrantLedger::new();
    grant_team(
        &grants,
        &asset,
        &admin,
        &sre,
        CatalogActionSet::allow([CatalogAction::Use]),
        role("team", 50),
    );
    let fleet = InMemoryMembershipLedger::new();
    fleet
        .append_founding(Team::found(sre.clone(), admin.clone(), "SRE"))
        .unwrap();
    fleet
        .append_admit(Admit::new(
            sre.clone(),
            alice.clone(),
            admin.clone(),
            role("oncall", 20),
            CatalogActionSet::allow([CatalogAction::Use]),
        ))
        .unwrap();
    let gov = GovernedFleet::new(fleet, grants);
    assert!(gov.is_member_authorized(&alice, &asset, CatalogAction::Use));

    // The owner removes Alice — a NEW fact; the live fold reflects it at once.
    gov.members()
        .append_remove(Removal::new(sre.clone(), alice.clone(), admin.clone()))
        .unwrap();
    assert!(!gov.is_member_authorized(&alice, &asset, CatalogAction::Use));
    assert_eq!(
        gov.resolve_member_warrant(&alice, &asset, CatalogAction::Use, &owner_root)
            .unwrap(),
        None
    );
}

// ---- Scenario 3: no escalation past the team --------------------------------

#[test]
fn member_role_cannot_widen_the_team_grant() {
    let admin = PartyId::new("admin");
    let sre = PartyId::new("team:sre");
    let bob = PartyId::new("bob");
    let asset = path("acme", "runbooks", "restart");
    let owner_root = warrant_calls(10); // secret_scope = None

    let grants = InMemoryGrantLedger::new();
    grant_team(
        &grants,
        &asset,
        &admin,
        &sre,
        CatalogActionSet::allow([CatalogAction::Use]),
        role("team", 10), // secret_scope = None
    );

    // Bob's membership role proposes a secret the team's warrant cannot resolve.
    let mut wide = warrant_calls(10);
    wide.secret_scope =
        SecretScope::AllowList([SecretRef("db-password".into())].into_iter().collect());
    let wide_role = Role {
        name: "wide".into(),
        version: 1,
        spec: wide,
        description: String::new(),
    };

    let fleet = InMemoryMembershipLedger::new();
    fleet
        .append_founding(Team::found(sre.clone(), admin.clone(), "SRE"))
        .unwrap();
    fleet
        .append_admit(Admit::new(
            sre.clone(),
            bob.clone(),
            admin.clone(),
            wide_role,
            CatalogActionSet::allow([CatalogAction::Use]),
        ))
        .unwrap();
    let gov = GovernedFleet::new(fleet, grants);

    // The widen surfaces loudly as a typed error — never a silently-wider warrant.
    let err = gov
        .resolve_member_warrant(&bob, &asset, CatalogAction::Use, &owner_root)
        .unwrap_err();
    assert!(matches!(err, GovernedFleetError::Narrowing(_)));
}

// ---- Scenario 4: action-aligned, no confused-deputy union -------------------

#[test]
fn multi_team_membership_is_action_aligned() {
    let admin = PartyId::new("admin");
    let sre = PartyId::new("team:sre");
    let dev = PartyId::new("team:dev");
    let carol = PartyId::new("carol");
    let shared = path("acme", "runbooks", "restart");
    let dev_only = path("acme", "data", "etl");

    let grants = InMemoryGrantLedger::new();
    grant_team(
        &grants,
        &shared,
        &admin,
        &sre,
        CatalogActionSet::allow([CatalogAction::Use]),
        role("sre", 50),
    );
    grant_team(
        &grants,
        &shared,
        &admin,
        &dev,
        CatalogActionSet::allow([CatalogAction::Read]),
        role("dev", 50),
    );
    grant_team(
        &grants,
        &dev_only,
        &admin,
        &dev,
        CatalogActionSet::allow([CatalogAction::Read]),
        role("dev", 50),
    );

    let fleet = InMemoryMembershipLedger::new();
    for (team, name) in [(&sre, "SRE"), (&dev, "Dev")] {
        fleet
            .append_founding(Team::found(team.clone(), admin.clone(), name))
            .unwrap();
    }
    fleet
        .append_admit(Admit::new(
            sre.clone(),
            carol.clone(),
            admin.clone(),
            role("c-sre", 50),
            CatalogActionSet::allow([CatalogAction::Use]),
        ))
        .unwrap();
    fleet
        .append_admit(Admit::new(
            dev.clone(),
            carol.clone(),
            admin.clone(),
            role("c-dev", 50),
            CatalogActionSet::allow([CatalogAction::Read]),
        ))
        .unwrap();
    let gov = GovernedFleet::new(fleet, grants);

    // Use comes ONLY via SRE; Read ONLY via Dev.
    assert!(gov.is_member_authorized(&carol, &shared, CatalogAction::Use));
    assert!(gov.is_member_authorized(&carol, &shared, CatalogAction::Read));
    // No confused-deputy synthesis: on dev-only (Dev holds only Read), Carol cannot Use.
    assert!(gov.is_member_authorized(&carol, &dev_only, CatalogAction::Read));
    assert!(!gov.is_member_authorized(&carol, &dev_only, CatalogAction::Use));
}

// ---- Scenario 5: admit-authority is a fold property -------------------------

#[test]
fn non_delegate_member_cannot_admit() {
    let admin = PartyId::new("admin");
    let sre = PartyId::new("team:sre");
    let dave = PartyId::new("dave");
    let eve = PartyId::new("eve");
    let asset = path("acme", "runbooks", "restart");

    let grants = InMemoryGrantLedger::new();
    grant_team(
        &grants,
        &asset,
        &admin,
        &sre,
        CatalogActionSet::allow([CatalogAction::Use]),
        role("team", 50),
    );
    let fleet = InMemoryMembershipLedger::new();
    fleet
        .append_founding(Team::found(sre.clone(), admin.clone(), "SRE"))
        .unwrap();
    // Dave: plain member, cap {Use}, NO Delegate.
    fleet
        .append_admit(Admit::new(
            sre.clone(),
            dave.clone(),
            admin.clone(),
            role("dave", 50),
            CatalogActionSet::allow([CatalogAction::Use]),
        ))
        .unwrap();
    // Dave tries to admit Eve — inert.
    fleet
        .append_admit(Admit::new(
            sre.clone(),
            eve.clone(),
            dave.clone(),
            role("eve", 50),
            CatalogActionSet::all(),
        ))
        .unwrap();
    let gov = GovernedFleet::new(fleet, grants);

    assert!(gov.members().is_member(&dave, &sre));
    assert!(!gov.members().is_member(&eve, &sre));
    assert!(!gov.is_member_authorized(&eve, &asset, CatalogAction::Use));

    // A Delegate-holding member CAN admit (the positive control).
    let frank = PartyId::new("frank");
    let lead = PartyId::new("lead");
    gov.members()
        .append_admit(Admit::new(
            sre.clone(),
            lead.clone(),
            admin.clone(),
            role("lead", 50),
            CatalogActionSet::allow([CatalogAction::Use, CatalogAction::Delegate]),
        ))
        .unwrap();
    gov.members()
        .append_admit(Admit::new(
            sre.clone(),
            frank.clone(),
            lead.clone(),
            role("frank", 50),
            CatalogActionSet::allow([CatalogAction::Use]),
        ))
        .unwrap();
    assert!(gov.members().is_member(&frank, &sre));
    assert!(gov.is_member_authorized(&frank, &asset, CatalogAction::Use));
}

// ---- Scenario 6: nested fleet-of-teams + cycle/depth fail-closed ------------

#[test]
fn member_resolves_through_a_nested_fleet() {
    let admin = PartyId::new("admin");
    let org = PartyId::new("fleet:platform");
    let sre = PartyId::new("team:sre");
    let frank = PartyId::new("frank");
    let asset = path("acme", "fleet", "deploy");
    let owner_root = warrant_calls(100);

    let grants = InMemoryGrantLedger::new();
    // The FLEET holds the grant (not the team directly).
    grant_team(
        &grants,
        &asset,
        &admin,
        &org,
        CatalogActionSet::allow([CatalogAction::Use]),
        role("fleet", 80),
    );

    let fleet = InMemoryMembershipLedger::new();
    fleet
        .append_founding(Team::found(org.clone(), admin.clone(), "Platform"))
        .unwrap();
    fleet
        .append_founding(Team::found(sre.clone(), admin.clone(), "SRE"))
        .unwrap();
    // team:sre is a MEMBER of fleet:platform (admitted by the fleet owner).
    fleet
        .append_admit(Admit::new(
            org.clone(),
            sre.clone(),
            admin.clone(),
            role("sre-in-org", 40),
            CatalogActionSet::allow([CatalogAction::Use]),
        ))
        .unwrap();
    // Frank is a member of team:sre.
    fleet
        .append_admit(Admit::new(
            sre.clone(),
            frank.clone(),
            admin.clone(),
            role("frank", 20),
            CatalogActionSet::allow([CatalogAction::Use]),
        ))
        .unwrap();
    let gov = GovernedFleet::new(fleet, grants);

    // Frank reaches the asset THROUGH the fleet; the warrant narrows at BOTH hops.
    let w = gov
        .resolve_member_warrant(&frank, &asset, CatalogAction::Use, &owner_root)
        .unwrap()
        .unwrap();
    assert_eq!(w.model_route.max_calls, 20); // min(100, 80, 40, 20)
    assert!(gov.is_member_authorized(&frank, &asset, CatalogAction::Use));
}

#[test]
fn membership_cycle_is_bounded_and_fail_closed() {
    let admin = PartyId::new("admin");
    let t1 = PartyId::new("team:1");
    let t2 = PartyId::new("team:2");
    let m = PartyId::new("m");
    let asset = path("acme", "x", "y");
    let owner_root = warrant_calls(100);

    let grants = InMemoryGrantLedger::new(); // NO team holds a grant on `asset`.
    let fleet = InMemoryMembershipLedger::new();
    fleet
        .append_founding(Team::found(t1.clone(), admin.clone(), "T1"))
        .unwrap();
    fleet
        .append_founding(Team::found(t2.clone(), admin.clone(), "T2"))
        .unwrap();
    // A membership cycle: t1 ∈ t2 and t2 ∈ t1 (both admitted by their owner).
    fleet
        .append_admit(Admit::new(
            t2.clone(),
            t1.clone(),
            admin.clone(),
            role("a", 50),
            CatalogActionSet::all(),
        ))
        .unwrap();
    fleet
        .append_admit(Admit::new(
            t1.clone(),
            t2.clone(),
            admin.clone(),
            role("b", 50),
            CatalogActionSet::all(),
        ))
        .unwrap();
    fleet
        .append_admit(Admit::new(
            t1.clone(),
            m.clone(),
            admin.clone(),
            role("m", 50),
            CatalogActionSet::all(),
        ))
        .unwrap();
    let gov = GovernedFleet::new(fleet, grants);

    // Terminates (cycle-guarded) and fails closed (no grant anywhere).
    assert!(!gov.is_member_authorized(&m, &asset, CatalogAction::Use));
    assert_eq!(
        gov.resolve_member_warrant(&m, &asset, CatalogAction::Use, &owner_root)
            .unwrap(),
        None
    );
}

#[test]
fn over_max_nesting_depth_fails_closed() {
    let admin = PartyId::new("admin");
    let asset = path("acme", "deep", "z");
    let owner_root = warrant_calls(100);
    let depth = MAX_TEAM_MEMBERS_WALK + 6; // beyond the bound

    let grants = InMemoryGrantLedger::new();
    let fleet = InMemoryMembershipLedger::new();
    // Chain: t0 ∈ t1 ∈ … ∈ t{depth}. The TOP team holds the only grant.
    let team = |i: usize| PartyId::new(format!("t{i}"));
    for i in 0..=depth {
        fleet
            .append_founding(Team::found(team(i), admin.clone(), "T"))
            .unwrap();
    }
    for i in 0..depth {
        // t{i} is a member of t{i+1}.
        fleet
            .append_admit(Admit::new(
                team(i + 1),
                team(i),
                admin.clone(),
                role("r", 50),
                CatalogActionSet::all(),
            ))
            .unwrap();
    }
    grant_team(
        &grants,
        &asset,
        &admin,
        &team(depth),
        CatalogActionSet::allow([CatalogAction::Use]),
        role("top", 50),
    );
    let member = PartyId::new("leaf-user");
    fleet
        .append_admit(Admit::new(
            team(0),
            member.clone(),
            admin.clone(),
            role("u", 50),
            CatalogActionSet::all(),
        ))
        .unwrap();
    let gov = GovernedFleet::new(fleet, grants);

    // The top grant is beyond MAX_TEAM_MEMBERS_WALK hops ⇒ unreachable ⇒ fail-closed.
    assert!(!gov.is_member_authorized(&member, &asset, CatalogAction::Use));
    assert_eq!(
        gov.resolve_member_warrant(&member, &asset, CatalogAction::Use, &owner_root)
            .unwrap(),
        None
    );
}

// ---- Scenario 7: fail-closed default ----------------------------------------

#[test]
fn unknown_member_or_team_resolves_to_nothing() {
    let admin = PartyId::new("admin");
    let sre = PartyId::new("team:sre");
    let asset = path("acme", "runbooks", "restart");
    let owner_root = warrant_calls(100);

    let grants = InMemoryGrantLedger::new();
    grant_team(
        &grants,
        &asset,
        &admin,
        &sre,
        CatalogActionSet::allow([CatalogAction::Use]),
        role("team", 50),
    );
    let fleet = InMemoryMembershipLedger::new();
    fleet
        .append_founding(Team::found(sre.clone(), admin.clone(), "SRE"))
        .unwrap();
    let gov = GovernedFleet::new(fleet, grants);

    assert!(!gov.is_member_authorized(&PartyId::new("nobody"), &asset, CatalogAction::Use));
    assert_eq!(
        gov.resolve_member_warrant(
            &PartyId::new("nobody"),
            &asset,
            CatalogAction::Use,
            &owner_root
        )
        .unwrap(),
        None
    );
    // `require_*` surfaces the typed Unauthorized.
    let err = gov
        .require_member_warrant(
            &PartyId::new("nobody"),
            &asset,
            CatalogAction::Use,
            &owner_root,
        )
        .unwrap_err();
    assert!(matches!(err, GovernedFleetError::Unauthorized { .. }));
}

// ---- Scenario 8: additive caps across parallel admits -----------------------

#[test]
fn parallel_admits_are_additive() {
    let admin = PartyId::new("admin");
    let sre = PartyId::new("team:sre");
    let grace = PartyId::new("grace");
    let asset = path("acme", "runbooks", "restart");

    let grants = InMemoryGrantLedger::new();
    grant_team(
        &grants,
        &asset,
        &admin,
        &sre,
        CatalogActionSet::allow([CatalogAction::Use, CatalogAction::Read]),
        role("team", 50),
    );
    let fleet = InMemoryMembershipLedger::new();
    fleet
        .append_founding(Team::found(sre.clone(), admin.clone(), "SRE"))
        .unwrap();
    // Two admits with disjoint caps.
    fleet
        .append_admit(Admit::new(
            sre.clone(),
            grace.clone(),
            admin.clone(),
            role("a", 50),
            CatalogActionSet::allow([CatalogAction::Use]),
        ))
        .unwrap();
    fleet
        .append_admit(Admit::new(
            sre.clone(),
            grace.clone(),
            admin.clone(),
            role("b", 50),
            CatalogActionSet::allow([CatalogAction::Read]),
        ))
        .unwrap();
    let gov = GovernedFleet::new(fleet, grants);

    // Grace effectively holds the union {Use, Read}.
    assert!(gov.is_member_authorized(&grace, &asset, CatalogAction::Use));
    assert!(gov.is_member_authorized(&grace, &asset, CatalogAction::Read));
    let mr = gov.members().member_role(&grace, &sre).unwrap();
    assert!(mr.action_cap().contains(CatalogAction::Use));
    assert!(mr.action_cap().contains(CatalogAction::Read));
}

// ---- ORACLE: the fold equals a brute-force recompute ------------------------

#[test]
fn effective_members_equals_brute_force_oracle() {
    let admin = PartyId::new("admin");
    let team = PartyId::new("team:t");
    let members: Vec<PartyId> = (0..8).map(|i| PartyId::new(format!("m{i}"))).collect();

    let fleet = InMemoryMembershipLedger::new();
    fleet
        .append_founding(Team::found(team.clone(), admin.clone(), "T"))
        .unwrap();
    // Owner-issued admits for everyone, then the owner removes a few.
    for m in &members {
        fleet
            .append_admit(Admit::new(
                team.clone(),
                m.clone(),
                admin.clone(),
                role("r", 50),
                CatalogActionSet::all(),
            ))
            .unwrap();
    }
    let removed: BTreeSet<PartyId> = [members[1].clone(), members[4].clone()]
        .into_iter()
        .collect();
    for m in &removed {
        fleet
            .append_remove(Removal::new(team.clone(), m.clone(), admin.clone()))
            .unwrap();
    }

    // Brute force from the fact log: owner-admitted AND not owner-removed.
    let mut admitted: BTreeSet<PartyId> = BTreeSet::new();
    let mut owner_removed: BTreeSet<PartyId> = BTreeSet::new();
    for fact in fleet.list_facts() {
        match fact {
            MembershipFact::Admit(a) if a.admitter() == &admin && a.team() == &team => {
                admitted.insert(a.member().clone());
            }
            MembershipFact::Remove(r) if r.remover() == &admin && r.team() == &team => {
                owner_removed.insert(r.member().clone());
            }
            _ => {}
        }
    }
    let expected: BTreeSet<PartyId> = admitted.difference(&owner_removed).cloned().collect();

    let got: BTreeSet<PartyId> = fleet
        .effective_members(&team)
        .into_iter()
        .map(|mr| mr.member().clone())
        .collect();
    assert_eq!(got, expected);
    assert_eq!(got.len(), members.len() - removed.len());
}

// ---- The SN-8 wall ----------------------------------------------------------

/// The structural wall: NO guarantee-path crate — and not even `kx-catalog` — may
/// depend on `kx-fleet`, so the compiler can never wire fleet governance onto the
/// identity / commit / selection path (SN-8 / D70). The dependency direction is
/// one-way (`kx-fleet → kx-catalog`). Read the manifests directly — a future
/// `kx-fleet` edge into any of these is a compile-independent regression this catches.
#[test]
fn guarantee_path_does_not_depend_on_fleet() {
    let crates = [
        "kx-scheduler",
        "kx-executor",
        "kx-projection",
        "kx-inference",
        "kx-mote",
        "kx-journal",
        // The one-way edge: the catalog must not depend back on the fleet.
        "kx-catalog",
    ];
    for c in crates {
        let manifest = format!("{}/../{c}/Cargo.toml", env!("CARGO_MANIFEST_DIR"));
        let toml =
            std::fs::read_to_string(&manifest).unwrap_or_else(|e| panic!("read {manifest}: {e}"));
        assert!(
            !toml.contains("kx-fleet"),
            "{c} must NOT depend on kx-fleet (the SN-8 governance wall)"
        );
    }
}

// ---- The D112 exit gate (composite) -----------------------------------------

#[test]
fn d112_exit_gate() {
    // A composite proof of the milestone: a team grant is shared, a member uses it
    // narrowed by their role (no escalation), removal is immediate, a nested fleet
    // resolves, and an unknown principal gets nothing — all on the off-trust-path,
    // off-journal separate truth.
    let admin = PartyId::new("admin");
    let org = PartyId::new("fleet:platform");
    let sre = PartyId::new("team:sre");
    let alice = PartyId::new("alice");
    let asset = path("acme", "runbooks", "restart");
    let owner_root = warrant_calls(100);

    let grants = InMemoryGrantLedger::new();
    grant_team(
        &grants,
        &asset,
        &admin,
        &org,
        CatalogActionSet::allow([CatalogAction::Use]),
        role("fleet", 60),
    );
    let fleet = InMemoryMembershipLedger::new();
    fleet
        .append_founding(Team::found(org.clone(), admin.clone(), "Platform"))
        .unwrap();
    fleet
        .append_founding(Team::found(sre.clone(), admin.clone(), "SRE"))
        .unwrap();
    fleet
        .append_admit(Admit::new(
            org.clone(),
            sre.clone(),
            admin.clone(),
            role("sre-in-org", 40),
            CatalogActionSet::allow([CatalogAction::Use]),
        ))
        .unwrap();
    fleet
        .append_admit(Admit::new(
            sre.clone(),
            alice.clone(),
            admin.clone(),
            role("alice", 25),
            CatalogActionSet::allow([CatalogAction::Use]),
        ))
        .unwrap();
    let gov = GovernedFleet::new(fleet, grants);

    // (1) nested resolution, narrowed at every hop (no escalation).
    let w = gov
        .resolve_member_warrant(&alice, &asset, CatalogAction::Use, &owner_root)
        .unwrap()
        .unwrap();
    assert_eq!(w.model_route.max_calls, 25); // min(100, 60, 40, 25)

    // (2) the disband cascade is immediate (owner disbands the whole SRE team).
    gov.members()
        .append_disband(Disband::new(sre.clone(), admin.clone()))
        .unwrap();
    assert!(!gov.is_member_authorized(&alice, &asset, CatalogAction::Use));

    // (3) the in-memory ledger is shareable (the Arc path a multi-threaded gateway uses).
    let shared = std::sync::Arc::new(InMemoryMembershipLedger::new());
    let _: &dyn MembershipLedger = &shared;
}

// ---- Lifecycle edge cases (post-review hardening) ---------------------------

/// Re-admission after removal RESTORES access — the time-ordered append-only model:
/// a removal cancels only the admits that precede it; a fresh re-admit survives.
#[test]
fn re_admission_after_removal_restores_access() {
    let admin = PartyId::new("admin");
    let sre = PartyId::new("team:sre");
    let alice = PartyId::new("alice");
    let asset = path("acme", "runbooks", "restart");

    let grants = InMemoryGrantLedger::new();
    grant_team(
        &grants,
        &asset,
        &admin,
        &sre,
        CatalogActionSet::allow([CatalogAction::Use]),
        role("team", 50),
    );
    let fleet = InMemoryMembershipLedger::new();
    fleet
        .append_founding(Team::found(sre.clone(), admin.clone(), "SRE"))
        .unwrap();
    let admit = |cap_role: &str| {
        Admit::new(
            sre.clone(),
            alice.clone(),
            admin.clone(),
            role(cap_role, 20),
            CatalogActionSet::allow([CatalogAction::Use]),
        )
    };
    fleet.append_admit(admit("v1")).unwrap();
    let gov = GovernedFleet::new(fleet, grants);
    assert!(gov.is_member_authorized(&alice, &asset, CatalogAction::Use));

    // Remove → access gone.
    gov.members()
        .append_remove(Removal::new(sre.clone(), alice.clone(), admin.clone()))
        .unwrap();
    assert!(!gov.is_member_authorized(&alice, &asset, CatalogAction::Use));

    // Re-admit (a NEW admit, appended AFTER the removal) → access restored.
    gov.members().append_admit(admit("v2")).unwrap();
    assert!(gov.members().is_member(&alice, &sre));
    assert!(gov.is_member_authorized(&alice, &asset, CatalogAction::Use));
}

/// Founding is genesis: a re-founding by the SAME owner is idempotent (no second
/// founding fact, first display name wins); a DIFFERENT owner is an OwnerConflict.
#[test]
fn re_founding_is_idempotent_per_owner() {
    let fleet = InMemoryMembershipLedger::new();
    let team = PartyId::new("team:x");
    let admin = PartyId::new("admin");

    let first = fleet
        .append_founding(Team::found(team.clone(), admin.clone(), "Name1"))
        .unwrap();
    assert!(first.is_appended());

    // Same owner, DIFFERENT display name → idempotent (no second genesis fact).
    let again = fleet
        .append_founding(Team::found(team.clone(), admin.clone(), "Name2"))
        .unwrap();
    assert!(!again.is_appended());
    assert_eq!(again.membership_id(), first.membership_id());
    assert_eq!(
        fleet.len(),
        1,
        "exactly one founding fact per team principal"
    );
    assert_eq!(fleet.owner_of_team(&team), Some(admin.clone()));

    // Different owner → OwnerConflict.
    let err = fleet
        .append_founding(Team::found(team, PartyId::new("usurper"), "Name3"))
        .unwrap_err();
    assert!(matches!(err, MembershipLedgerError::OwnerConflict(_)));
    assert_eq!(fleet.len(), 1);
}

/// Revoking a delegate cascades: members the delegate admitted go inert once the
/// delegate's own (authority-bearing) membership is removed.
#[test]
fn revoking_a_delegate_cascades_to_their_admits() {
    let admin = PartyId::new("admin");
    let sre = PartyId::new("team:sre");
    let lead = PartyId::new("lead");
    let carol = PartyId::new("carol");
    let asset = path("acme", "runbooks", "restart");

    let grants = InMemoryGrantLedger::new();
    grant_team(
        &grants,
        &asset,
        &admin,
        &sre,
        CatalogActionSet::allow([CatalogAction::Use]),
        role("team", 50),
    );
    let fleet = InMemoryMembershipLedger::new();
    fleet
        .append_founding(Team::found(sre.clone(), admin.clone(), "SRE"))
        .unwrap();
    // Owner admits lead WITH Delegate; lead admits carol.
    fleet
        .append_admit(Admit::new(
            sre.clone(),
            lead.clone(),
            admin.clone(),
            role("lead", 50),
            CatalogActionSet::allow([CatalogAction::Use, CatalogAction::Delegate]),
        ))
        .unwrap();
    fleet
        .append_admit(Admit::new(
            sre.clone(),
            carol.clone(),
            lead.clone(),
            role("carol", 50),
            CatalogActionSet::allow([CatalogAction::Use]),
        ))
        .unwrap();
    let gov = GovernedFleet::new(fleet, grants);
    assert!(gov.members().is_member(&carol, &sre));
    assert!(gov.is_member_authorized(&carol, &asset, CatalogAction::Use));

    // The owner removes the lead → carol's admit (by the now-inert lead) cascades off.
    gov.members()
        .append_remove(Removal::new(sre.clone(), lead.clone(), admin.clone()))
        .unwrap();
    assert!(!gov.members().is_member(&lead, &sre));
    assert!(
        !gov.members().is_member(&carol, &sre),
        "carol's admit by the de-authorized lead is now inert (cascade)"
    );
    assert!(!gov.is_member_authorized(&carol, &asset, CatalogAction::Use));
}

/// A role widen ANYWHERE in a NESTED chain surfaces as a typed NarrowingError —
/// never a silently-wider warrant through the fleet.
#[test]
fn nested_chain_widen_is_refused() {
    let admin = PartyId::new("admin");
    let org = PartyId::new("fleet:platform");
    let sre = PartyId::new("team:sre");
    let frank = PartyId::new("frank");
    let asset = path("acme", "fleet", "deploy");
    let owner_root = warrant_calls(10); // secret_scope = None

    let grants = InMemoryGrantLedger::new();
    grant_team(
        &grants,
        &asset,
        &admin,
        &org,
        CatalogActionSet::allow([CatalogAction::Use]),
        role("fleet", 10),
    );

    // The INNER hop (sre-in-org) proposes a secret the fleet's warrant cannot resolve.
    let mut wide = warrant_calls(10);
    wide.secret_scope =
        SecretScope::AllowList([SecretRef("prod-key".into())].into_iter().collect());
    let wide_role = Role {
        name: "wide".into(),
        version: 1,
        spec: wide,
        description: String::new(),
    };

    let fleet = InMemoryMembershipLedger::new();
    fleet
        .append_founding(Team::found(org.clone(), admin.clone(), "Platform"))
        .unwrap();
    fleet
        .append_founding(Team::found(sre.clone(), admin.clone(), "SRE"))
        .unwrap();
    fleet
        .append_admit(Admit::new(
            org.clone(),
            sre.clone(),
            admin.clone(),
            wide_role,
            CatalogActionSet::allow([CatalogAction::Use]),
        ))
        .unwrap();
    fleet
        .append_admit(Admit::new(
            sre.clone(),
            frank.clone(),
            admin.clone(),
            role("frank", 10),
            CatalogActionSet::allow([CatalogAction::Use]),
        ))
        .unwrap();
    let gov = GovernedFleet::new(fleet, grants);

    let err = gov
        .resolve_member_warrant(&frank, &asset, CatalogAction::Use, &owner_root)
        .unwrap_err();
    assert!(matches!(err, GovernedFleetError::Narrowing(_)));
}

/// A DE-AUTHORIZED admitter — one who admitted a member and has since lost their own
/// membership, and with it their `Delegate` edge — can NO LONGER evict that member
/// (tightened by D232: an inert admit confers no eviction). Owner removals are
/// unaffected; the owner's own removal mid-test still cascades.
#[test]
fn de_authorized_admitter_cannot_evict_what_they_once_admitted() {
    let admin = PartyId::new("admin");
    let sre = PartyId::new("team:sre");
    let bob = PartyId::new("bob");
    let carol = PartyId::new("carol");

    let fleet = InMemoryMembershipLedger::new();
    fleet
        .append_founding(Team::found(sre.clone(), admin.clone(), "SRE"))
        .unwrap();
    // Bob is admitted WITH Delegate; Bob admits Carol.
    fleet
        .append_admit(Admit::new(
            sre.clone(),
            bob.clone(),
            admin.clone(),
            role("bob", 50),
            CatalogActionSet::allow([CatalogAction::Use, CatalogAction::Delegate]),
        ))
        .unwrap();
    fleet
        .append_admit(Admit::new(
            sre.clone(),
            carol.clone(),
            bob.clone(),
            role("carol", 50),
            CatalogActionSet::allow([CatalogAction::Use]),
        ))
        .unwrap();
    // The owner removes Bob; Carol cascades off (Bob no longer authorizes her).
    fleet
        .append_remove(Removal::new(sre.clone(), bob.clone(), admin.clone()))
        .unwrap();
    assert!(!fleet.is_member(&carol, &sre));

    // Re-admit Carol directly by the OWNER (so she's active again).
    fleet
        .append_admit(Admit::new(
            sre.clone(),
            carol.clone(),
            admin.clone(),
            role("carol2", 50),
            CatalogActionSet::allow([CatalogAction::Use]),
        ))
        .unwrap();
    assert!(fleet.is_member(&carol, &sre));
    // Bob (a former admitter of Carol, now de-authorized — the owner removed him, so he
    // holds no active Delegate edge) records a removal. It is INERT: he is neither the
    // owner nor an authorized admitter, so Carol's owner-granted admit survives.
    fleet
        .append_remove(Removal::new(sre.clone(), carol.clone(), bob.clone()))
        .unwrap();
    assert!(
        fleet.is_member(&carol, &sre),
        "a de-authorized admitter with no active edge cannot evict"
    );
}

/// D232 regression — the removal-DoS. A party with NO authority on the team self-issues
/// an `Admit` for a member (recorded, but inert as an edge: they hold no `Delegate`),
/// then records a `Removal`. Because removal authority was once read off the mere
/// PRESENCE of an admit, that inert fact used to confer eviction rights over a member
/// the OWNER admitted. It must not.
#[test]
fn an_inert_self_issued_admit_confers_no_eviction_rights() {
    let admin = PartyId::new("admin");
    let sre = PartyId::new("team:sre");
    let alice = PartyId::new("alice");
    let mallory = PartyId::new("mallory");

    let fleet = InMemoryMembershipLedger::new();
    fleet
        .append_founding(Team::found(sre.clone(), admin.clone(), "SRE"))
        .unwrap();
    // The OWNER admits Alice — a legitimate, authorized edge.
    fleet
        .append_admit(Admit::new(
            sre.clone(),
            alice.clone(),
            admin.clone(),
            role("alice", 50),
            CatalogActionSet::allow([CatalogAction::Use]),
        ))
        .unwrap();
    assert!(fleet.is_member(&alice, &sre));

    // Mallory — not the owner, not a member, holding no Delegate — self-issues an admit
    // for Alice. The fact is RECORDED; as an edge it is inert (it mints nothing).
    fleet
        .append_admit(Admit::new(
            sre.clone(),
            alice.clone(),
            mallory.clone(),
            role("alice-by-mallory", 50),
            CatalogActionSet::allow([CatalogAction::Use]),
        ))
        .unwrap();
    // Now Mallory tries to evict Alice on the strength of that inert admit.
    fleet
        .append_remove(Removal::new(sre.clone(), alice.clone(), mallory.clone()))
        .unwrap();

    assert!(
        fleet.is_member(&alice, &sre),
        "an inert self-issued admit must not authorize a removal (D232 removal-DoS)"
    );
}

/// An owner disband is TERMINAL: re-founding the same team principal cannot
/// resurrect it (genesis is idempotent; the disband persists in the fold).
#[test]
fn disband_is_terminal_across_refounding() {
    let admin = PartyId::new("admin");
    let sre = PartyId::new("team:sre");
    let alice = PartyId::new("alice");

    let fleet = InMemoryMembershipLedger::new();
    fleet
        .append_founding(Team::found(sre.clone(), admin.clone(), "SRE"))
        .unwrap();
    fleet
        .append_admit(Admit::new(
            sre.clone(),
            alice.clone(),
            admin.clone(),
            role("a", 50),
            CatalogActionSet::all(),
        ))
        .unwrap();
    fleet
        .append_disband(Disband::new(sre.clone(), admin.clone()))
        .unwrap();
    assert!(!fleet.is_member(&alice, &sre));

    // Re-founding is idempotent (no new genesis) and does NOT clear the disband.
    let out = fleet
        .append_founding(Team::found(sre.clone(), admin.clone(), "SRE-again"))
        .unwrap();
    assert!(matches!(out, MembershipOutcome::AlreadyPresent(_)));
    assert!(
        !fleet.is_member(&alice, &sre),
        "a disbanded team stays disbanded — re-founding cannot resurrect it"
    );
}
