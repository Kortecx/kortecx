// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! In-crate smoke tests: identity stability, schema-version honesty, and the
//! content-id no-aliasing property. The end-to-end governance + scale behavior lives
//! in `tests/*.rs` (separate-crate integration tests).

use kx_catalog::{CatalogAction, CatalogActionSet, PartyId};
use kx_warrant::{Role, WarrantSpec};

use crate::membership::{Admit, Removal};
use crate::team::{Team, FLEET_SCHEMA_VERSION};

fn role(name: &str) -> Role {
    Role {
        name: name.into(),
        version: 1,
        spec: WarrantSpec::default(),
        description: String::new(),
    }
}

#[test]
fn team_id_is_stable_and_content_addressed() {
    let a = Team::found(PartyId::new("team:x"), PartyId::new("owner"), "X");
    let b = Team::found(PartyId::new("team:x"), PartyId::new("owner"), "X");
    let c = Team::found(PartyId::new("team:x"), PartyId::new("owner"), "Y");
    assert_eq!(a.team_id(), b.team_id(), "byte-identical teams share an id");
    assert_ne!(
        a.team_id(),
        c.team_id(),
        "a display-name change is a new id"
    );
}

#[test]
fn schema_version_is_constructor_set() {
    let t = Team::found(PartyId::new("t"), PartyId::new("o"), "d");
    assert_eq!(t.schema_version(), FLEET_SCHEMA_VERSION);
    let a = Admit::new(
        PartyId::new("t"),
        PartyId::new("m"),
        PartyId::new("o"),
        role("r"),
        CatalogActionSet::allow([CatalogAction::Use]),
    );
    assert_eq!(a.schema_version(), FLEET_SCHEMA_VERSION);
}

#[test]
fn admit_id_folds_role_and_cap() {
    // Two admits differing ONLY in role are distinct facts (so a re-admit under a new
    // role is recorded, not silently deduped).
    let mk = |r: Role, cap: CatalogActionSet| {
        Admit::new(
            PartyId::new("t"),
            PartyId::new("m"),
            PartyId::new("o"),
            r,
            cap,
        )
        .admit_id()
    };
    let cap = CatalogActionSet::allow([CatalogAction::Use]);
    assert_eq!(mk(role("r"), cap.clone()), mk(role("r"), cap.clone()));
    assert_ne!(mk(role("r"), cap.clone()), mk(role("r2"), cap.clone()));
    assert_ne!(
        mk(role("r"), cap),
        mk(role("r"), CatalogActionSet::allow([CatalogAction::Read]))
    );
}

#[test]
fn content_ids_do_not_alias_across_party_concatenation() {
    // ("ab","c") vs ("a","bc") must not collide — canonical bincode length-prefixes
    // each string, so the boundary is unambiguous (no length-extension aliasing).
    let x = Removal::new(PartyId::new("ab"), PartyId::new("c"), PartyId::new("o")).removal_id();
    let y = Removal::new(PartyId::new("a"), PartyId::new("bc"), PartyId::new("o")).removal_id();
    assert_ne!(x, y);
}

#[test]
fn removal_and_admit_ids_are_domain_separated() {
    // Same parties, different fact kinds ⇒ different ids (per-variant domain tags).
    let admit = Admit::new(
        PartyId::new("t"),
        PartyId::new("m"),
        PartyId::new("o"),
        role("r"),
        CatalogActionSet::all(),
    )
    .admit_id();
    let removal =
        Removal::new(PartyId::new("t"), PartyId::new("m"), PartyId::new("o")).removal_id();
    assert_ne!(admit, removal);
}
