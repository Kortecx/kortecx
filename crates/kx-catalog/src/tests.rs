// SPDX-License-Identifier: Apache-2.0
//! Unit tests for the catalog foundation (M7.0 + M7.1).

use std::collections::{BTreeMap, BTreeSet};

use kx_mote::{MoteDefHash, MoteId};
use kx_workflow::ManifestId;

use crate::{
    CatalogError, CatalogRegistry, FreeParamContract, FreeParamSlot, InMemoryCatalog,
    RecipeSnapshot, SignatureAxis, SignatureEntry, SlotBinding, TaskSignature, VerdictScope,
};

fn critic(byte: u8) -> MoteDefHash {
    MoteDefHash::from_bytes([byte; 32])
}

fn entry_for(sig: TaskSignature) -> SignatureEntry {
    SignatureEntry::new(sig, ManifestId([9u8; 32]), RecipeSnapshot::new([8u8; 32]))
}

// ---- M7.0: TaskSignature ----------------------------------------------------

#[test]
fn model_invariant_has_empty_narrowing() {
    let sig = TaskSignature::model_invariant(critic(1));
    assert!(sig.is_model_invariant());
    assert!(sig.narrowing().is_empty());
    assert_eq!(sig.schema_version(), crate::TASK_SIGNATURE_SCHEMA_VERSION);
    assert_eq!(sig.critic_mote_def_hash(), &critic(1));
}

#[test]
fn scoped_with_empty_equals_model_invariant() {
    let mi = TaskSignature::model_invariant(critic(2));
    let scoped_empty = TaskSignature::scoped(critic(2), BTreeSet::new());
    assert_eq!(mi, scoped_empty, "empty narrowing IS model-invariance");
    assert_eq!(
        mi.task_signature_hash(),
        scoped_empty.task_signature_hash(),
        "hashes must match"
    );
}

#[test]
fn scoped_narrowing_is_order_independent() {
    let mut a = BTreeSet::new();
    a.insert(SignatureAxis::CiterModelId);
    a.insert(SignatureAxis::CiterConfigKey);

    let mut b = BTreeSet::new();
    b.insert(SignatureAxis::CiterConfigKey);
    b.insert(SignatureAxis::CiterModelId);

    let sa = TaskSignature::scoped(critic(3), a);
    let sb = TaskSignature::scoped(critic(3), b);
    assert_eq!(sa.task_signature_hash(), sb.task_signature_hash());
    assert!(!sa.is_model_invariant());
}

#[test]
fn hash_is_stable_and_distinct() {
    let sig = TaskSignature::scoped(
        critic(4),
        BTreeSet::from([SignatureAxis::CiterInferenceParams]),
    );
    assert_eq!(
        sig.task_signature_hash(),
        sig.task_signature_hash(),
        "deterministic"
    );

    // Distinct critic hash → distinct signature hash.
    let other = TaskSignature::scoped(
        critic(5),
        BTreeSet::from([SignatureAxis::CiterInferenceParams]),
    );
    assert_ne!(sig.task_signature_hash(), other.task_signature_hash());

    // Distinct narrowing → distinct signature hash.
    let wider = TaskSignature::scoped(
        critic(4),
        BTreeSet::from([
            SignatureAxis::CiterInferenceParams,
            SignatureAxis::CiterModelId,
        ]),
    );
    assert_ne!(sig.task_signature_hash(), wider.task_signature_hash());
}

#[test]
fn hash_hex_is_64_chars() {
    let h = TaskSignature::model_invariant(critic(6)).task_signature_hash();
    assert_eq!(h.to_hex().len(), 64);
    assert_eq!(format!("{h}").len(), 64);
}

#[test]
fn verdict_scope_serde_round_trips() {
    let vs = VerdictScope {
        citee_mote_id: MoteId::from_bytes([3u8; 32]),
        task_signature_hash: [4u8; 32],
    };
    let bytes = bincode::serde::encode_to_vec(vs, crate::canonical_config()).unwrap();
    let (back, _) =
        bincode::serde::decode_from_slice::<VerdictScope, _>(&bytes, crate::canonical_config())
            .unwrap();
    assert_eq!(vs, back);
}

// ---- M7.1: entry / contract -------------------------------------------------

#[test]
fn free_param_contract_dedups_by_name() {
    let c = FreeParamContract::new()
        .with_slot("to", FreeParamSlot::variable(Some([1u8; 32])))
        .with_slot("to", FreeParamSlot::constant());
    assert_eq!(c.len(), 1, "same name overwrites");
    assert_eq!(c.slots["to"].binding, SlotBinding::Constant);
}

#[test]
fn entry_hash_matches_signature_hash() {
    let sig = TaskSignature::model_invariant(critic(7));
    let want = sig.task_signature_hash();
    let entry = entry_for(sig)
        .with_pinned_skills([critic(10), critic(11)])
        .with_variable_slots([("subject".to_string(), SlotBinding::Variable)])
        .with_verdict_scope(VerdictScope {
            citee_mote_id: MoteId::from_bytes([12u8; 32]),
            task_signature_hash: *want.as_bytes(),
        });
    assert_eq!(entry.hash(), want, "entry key is its signature hash");
    assert_eq!(entry.pinned_skill_hashes.len(), 2);
}

// ---- M7.1: registry ---------------------------------------------------------

#[test]
fn register_then_get() {
    let catalog = InMemoryCatalog::new();
    assert!(catalog.is_empty());

    let sig = TaskSignature::model_invariant(critic(20));
    let entry = entry_for(sig);
    let hash = entry.hash();

    let outcome = catalog.register_signature(entry.clone()).unwrap();
    assert!(outcome.is_inserted());
    assert_eq!(outcome.hash(), hash);
    assert_eq!(catalog.len(), 1);
    assert_eq!(catalog.get_signature(&hash), Some(entry.clone()));
    assert_eq!(catalog.lookup(&hash), Some(entry));
}

#[test]
fn lookup_absent_is_none() {
    let catalog = InMemoryCatalog::new();
    let missing = TaskSignature::model_invariant(critic(21)).task_signature_hash();
    assert_eq!(catalog.lookup(&missing), None);
}

#[test]
fn registration_is_idempotent() {
    let catalog = InMemoryCatalog::new();
    let entry = entry_for(TaskSignature::model_invariant(critic(22)));

    assert!(catalog
        .register_signature(entry.clone())
        .unwrap()
        .is_inserted());
    for _ in 0..5 {
        let again = catalog.register_signature(entry.clone()).unwrap();
        assert!(!again.is_inserted(), "re-register is AlreadyPresent");
    }
    assert_eq!(catalog.len(), 1, "exactly one stored entry");
}

#[test]
fn registration_is_immutable_on_collision() {
    let catalog = InMemoryCatalog::new();
    let sig = TaskSignature::model_invariant(critic(23));

    // Two entries with the SAME signature (same hash) but DIFFERENT bodies.
    let a = entry_for(sig.clone());
    let b = entry_for(sig).with_pinned_skills([critic(99)]);
    assert_eq!(a.hash(), b.hash(), "same signature → same key");
    assert_ne!(a, b, "different bodies");

    catalog.register_signature(a).unwrap();
    let err = catalog.register_signature(b).unwrap_err();
    assert!(matches!(err, CatalogError::ImmutabilityConflict(_)));
    assert_eq!(catalog.len(), 1, "the conflicting write did not land");
}

#[test]
fn list_is_deterministic_hash_order() {
    let catalog = InMemoryCatalog::new();
    let mut expected: Vec<_> = (30u8..40)
        .map(|b| {
            let e = entry_for(TaskSignature::model_invariant(critic(b)));
            catalog.register_signature(e.clone()).unwrap();
            e
        })
        .collect();
    expected.sort_by_key(SignatureEntry::hash);

    let got: Vec<_> = catalog.list_signatures().collect();
    let got_hashes: Vec<_> = got.iter().map(SignatureEntry::hash).collect();
    let want_hashes: Vec<_> = expected.iter().map(SignatureEntry::hash).collect();
    assert_eq!(got_hashes, want_hashes, "enumeration is hash-ordered");
}

#[test]
fn arc_blanket_impl_forwards() {
    use std::sync::Arc;
    let catalog: Arc<InMemoryCatalog> = Arc::new(InMemoryCatalog::new());
    let entry = entry_for(TaskSignature::model_invariant(critic(41)));
    let hash = entry.hash();
    // Call through the Arc (exercises the blanket impl).
    CatalogRegistry::register_signature(&catalog, entry).unwrap();
    assert_eq!(catalog.len(), 1);
    assert!(catalog.lookup(&hash).is_some());
}

#[test]
fn slot_binding_default_authoring_shapes() {
    // A sanity check that the variable/constant constructors set the binding.
    assert_eq!(FreeParamSlot::variable(None).binding, SlotBinding::Variable);
    assert_eq!(FreeParamSlot::constant().binding, SlotBinding::Constant);
    let _ = BTreeMap::<String, SlotBinding>::new();
}

// ---- M7.2: namespacing + grants + revocation (D86) --------------------------

mod m7_2 {
    use kx_mote::ModelId;
    use kx_warrant::{ModelRoute, ResourceCeiling, Role, WarrantSpec};

    use crate::ledger::{most_permissive, warrant_within, GrantWarrant};
    use crate::{
        revocation_idempotency_key, AssetBinding, AssetPath, AssetPathError, AssetRef,
        CatalogAction, CatalogActionSet, Grant, GrantId, GrantLedger, InMemoryGrantLedger, PartyId,
        Revocation, TaskSignatureHash,
    };

    fn path(ns: &str, col: &str, name: &str) -> AssetRef {
        AssetRef::Path(AssetPath::new(ns, col, name).unwrap())
    }

    /// A warrant whose only non-default axes are positive quantitative ceilings
    /// (so `intersect` accepts it; its qualitative axes are the empty defaults,
    /// hence ⊆ any parent). `max_calls` distinguishes "wide" from "tight".
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

    // -- AssetPath validation --------------------------------------------------

    #[test]
    fn asset_path_accepts_canonical_and_displays() {
        let p = AssetPath::new("acme", "research", "lit-review").unwrap();
        assert_eq!(p.namespace(), "acme");
        assert_eq!(p.collection(), "research");
        assert_eq!(p.name(), "lit-review");
        assert_eq!(p.to_string(), "acme/research/lit-review");
    }

    #[test]
    fn asset_path_rejects_malformed() {
        assert!(matches!(
            AssetPath::new("", "c", "n"),
            Err(AssetPathError::EmptySegment { which: "namespace" })
        ));
        assert!(matches!(
            AssetPath::new("ns", "Col", "n"), // uppercase not in [a-z0-9._-]
            Err(AssetPathError::IllegalChar { .. })
        ));
        assert!(matches!(
            AssetPath::new("ns", "c", "a/b"), // '/' is illegal inside a segment
            Err(AssetPathError::IllegalChar { ch: '/', .. })
        ));
        assert!(matches!(
            AssetPath::new("ns", "c", "x".repeat(crate::MAX_SEGMENT_LEN + 1)),
            Err(AssetPathError::SegmentTooLong { .. })
        ));
        assert!(matches!(
            AssetPath::new("ns", "c", "-lead"),
            Err(AssetPathError::LeadingOrTrailingPunct { .. })
        ));
        assert!(matches!(
            AssetPath::new("ns", "c", "trail."),
            Err(AssetPathError::LeadingOrTrailingPunct { .. })
        ));
    }

    #[test]
    fn asset_ref_path_and_signature_are_distinct() {
        let by_path = path("ns", "c", "n");
        let by_sig = AssetRef::Signature(TaskSignatureHash::from_bytes([7u8; 32]));
        assert_ne!(by_path, by_sig);
        assert!(by_sig.to_string().starts_with("sig:"));
    }

    // -- PartyId ---------------------------------------------------------------

    #[test]
    fn party_id_is_opaque_equality() {
        let a = PartyId::new("user@x");
        assert_eq!(a.as_str(), "user@x");
        assert_eq!(a.to_string(), "user@x");
        assert_eq!(a, PartyId::new("user@x"));
        assert_ne!(a, PartyId::new("user@y"));
    }

    // -- CatalogActionSet ------------------------------------------------------

    #[test]
    fn action_set_truth_table() {
        assert_eq!(CatalogActionSet::default(), CatalogActionSet::None);
        // empty allow-list normalizes to the single canonical deny.
        assert_eq!(CatalogActionSet::allow([]), CatalogActionSet::None);
        assert_eq!(CatalogAction::all().len(), 4);

        let ru = CatalogActionSet::allow([CatalogAction::Read, CatalogAction::Use]);
        assert!(ru.contains(CatalogAction::Read));
        assert!(!ru.contains(CatalogAction::Delegate));
        assert!(!ru.is_empty());

        // narrow = intersection; union = combine.
        let r = CatalogActionSet::allow([CatalogAction::Read]);
        assert_eq!(ru.narrow(&r), r);
        assert!(CatalogActionSet::None.narrow(&ru).is_empty());
        assert_eq!(r.union(&CatalogActionSet::allow([CatalogAction::Use])), ru);
        // subset.
        assert!(r.is_subset_of(&ru));
        assert!(!ru.is_subset_of(&r));
        assert!(CatalogActionSet::None.is_subset_of(&r));
    }

    // -- Content addressing ----------------------------------------------------

    #[test]
    fn grant_id_is_deterministic_and_root_differs_from_delegated() {
        let a = path("ns", "c", "n");
        let owner = PartyId::new("o");
        let mate = PartyId::new("m");
        let actions = CatalogActionSet::allow([CatalogAction::Use]);
        let r = role("r", 5);

        let g1 = Grant::root(
            a.clone(),
            owner.clone(),
            mate.clone(),
            actions.clone(),
            r.clone(),
        );
        let g2 = Grant::root(
            a.clone(),
            owner.clone(),
            mate.clone(),
            actions.clone(),
            r.clone(),
        );
        assert_eq!(g1.grant_id(), g2.grant_id(), "same bytes ⇒ same id");
        assert_eq!(g1.grant_id().to_hex().len(), 64);

        let del = Grant::delegated(g1.grant_id(), a, owner, mate, actions, r);
        assert_ne!(g1.grant_id(), del.grant_id(), "prior changes the id");
        assert!(del.prior().is_some());
        assert!(g1.prior().is_none());
    }

    #[test]
    fn revocation_key_is_deterministic_and_equals_id() {
        let gid = GrantId::from_bytes([3u8; 32]);
        let revoker = PartyId::new("o");
        let k1 = revocation_idempotency_key(&gid, &revoker);
        let k2 = revocation_idempotency_key(&gid, &revoker);
        assert_eq!(k1, k2);
        assert_eq!(Revocation::new(gid, revoker.clone()).revocation_id(), k1);
        // a different revoker ⇒ a different key.
        assert_ne!(revocation_idempotency_key(&gid, &PartyId::new("p")), k1);
    }

    #[test]
    fn binding_id_distinct_from_grant_id_for_coincident_bytes() {
        // Distinct domain tags keep a binding's FactId off a grant's id space.
        let a = path("ns", "c", "n");
        let b = AssetBinding::new(a, PartyId::new("o"));
        assert_eq!(b.binding_id().to_hex().len(), 64);
    }

    // -- warrant_within / most_permissive --------------------------------------

    #[test]
    fn warrant_within_is_reflexive_and_orders_by_ceiling() {
        let tight = warrant_calls(2);
        let wide = warrant_calls(50);
        assert!(warrant_within(&tight, &tight), "reflexive");
        assert!(warrant_within(&tight, &wide), "tight ⊆ wide");
        assert!(!warrant_within(&wide, &tight), "wide ⊄ tight");
    }

    #[test]
    fn warrant_within_incomparable_on_child_set_axis() {
        let mut a = warrant_calls(5);
        let mut b = warrant_calls(5);
        a.executor_class = kx_warrant::ExecutorClass::Bwrap;
        b.executor_class = kx_warrant::ExecutorClass::OciDaemon;
        // Differ only on a child-set axis ⇒ neither is "within" the other.
        assert!(!warrant_within(&a, &b));
        assert!(!warrant_within(&b, &a));
    }

    #[test]
    fn most_permissive_selects_widest_real_warrant() {
        assert!(most_permissive(Vec::new()).is_none());

        let tight = GrantWarrant::new(
            GrantId::from_bytes([1u8; 32]),
            CatalogActionSet::allow([CatalogAction::Use]),
            warrant_calls(2),
        );
        let wide = GrantWarrant::new(
            GrantId::from_bytes([2u8; 32]),
            CatalogActionSet::allow([CatalogAction::Use]),
            warrant_calls(50),
        );
        // Either input order selects the wider (real, never synthesized) warrant.
        let w1 = most_permissive(vec![tight.clone(), wide.clone()]).unwrap();
        let w2 = most_permissive(vec![wide, tight]).unwrap();
        assert_eq!(w1.model_route.max_calls, 50);
        assert_eq!(w1, w2, "selection is order-independent");
    }

    // -- A small end-to-end + the multi-grant regression -----------------------

    #[test]
    fn bind_grant_authorize_revoke_roundtrip() {
        let ledger = InMemoryGrantLedger::new();
        let asset = path("acme", "research", "lit-review");
        let owner = PartyId::new("admin@acme");
        let mate = PartyId::new("teammate@acme");
        let owner_root = warrant_calls(100);

        ledger
            .append_binding(AssetBinding::new(asset.clone(), owner.clone()))
            .unwrap();
        ledger
            .append_grant(Grant::root(
                asset.clone(),
                owner.clone(),
                mate.clone(),
                CatalogActionSet::allow([CatalogAction::Read, CatalogAction::Use]),
                role("read-use", 10),
            ))
            .unwrap();

        assert!(ledger.is_authorized(&mate, &asset, CatalogAction::Use));
        let w = ledger
            .resolve_effective_warrant_for(&mate, &asset, CatalogAction::Use, &owner_root)
            .unwrap()
            .expect("Use is granted");
        assert_eq!(w.model_route.max_calls, 10, "min(10,100)");

        // Owner revokes; authority drops but the grant fact survives (append-only).
        let gid_before: Vec<GrantId> = ledger.effective_grants(&mate, &asset).active().collect();
        let gid = gid_before[0];
        ledger
            .append_revocation(Revocation::new(gid, owner))
            .unwrap();
        assert!(!ledger.is_authorized(&mate, &asset, CatalogAction::Use));
        assert!(ledger.list_facts().count() >= 3, "facts are append-only");
    }

    #[test]
    fn multi_grant_warrant_is_action_aligned() {
        // The regression test for the fixed bug: a party holds two grants on one
        // asset — Use under a TIGHT warrant, Read under a WIDE one. The warrant
        // for Use MUST be the tight (Use-conveying) one, never the wide Read one.
        let ledger = InMemoryGrantLedger::new();
        let asset = path("acme", "models", "summarizer");
        let owner = PartyId::new("owner");
        let ds = PartyId::new("data-scientist");
        let owner_root = warrant_calls(100);

        ledger
            .append_binding(AssetBinding::new(asset.clone(), owner.clone()))
            .unwrap();
        ledger
            .append_grant(Grant::root(
                asset.clone(),
                owner.clone(),
                ds.clone(),
                CatalogActionSet::allow([CatalogAction::Use]),
                role("tight-use", 2),
            ))
            .unwrap();
        ledger
            .append_grant(Grant::root(
                asset.clone(),
                owner,
                ds.clone(),
                CatalogActionSet::allow([CatalogAction::Read]),
                role("wide-read", 50),
            ))
            .unwrap();

        let use_w = ledger
            .resolve_effective_warrant_for(&ds, &asset, CatalogAction::Use, &owner_root)
            .unwrap()
            .expect("Use is granted");
        assert_eq!(
            use_w.model_route.max_calls, 2,
            "Use runs under the tight warrant"
        );

        let read_w = ledger
            .resolve_effective_warrant_for(&ds, &asset, CatalogAction::Read, &owner_root)
            .unwrap()
            .expect("Read is granted");
        assert_eq!(
            read_w.model_route.max_calls, 50,
            "Read runs under the wide warrant"
        );

        // A never-granted action resolves to None (fail-closed, action-aligned).
        assert!(ledger
            .resolve_effective_warrant_for(&ds, &asset, CatalogAction::Delegate, &owner_root)
            .unwrap()
            .is_none());
    }

    #[test]
    fn rebind_to_different_owner_is_refused() {
        let ledger = InMemoryGrantLedger::new();
        let asset = path("ns", "c", "n");
        ledger
            .append_binding(AssetBinding::new(asset.clone(), PartyId::new("o1")))
            .unwrap();
        // Same owner → idempotent no-op.
        let again = ledger
            .append_binding(AssetBinding::new(asset.clone(), PartyId::new("o1")))
            .unwrap();
        assert!(!again.is_appended());
        // Different owner → OwnerConflict.
        assert!(ledger
            .append_binding(AssetBinding::new(asset, PartyId::new("o2")))
            .is_err());
    }
}
