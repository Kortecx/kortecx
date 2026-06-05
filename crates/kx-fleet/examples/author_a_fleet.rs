// SPDX-License-Identifier: Apache-2.0
//! Author a team / fleet and resolve a member's effective warrant (M7, D112).
//!
//! Run with:
//!
//! ```sh
//! cargo run -p kx-fleet --example author_a_fleet
//! ```
//!
//! This is the fleet analogue of `kx-workflow`'s `author_a_workflow`: zero model, zero
//! network, fully deterministic. It shows the whole D112 story end to end —
//!
//! 1. an org admin **founds a team** and **grants the team** `Use` on a catalog recipe
//!    (an ordinary `kx-catalog` grant — a team is just a `PartyId`);
//! 2. the admin **admits two members** under different runtime roles;
//! 3. each member's effective warrant on the recipe is resolved as
//!    `intersect(team_grant_warrant, member_role)` — structurally `⊆` the team's grant
//!    (a member can never exceed the team);
//! 4. a **nested fleet** (a team admitted into a fleet that holds the grant) resolves
//!    the same way, narrowed at every hop;
//! 5. **removing** a member makes their access vanish immediately (revoke-by-new-fact).
//!
//! See the README (How it works) / GLOSSARY.md for how this sits OFF the trust path (SN-8): the
//! fleet layer never gates selection/promotion and never touches the journal.

// Example code: `.unwrap()` on the ledger appends is the right "fail loud in a demo"
// behavior, and the linear narrative reads better as one `main` than split helpers.
#![allow(clippy::unwrap_used, clippy::too_many_lines)]

use kx_catalog::{
    AssetBinding, AssetPath, AssetRef, CatalogAction, CatalogActionSet, Grant, GrantLedger,
    InMemoryGrantLedger, PartyId,
};
use kx_fleet::{Admit, GovernedFleet, InMemoryMembershipLedger, MembershipLedger, Removal, Team};
use kx_mote::ModelId;
use kx_warrant::{ModelRoute, ResourceCeiling, Role, WarrantSpec};

/// A warrant capping inference calls at `max_calls` (positive model route, since
/// `kx_warrant::intersect` rejects a zero route).
fn warrant(max_calls: u32) -> WarrantSpec {
    WarrantSpec {
        model_route: ModelRoute {
            model_id: ModelId("gemma".into()),
            max_input_tokens: 4_096,
            max_output_tokens: 1_024,
            max_calls,
        },
        resource_ceiling: ResourceCeiling {
            cpu_milli: 2_000,
            mem_bytes: 512 << 20,
            wall_clock_ms: 30_000,
            fd_count: 64,
            disk_bytes: 1 << 30,
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

fn main() {
    // The principals. A team is a group `PartyId`, exactly like a user.
    let admin = PartyId::new("admin@acme");
    let sre = PartyId::new("team:sre@acme");
    let platform = PartyId::new("fleet:platform@acme");
    let alice = PartyId::new("alice@acme");
    let bob = PartyId::new("bob@acme");
    let frank = PartyId::new("frank@acme");

    // The shared catalog recipe + the org's base warrant (the ceiling everything
    // narrows under).
    let recipe = AssetRef::Path(AssetPath::new("acme", "runbooks", "restart-service").unwrap());
    let owner_root = warrant(100);

    // ---- Step 1: grant the TEAM `Use` on the recipe (an ordinary catalog grant) ----
    let grants = InMemoryGrantLedger::new();
    grants
        .append_binding(AssetBinding::new(recipe.clone(), admin.clone()))
        .unwrap();
    grants
        .append_grant(Grant::root(
            recipe.clone(),
            admin.clone(),
            sre.clone(),
            CatalogActionSet::allow([CatalogAction::Use]),
            role("sre-team", 50),
        ))
        .unwrap();

    // ---- Step 2: found the team + admit two members under different roles ----------
    let fleet = InMemoryMembershipLedger::new();
    fleet
        .append_founding(Team::found(sre.clone(), admin.clone(), "SRE on-call"))
        .unwrap();
    fleet
        .append_admit(Admit::new(
            sre.clone(),
            alice.clone(),
            admin.clone(),
            role("oncall-senior", 30),
            CatalogActionSet::allow([CatalogAction::Use]),
        ))
        .unwrap();
    fleet
        .append_admit(Admit::new(
            sre.clone(),
            bob.clone(),
            admin.clone(),
            role("oncall-junior", 5),
            CatalogActionSet::allow([CatalogAction::Use]),
        ))
        .unwrap();

    let gov = GovernedFleet::new(fleet, grants);

    // ---- Step 3: resolve each member's effective warrant (narrowed, never wider) ---
    let team_eff = gov
        .grants()
        .resolve_effective_warrant_for(&sre, &recipe, CatalogAction::Use, &owner_root)
        .unwrap()
        .unwrap();
    let alice_w = gov
        .resolve_member_warrant(&alice, &recipe, CatalogAction::Use, &owner_root)
        .unwrap()
        .unwrap();
    let bob_w = gov
        .resolve_member_warrant(&bob, &recipe, CatalogAction::Use, &owner_root)
        .unwrap()
        .unwrap();

    println!("recipe: {recipe}");
    println!(
        "team SRE effective max_calls = {} (= min(owner 100, team 50))",
        team_eff.model_route.max_calls
    );
    println!(
        "  alice (oncall-senior, 30) -> max_calls = {} (= min(team 50, 30))",
        alice_w.model_route.max_calls
    );
    println!(
        "  bob   (oncall-junior,  5) -> max_calls = {} (= min(team 50, 5))",
        bob_w.model_route.max_calls
    );
    assert_eq!(alice_w.model_route.max_calls, 30);
    assert_eq!(bob_w.model_route.max_calls, 5);
    assert!(alice_w.model_route.max_calls <= team_eff.model_route.max_calls);

    // ---- Step 4: a NESTED fleet resolves the same way, narrowed at every hop -------
    // A separate recipe the FLEET (not the team) holds; team:sre is a member of it.
    let deploy = AssetRef::Path(AssetPath::new("acme", "runbooks", "deploy").unwrap());
    gov.grants()
        .append_binding(AssetBinding::new(deploy.clone(), admin.clone()))
        .unwrap();
    gov.grants()
        .append_grant(Grant::root(
            deploy.clone(),
            admin.clone(),
            platform.clone(),
            CatalogActionSet::allow([CatalogAction::Use]),
            role("platform-fleet", 80),
        ))
        .unwrap();
    gov.members()
        .append_founding(Team::found(
            platform.clone(),
            admin.clone(),
            "Platform fleet",
        ))
        .unwrap();
    gov.members()
        .append_admit(Admit::new(
            platform.clone(),
            sre.clone(),
            admin.clone(),
            role("sre-in-platform", 40),
            CatalogActionSet::allow([CatalogAction::Use]),
        ))
        .unwrap();
    gov.members()
        .append_admit(Admit::new(
            sre.clone(),
            frank.clone(),
            admin.clone(),
            role("frank", 25),
            CatalogActionSet::allow([CatalogAction::Use]),
        ))
        .unwrap();
    let frank_w = gov
        .resolve_member_warrant(&frank, &deploy, CatalogAction::Use, &owner_root)
        .unwrap()
        .unwrap();
    println!(
        "  frank via fleet:platform -> max_calls = {} (= min(owner 100, fleet 80, sre-in-platform 40, frank 25))",
        frank_w.model_route.max_calls
    );
    assert_eq!(frank_w.model_route.max_calls, 25);

    // ---- Step 5: remove a member -> access vanishes immediately ---------------------
    assert!(gov.is_member_authorized(&bob, &recipe, CatalogAction::Use));
    gov.members()
        .append_remove(Removal::new(sre.clone(), bob.clone(), admin.clone()))
        .unwrap();
    assert!(!gov.is_member_authorized(&bob, &recipe, CatalogAction::Use));
    println!(
        "removed bob from team:sre -> bob authorized for Use? {}",
        gov.is_member_authorized(&bob, &recipe, CatalogAction::Use)
    );

    println!(
        "\nOK: a member's warrant is always the team's grant narrowed by their role — never wider."
    );
}
