// SPDX-License-Identifier: Apache-2.0
//! Scale-smoke: the membership ledger + the composed fleet resolution stay
//! sub-linear / depth-bounded at scale (M7, D112).
//!
//! `#[ignore]`d — run in `--release` via the `scale-smoke` recipe. Append + membership
//! queries are `BTreeMap` insert/get keyed by `(team, member)`, so both are O(log n);
//! a nested resolution walks at most `MAX_TEAM_MEMBERS_WALK` hops, so its cost is
//! bounded by the depth cap, NOT by how deep the fleet nests. Mirrors
//! `kx-catalog/tests/scale.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Instant;

use kx_catalog::{
    AssetBinding, AssetPath, AssetRef, CatalogAction, CatalogActionSet, Grant, GrantLedger,
    InMemoryGrantLedger, PartyId,
};
use kx_fleet::{
    Admit, GovernedFleet, InMemoryMembershipLedger, MembershipLedger, Team, MAX_TEAM_MEMBERS_WALK,
};
use kx_mote::ModelId;
use kx_warrant::{ModelRoute, ResourceCeiling, Role, WarrantSpec};

const SIZES: &[usize] = &[1_000, 5_000, 10_000, 25_000];

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

fn assert_sublinear(label: &str, series: &[(usize, f64)]) {
    let first = series.first().unwrap().1;
    let last = series.last().unwrap().1;
    assert!(
        last <= first * 4.0,
        "{label} must stay sub-linear (n=1k {first:.1}ns vs n=25k {last:.1}ns)"
    );
}

/// Membership append + per-member query (`is_member` / `member_edges`) stay O(log n)
/// in the number of distinct (team, member) edges — the `BTreeMap` indices keep
/// governance sub-linear at fleet scale.
#[test]
#[ignore = "scale-smoke: run with --release --ignored --nocapture --test-threads=1"]
fn membership_append_and_query_stay_sublinear() {
    let owner = PartyId::new("owner");
    let mut admit_ns: Vec<(usize, f64)> = Vec::new();
    let mut is_member_ns: Vec<(usize, f64)> = Vec::new();
    let mut edges_ns: Vec<(usize, f64)> = Vec::new();

    for &n in SIZES {
        let fleet = InMemoryMembershipLedger::new();
        // n distinct teams, each with one member (a distinct (team, member) edge).
        let teams: Vec<PartyId> = (0..n).map(|i| PartyId::new(format!("t{i}"))).collect();
        let members: Vec<PartyId> = (0..n).map(|i| PartyId::new(format!("m{i}"))).collect();
        for t in &teams {
            fleet
                .append_founding(Team::found(t.clone(), owner.clone(), "T"))
                .unwrap();
        }

        let start = Instant::now();
        for (t, m) in teams.iter().zip(&members) {
            fleet
                .append_admit(Admit::new(
                    t.clone(),
                    m.clone(),
                    owner.clone(),
                    role_calls(10),
                    CatalogActionSet::allow([CatalogAction::Use]),
                ))
                .unwrap();
        }
        let admit_elapsed = start.elapsed();

        let start = Instant::now();
        for (t, m) in teams.iter().zip(&members) {
            assert!(fleet.is_member(m, t));
        }
        let is_member_elapsed = start.elapsed();

        let start = Instant::now();
        for m in &members {
            assert_eq!(fleet.member_edges(m).len(), 1);
        }
        let edges_elapsed = start.elapsed();

        #[allow(clippy::cast_precision_loss)]
        let per = |d: std::time::Duration| d.as_nanos() as f64 / n as f64;
        println!(
            "fleet-membership: n={n} admit_per_ns={:.1} is_member_per_ns={:.1} edges_per_ns={:.1}",
            per(admit_elapsed),
            per(is_member_elapsed),
            per(edges_elapsed)
        );
        admit_ns.push((n, per(admit_elapsed)));
        is_member_ns.push((n, per(is_member_elapsed)));
        edges_ns.push((n, per(edges_elapsed)));
    }

    assert_sublinear("membership-admit", &admit_ns);
    assert_sublinear("membership-is-member", &is_member_ns);
    assert_sublinear("membership-edges", &edges_ns);
}

/// The full composed resolution (`resolve_member_warrant`: membership fold + grant
/// fold + intersect) stays flat per-op as the fleet grows — n distinct
/// (team, member, asset) triples, each a one-hop resolution.
#[test]
#[ignore = "scale-smoke: run with --release --ignored --nocapture --test-threads=1"]
fn governed_resolve_member_warrant_stays_sublinear() {
    let admin = PartyId::new("admin");
    let owner_root = warrant_calls(100);
    let mut resolve_ns: Vec<(usize, f64)> = Vec::new();
    let mut authz_ns: Vec<(usize, f64)> = Vec::new();

    for &n in SIZES {
        let grants = InMemoryGrantLedger::new();
        let fleet = InMemoryMembershipLedger::new();
        let teams: Vec<PartyId> = (0..n).map(|i| PartyId::new(format!("t{i}"))).collect();
        let members: Vec<PartyId> = (0..n).map(|i| PartyId::new(format!("m{i}"))).collect();
        let assets: Vec<AssetRef> = (0..n)
            .map(|i| AssetRef::Path(AssetPath::new("ns", "c", format!("a{i}")).unwrap()))
            .collect();
        for ((t, m), a) in teams.iter().zip(&members).zip(&assets) {
            grants
                .append_binding(AssetBinding::new(a.clone(), admin.clone()))
                .unwrap();
            grants
                .append_grant(Grant::root(
                    a.clone(),
                    admin.clone(),
                    t.clone(),
                    CatalogActionSet::allow([CatalogAction::Use]),
                    role_calls(50),
                ))
                .unwrap();
            fleet
                .append_founding(Team::found(t.clone(), admin.clone(), "T"))
                .unwrap();
            fleet
                .append_admit(Admit::new(
                    t.clone(),
                    m.clone(),
                    admin.clone(),
                    role_calls(25),
                    CatalogActionSet::allow([CatalogAction::Use]),
                ))
                .unwrap();
        }
        let gov = GovernedFleet::new(fleet, grants);

        let start = Instant::now();
        for (m, a) in members.iter().zip(&assets) {
            assert!(gov
                .resolve_member_warrant(m, a, CatalogAction::Use, &owner_root)
                .unwrap()
                .is_some());
        }
        let resolve_elapsed = start.elapsed();

        let start = Instant::now();
        for (m, a) in members.iter().zip(&assets) {
            assert!(gov.is_member_authorized(m, a, CatalogAction::Use));
        }
        let authz_elapsed = start.elapsed();

        #[allow(clippy::cast_precision_loss)]
        let per = |d: std::time::Duration| d.as_nanos() as f64 / n as f64;
        println!(
            "fleet-resolve: n={n} resolve_per_ns={:.1} authz_per_ns={:.1}",
            per(resolve_elapsed),
            per(authz_elapsed)
        );
        resolve_ns.push((n, per(resolve_elapsed)));
        authz_ns.push((n, per(authz_elapsed)));
    }

    assert_sublinear("fleet-resolve-warrant", &resolve_ns);
    assert_sublinear("fleet-authorize", &authz_ns);
}

/// A nested resolution is BOUNDED by `MAX_TEAM_MEMBERS_WALK` (64), independent of how
/// deeply the fleet actually nests — the `DoS` / stack guard. A 50k-deep nesting chain
/// queries no slower than a 1k-deep one (both walk at most 64 hops, then fail closed),
/// so cost stays flat as depth grows. Mirrors `kx-catalog`'s `deep_chain_query_is_bounded`.
#[test]
#[ignore = "scale-smoke: run with --release --ignored --nocapture --test-threads=1"]
fn deep_nested_chain_query_is_bounded() {
    const DEPTHS: &[usize] = &[1_000, 10_000, 50_000];
    const ITERS: usize = 2_000;
    let admin = PartyId::new("admin");
    let mut query_ns: Vec<(usize, f64)> = Vec::new();

    for &depth in DEPTHS {
        let grants = InMemoryGrantLedger::new();
        let fleet = InMemoryMembershipLedger::new();
        let asset = AssetRef::Path(AssetPath::new("ns", "c", "deep").unwrap());
        let team = |i: usize| PartyId::new(format!("t{i}"));
        for i in 0..=depth {
            fleet
                .append_founding(Team::found(team(i), admin.clone(), "T"))
                .unwrap();
        }
        // t{i} ∈ t{i+1}; the leaf member is in t0; the TOP team holds the grant.
        for i in 0..depth {
            fleet
                .append_admit(Admit::new(
                    team(i + 1),
                    team(i),
                    admin.clone(),
                    role_calls(50),
                    CatalogActionSet::all(),
                ))
                .unwrap();
        }
        let member = PartyId::new("leaf");
        fleet
            .append_admit(Admit::new(
                team(0),
                member.clone(),
                admin.clone(),
                role_calls(50),
                CatalogActionSet::all(),
            ))
            .unwrap();
        grants
            .append_binding(AssetBinding::new(asset.clone(), admin.clone()))
            .unwrap();
        grants
            .append_grant(Grant::root(
                asset.clone(),
                admin.clone(),
                team(depth),
                CatalogActionSet::allow([CatalogAction::Use]),
                role_calls(50),
            ))
            .unwrap();
        let gov = GovernedFleet::new(fleet, grants);

        // The grant is beyond MAX hops ⇒ the walk caps at MAX, independent of `depth`.
        let start = Instant::now();
        for _ in 0..ITERS {
            let _ = gov.is_member_authorized(&member, &asset, CatalogAction::Use);
        }
        let elapsed = start.elapsed();
        #[allow(clippy::cast_precision_loss)]
        let per = elapsed.as_nanos() as f64 / ITERS as f64;
        println!("fleet-deep-nest: depth={depth} query_per_ns={per:.1}");
        query_ns.push((depth, per));

        // Sanity: unreachable beyond the bound (the top grant is `depth` > MAX hops up).
        assert!(
            depth <= MAX_TEAM_MEMBERS_WALK
                || !gov.is_member_authorized(&member, &asset, CatalogAction::Use)
        );
    }

    let first = query_ns.first().unwrap().1;
    let last = query_ns.last().unwrap().1;
    assert!(
        last <= first * 4.0,
        "deep-nest query must be depth-bounded (depth 1k {first:.1}ns vs 50k {last:.1}ns)"
    );
}
