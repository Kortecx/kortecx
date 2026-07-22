// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! M7.3 Mote-as-MCP advertisement (D85/D86) — descriptor-only, governance-gated.
//!
//! Proves: a granted, published snapshot advertises a typed MCP descriptor whose
//! `input_schema` round-trips through `validate_args` (the M8 contract); an
//! ungranted / confused-deputy / unpublished snapshot is refused fail-closed;
//! `Constant` slots are excluded; untyped/unresolved/garbage schemas are refused;
//! and advertisement is a pure read (emits no run / appends no fact).

#![allow(clippy::unwrap_used)]

use kx_catalog::{
    advertise_snapshot, encode_param_schema, validate_args, AdvertiseError, AssetBinding,
    AssetPath, AssetRef, AssetVersion, CatalogAction, CatalogActionSet, FreeParamContract,
    FreeParamSlot, GovernedCatalog, Grant, GrantLedger, InMemoryGrantLedger, InMemoryVersionLedger,
    ParamType, PartyId, Provenance, RecipeSnapshot, Role, TaskSignatureHash, VersionLedger,
    VersionedContent, WarrantSpec,
};

type Catalog = GovernedCatalog<InMemoryGrantLedger, InMemoryVersionLedger>;

fn role() -> Role {
    Role {
        name: "r".into(),
        version: 1,
        spec: WarrantSpec::default(),
        description: String::new(),
    }
}

fn handle() -> AssetPath {
    AssetPath::new("acme", "recipes", "summarize").unwrap()
}

/// The schema content-ref convention used by these tests.
const LIMIT_SCHEMA_REF: [u8; 32] = [42u8; 32];

/// A snapshot with one Variable typed slot (`limit`) + one Constant slot
/// (`internal`, must be excluded from the descriptor).
fn snapshot() -> RecipeSnapshot {
    RecipeSnapshot::new([1u8; 32]).with_free_params(
        FreeParamContract::new()
            .with_slot("limit", FreeParamSlot::variable(Some(LIMIT_SCHEMA_REF)))
            .with_slot("internal", FreeParamSlot::constant()),
    )
}

/// Resolver: `LIMIT_SCHEMA_REF` → canonical bincode of `Int{1..=100}`.
fn resolver(r: &[u8; 32]) -> Option<Vec<u8>> {
    if *r == LIMIT_SCHEMA_REF {
        Some(encode_param_schema(&ParamType::Int {
            min: Some(1),
            max: Some(100),
        }))
    } else {
        None
    }
}

/// Build a governed catalog: `owner` (full authority, can publish), `user` (Use),
/// `reader` (Read). The snapshot's handle is published.
fn setup() -> (Catalog, PartyId, PartyId) {
    let owner = PartyId::new("owner");
    let user = PartyId::new("user");
    let reader = PartyId::new("reader");
    let h = handle();
    let asset = AssetRef::Path(h.clone());

    let grants = InMemoryGrantLedger::new();
    grants
        .append_binding(AssetBinding::new(asset.clone(), owner.clone()))
        .unwrap();
    // Owner self-grant (full authority) → owner may publish (Register).
    grants
        .append_grant(Grant::root(
            asset.clone(),
            owner.clone(),
            owner.clone(),
            CatalogActionSet::all(),
            role(),
        ))
        .unwrap();
    grants
        .append_grant(Grant::root(
            asset.clone(),
            owner.clone(),
            user.clone(),
            CatalogActionSet::allow([CatalogAction::Use]),
            role(),
        ))
        .unwrap();
    grants
        .append_grant(Grant::root(
            asset,
            owner.clone(),
            reader.clone(),
            CatalogActionSet::allow([CatalogAction::Read]),
            role(),
        ))
        .unwrap();

    let versions = InMemoryVersionLedger::new();
    let governed = GovernedCatalog::new(grants, versions);
    governed
        .publish(AssetVersion::root(
            h,
            VersionedContent::Recipe(TaskSignatureHash::from_bytes([1u8; 32])),
            owner,
            Provenance::from_recipe([1u8; 32]),
        ))
        .unwrap();
    (governed, user, reader)
}

#[test]
fn granted_use_party_gets_a_descriptor_that_validates() {
    let (cat, user, _) = setup();
    let ad = advertise_snapshot(
        &cat,
        &user,
        &handle(),
        &snapshot(),
        "summarize an incident",
        &resolver,
    )
    .unwrap();

    assert_eq!(ad.name, "acme/recipes/summarize");
    assert_eq!(ad.description, "summarize an incident");
    assert_eq!(ad.asset, AssetRef::Path(handle()));
    // Only the Variable slot becomes a param; the Constant slot is excluded.
    assert_eq!(ad.input_schema.params.len(), 1);
    assert_eq!(ad.input_schema.params[0].name, "limit");
    assert!(ad.input_schema.params[0].required);
    assert!(ad.input_schema.deny_unknown);

    // The descriptor's schema is the M8 contract — it round-trips through validate_args.
    assert!(validate_args(&ad.input_schema, br#"{"limit": 50}"#).is_ok());
    assert!(validate_args(&ad.input_schema, br#"{"limit": 999}"#).is_err()); // out of [1,100]
    assert!(validate_args(&ad.input_schema, br#"{"limit": 1.5}"#).is_err()); // no float (SN-8)
    assert!(validate_args(&ad.input_schema, br#"{"bogus": 1}"#).is_err()); // deny_unknown + missing
}

#[test]
fn read_only_party_can_advertise() {
    let (cat, _, reader) = setup();
    assert!(advertise_snapshot(&cat, &reader, &handle(), &snapshot(), "d", &resolver).is_ok());
}

#[test]
fn ungranted_party_is_refused() {
    let (cat, _, _) = setup();
    let intruder = PartyId::new("intruder");
    let err =
        advertise_snapshot(&cat, &intruder, &handle(), &snapshot(), "d", &resolver).unwrap_err();
    assert!(matches!(err, AdvertiseError::Unauthorized { .. }));
}

#[test]
fn confused_deputy_grant_on_other_asset_does_not_advertise() {
    let (cat, _, _) = setup();
    // `stranger` is granted Use on a DIFFERENT asset; it must not advertise `handle`.
    let owner = PartyId::new("owner");
    let stranger = PartyId::new("stranger");
    let other = AssetRef::Path(AssetPath::new("acme", "recipes", "other").unwrap());
    cat.grants()
        .append_binding(AssetBinding::new(other.clone(), owner.clone()))
        .unwrap();
    cat.grants()
        .append_grant(Grant::root(
            other,
            owner,
            stranger.clone(),
            CatalogActionSet::allow([CatalogAction::Use]),
            role(),
        ))
        .unwrap();
    let err =
        advertise_snapshot(&cat, &stranger, &handle(), &snapshot(), "d", &resolver).unwrap_err();
    assert!(matches!(err, AdvertiseError::Unauthorized { .. }));
}

#[test]
fn unpublished_handle_is_refused() {
    let (cat, _, _) = setup();
    // A handle the user is granted Read on, but with no published version.
    let owner = PartyId::new("owner");
    let user = PartyId::new("user");
    let unpub = AssetPath::new("acme", "recipes", "draft").unwrap();
    let asset = AssetRef::Path(unpub.clone());
    cat.grants()
        .append_binding(AssetBinding::new(asset.clone(), owner.clone()))
        .unwrap();
    cat.grants()
        .append_grant(Grant::root(
            asset,
            owner,
            user.clone(),
            CatalogActionSet::allow([CatalogAction::Read]),
            role(),
        ))
        .unwrap();
    let err = advertise_snapshot(&cat, &user, &unpub, &snapshot(), "d", &resolver).unwrap_err();
    assert!(matches!(err, AdvertiseError::NotPublished(_)));
}

#[test]
fn untyped_variable_slot_is_refused() {
    let (cat, user, _) = setup();
    let snap = RecipeSnapshot::new([1u8; 32])
        .with_free_params(FreeParamContract::new().with_slot("x", FreeParamSlot::variable(None)));
    let err = advertise_snapshot(&cat, &user, &handle(), &snap, "d", &resolver).unwrap_err();
    assert!(matches!(err, AdvertiseError::UntypedVariableSlot { slot } if slot == "x"));
}

#[test]
fn unresolved_schema_ref_is_refused() {
    let (cat, user, _) = setup();
    let snap = RecipeSnapshot::new([1u8; 32]).with_free_params(
        FreeParamContract::new().with_slot("x", FreeParamSlot::variable(Some([7u8; 32]))),
    );
    // resolver only knows LIMIT_SCHEMA_REF.
    let err = advertise_snapshot(&cat, &user, &handle(), &snap, "d", &resolver).unwrap_err();
    assert!(matches!(err, AdvertiseError::SchemaUnresolved { slot } if slot == "x"));
}

#[test]
fn garbage_schema_bytes_are_refused() {
    let (cat, user, _) = setup();
    let bad_ref = [9u8; 32];
    let snap = RecipeSnapshot::new([1u8; 32]).with_free_params(
        FreeParamContract::new().with_slot("x", FreeParamSlot::variable(Some(bad_ref))),
    );
    let bad_resolver = |r: &[u8; 32]| (*r == bad_ref).then(|| vec![0xFFu8, 0x00, 0x01]);
    let err = advertise_snapshot(&cat, &user, &handle(), &snap, "d", &bad_resolver).unwrap_err();
    assert!(matches!(err, AdvertiseError::SchemaDecode { slot } if slot == "x"));
}

#[test]
fn param_types_round_trip_through_the_descriptor() {
    let (cat, user, _) = setup();
    for ty in [
        ParamType::Int {
            min: None,
            max: None,
        },
        ParamType::Str { max_len: 64 },
        ParamType::Bytes { max_len: 8 },
        ParamType::Bool,
    ] {
        let r = [200u8; 32];
        let snap = RecipeSnapshot::new([1u8; 32]).with_free_params(
            FreeParamContract::new().with_slot("p", FreeParamSlot::variable(Some(r))),
        );
        let ty2 = ty.clone();
        let res = move |q: &[u8; 32]| (*q == r).then(|| encode_param_schema(&ty2));
        let ad = advertise_snapshot(&cat, &user, &handle(), &snap, "d", &res).unwrap();
        assert_eq!(ad.input_schema.params[0].ty, ty);
    }
}

#[test]
fn advertise_emits_no_run_and_appends_no_fact() {
    let (cat, user, _) = setup();
    let versions_before = cat.versions().len();
    let grants_before = cat.grants().len();
    let _ = advertise_snapshot(&cat, &user, &handle(), &snapshot(), "d", &resolver).unwrap();
    // Descriptor-only: no coordinator/journal, no new fact in either ledger.
    assert_eq!(cat.versions().len(), versions_before);
    assert_eq!(cat.grants().len(), grants_before);
}

/// M7.3 (PR-B) exit gate: the new `kx-catalog → kx-tool-registry` edge must NOT
/// create a cycle — kx-tool-registry and the crates it depends on must stay off
/// kx-catalog, so the SN-8 wall and the dependency direction hold (complements
/// `guarantee_path_does_not_depend_on_catalog` in `security_governance.rs`).
#[test]
fn advertisement_edge_introduces_no_cycle() {
    for c in ["kx-tool-registry", "kx-mote", "kx-content", "kx-warrant"] {
        let manifest = format!("{}/../{c}/Cargo.toml", env!("CARGO_MANIFEST_DIR"));
        let toml =
            std::fs::read_to_string(&manifest).unwrap_or_else(|e| panic!("read {manifest}: {e}"));
        assert!(
            !toml.contains("kx-catalog"),
            "{c} must NOT depend on kx-catalog (no cycle via the M7.3 advertise edge)"
        );
    }
}
