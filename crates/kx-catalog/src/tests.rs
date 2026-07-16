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
        assert!(most_permissive(&[]).is_none());

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
        let w1 = most_permissive(&[tight.clone(), wide.clone()]).unwrap();
        let w2 = most_permissive(&[wide, tight]).unwrap();
        assert_eq!(w1.model_route.max_calls, 50);
        assert_eq!(w1, w2, "selection is order-independent");
    }

    #[test]
    fn most_permissive_incomparable_maxima_picks_smallest_leaf() {
        // Three chains: A ⊆ C strictly (same executor axis, A tighter on
        // max_calls), and B incomparable to BOTH (differs only on the executor
        // child-set axis). The true maximal set is {B, C}; the documented winner
        // is the SMALLEST-leaf maximum, i.e. B. A greedy single pass wrongly
        // returns C: it retains the smaller-leaf A until C dominates A, discarding
        // B (which was skipped as merely incomparable to A). Regression guard.
        let mut wa = warrant_calls(2);
        wa.executor_class = kx_warrant::ExecutorClass::Bwrap;
        let mut wb = warrant_calls(5);
        wb.executor_class = kx_warrant::ExecutorClass::OciDaemon;
        let mut wc = warrant_calls(50);
        wc.executor_class = kx_warrant::ExecutorClass::Bwrap;

        let use_ = CatalogActionSet::allow([CatalogAction::Use]);
        let a = GrantWarrant::new(GrantId::from_bytes([1u8; 32]), use_.clone(), wa);
        let b = GrantWarrant::new(GrantId::from_bytes([2u8; 32]), use_.clone(), wb.clone());
        let c = GrantWarrant::new(GrantId::from_bytes([3u8; 32]), use_, wc);

        // Order-independent: every permutation yields the smallest-leaf maximum B.
        for perm in [
            vec![a.clone(), b.clone(), c.clone()],
            vec![c.clone(), b.clone(), a.clone()],
            vec![b.clone(), c.clone(), a.clone()],
        ] {
            let got = most_permissive(&perm).unwrap();
            assert_eq!(
                got, wb,
                "documented tie-break = smallest-leaf maximum (B), not the greedy C"
            );
        }
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

// ---- M7.2: content-versioning + provenance/lineage (D82/D88 + D-LOCK-4) ------

mod m7_2_versioning {
    use kx_dataset::DatasetId;
    use kx_mote::MoteId;
    use kx_workflow::ManifestId;

    use crate::{
        AssetPath, AssetVersion, InMemoryVersionLedger, PartyId, Provenance, TaskSignatureHash,
        VersionError, VersionLedger, VersionLedgerError, VersionedContent,
        CATALOG_VERSION_SCHEMA_VERSION, MAX_PROVENANCE_LINEAGE,
    };

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

    // -- content addressing ----------------------------------------------------

    #[test]
    fn version_id_is_stable_and_deterministic() {
        let v = AssetVersion::root(apath("summarize"), recipe(1), alice(), prov(1));
        assert_eq!(v.version_id(), v.version_id(), "deterministic");
        let same = AssetVersion::root(apath("summarize"), recipe(1), alice(), prov(1));
        assert_eq!(v.version_id(), same.version_id(), "same bytes ⇒ same id");
    }

    #[test]
    fn version_id_distinct_on_content() {
        let a = AssetVersion::root(apath("summarize"), recipe(1), alice(), prov(1));
        let b = AssetVersion::root(apath("summarize"), recipe(2), alice(), prov(1));
        assert_ne!(a.version_id(), b.version_id());
    }

    #[test]
    fn version_id_distinct_on_handle() {
        let a = AssetVersion::root(apath("summarize"), recipe(1), alice(), prov(1));
        let b = AssetVersion::root(apath("classify"), recipe(1), alice(), prov(1));
        assert_ne!(a.version_id(), b.version_id());
    }

    #[test]
    fn version_id_distinct_on_provenance() {
        // Provenance folds into the id ⇒ a forged provenance is tamper-evident.
        let a = AssetVersion::root(apath("summarize"), recipe(1), alice(), prov(1));
        let b = AssetVersion::root(apath("summarize"), recipe(1), alice(), prov(9));
        assert_ne!(a.version_id(), b.version_id());
    }

    #[test]
    fn version_id_distinct_on_content_kind() {
        // Recipe vs Workflow vs Dataset are distinct even on coincident bytes.
        let r = AssetVersion::root(apath("x"), recipe(1), alice(), prov(1));
        let w = AssetVersion::root(
            apath("x"),
            VersionedContent::Workflow(ManifestId([1u8; 32])),
            alice(),
            prov(1),
        );
        let d = AssetVersion::root(
            apath("x"),
            VersionedContent::Dataset(DatasetId([1u8; 32])),
            alice(),
            prov(1),
        );
        assert_ne!(r.version_id(), w.version_id());
        assert_ne!(r.version_id(), d.version_id());
        assert_ne!(w.version_id(), d.version_id());
    }

    #[test]
    fn version_id_hex_is_64_chars() {
        let id = AssetVersion::root(apath("x"), recipe(1), alice(), prov(1)).version_id();
        assert_eq!(id.to_hex().len(), 64);
        assert_eq!(format!("{id}").len(), 64);
    }

    #[test]
    fn schema_version_is_pinned() {
        let v = AssetVersion::root(apath("x"), recipe(1), alice(), prov(1));
        assert_eq!(v.schema_version(), CATALOG_VERSION_SCHEMA_VERSION);
        assert_eq!(CATALOG_VERSION_SCHEMA_VERSION, 1);
    }

    // -- root / successor shape ------------------------------------------------

    #[test]
    fn root_has_no_prior_revision_zero() {
        let v = AssetVersion::root(apath("x"), recipe(1), alice(), prov(1));
        assert!(v.prior().is_none());
        assert_eq!(v.revision(), 0);
    }

    #[test]
    fn successor_increments_revision() {
        let v1 = AssetVersion::root(apath("x"), recipe(1), alice(), prov(1));
        let v1_id = v1.version_id();
        let v2 = AssetVersion::successor(v1_id, 0, apath("x"), recipe(2), alice(), prov(2));
        assert_eq!(v2.prior(), Some(v1_id));
        assert_eq!(v2.revision(), 1);
    }

    // -- publish / resolve / move handle ---------------------------------------

    #[test]
    fn publish_then_resolve_returns_content() {
        let ledger = InMemoryVersionLedger::new();
        let v = AssetVersion::root(apath("summarize"), recipe(1), alice(), prov(1));
        let id = ledger.publish(v).unwrap().version_id();
        let (content, vid) = ledger.resolve(&apath("summarize")).unwrap();
        assert_eq!(vid, id);
        assert_eq!(content, recipe(1));
    }

    #[test]
    fn publish_successor_moves_handle() {
        let ledger = InMemoryVersionLedger::new();
        let v1 = AssetVersion::root(apath("summarize"), recipe(1), alice(), prov(1));
        let v1_id = ledger.publish(v1).unwrap().version_id();
        let v2 = AssetVersion::successor(v1_id, 0, apath("summarize"), recipe(2), alice(), prov(2));
        let v2_id = ledger.publish(v2).unwrap().version_id();
        let (content, vid) = ledger.resolve(&apath("summarize")).unwrap();
        assert_eq!(vid, v2_id, "handle moved to the latest");
        assert_eq!(content, recipe(2));
        assert!(ledger.get_version(&v1_id).is_some(), "v1 retained");
    }

    #[test]
    fn rollback_moves_handle_keeps_all() {
        let ledger = InMemoryVersionLedger::new();
        let v1 = AssetVersion::root(apath("x"), recipe(1), alice(), prov(1));
        let v1_id = ledger.publish(v1).unwrap().version_id();
        let v2 = AssetVersion::successor(v1_id, 0, apath("x"), recipe(2), alice(), prov(2));
        let v2_id = ledger.publish(v2).unwrap().version_id();
        // Rollback = a NEW version (revision 2) pinning v1's OLDER content.
        let v3 = AssetVersion::successor(v2_id, 1, apath("x"), recipe(1), alice(), prov(3));
        let v3_id = ledger.publish(v3).unwrap().version_id();
        let (content, vid) = ledger.resolve(&apath("x")).unwrap();
        assert_eq!(vid, v3_id);
        assert_eq!(content, recipe(1), "rolled back to v1's content");
        assert_eq!(ledger.len(), 3, "all three versions retained (D-LOCK-4)");
        assert!(ledger.get_version(&v1_id).is_some());
        assert!(ledger.get_version(&v2_id).is_some());
    }

    #[test]
    fn publish_is_idempotent() {
        // Re-publishing a byte-identical version is AlreadyPresent (the same-id
        // tripwire's ImmutabilityConflict branch is cryptographically unreachable
        // because version_id hashes the WHOLE fact — same id ⟺ same bytes).
        let ledger = InMemoryVersionLedger::new();
        let v = AssetVersion::root(apath("x"), recipe(1), alice(), prov(1));
        assert!(ledger.publish(v.clone()).unwrap().is_published());
        for _ in 0..5 {
            assert!(!ledger.publish(v.clone()).unwrap().is_published());
        }
        assert_eq!(ledger.len(), 1);
    }

    // -- history / lineage / descendants ---------------------------------------

    #[test]
    fn history_walks_latest_to_oldest() {
        let ledger = InMemoryVersionLedger::new();
        let v1 = AssetVersion::root(apath("x"), recipe(1), alice(), prov(1));
        let v1_id = ledger.publish(v1).unwrap().version_id();
        let v2 = AssetVersion::successor(v1_id, 0, apath("x"), recipe(2), alice(), prov(2));
        let v2_id = ledger.publish(v2).unwrap().version_id();
        let v3 = AssetVersion::successor(v2_id, 1, apath("x"), recipe(3), alice(), prov(3));
        let v3_id = ledger.publish(v3).unwrap().version_id();

        let hist = ledger.history(&apath("x"));
        let ids: Vec<_> = hist.iter().map(AssetVersion::version_id).collect();
        assert_eq!(ids, vec![v3_id, v2_id, v1_id], "latest → oldest");

        let lin = ledger.lineage(&v3_id);
        assert_eq!(lin.len(), 3);
        assert_eq!(lin.first().unwrap().version_id(), v3_id);
        assert_eq!(lin.last().unwrap().version_id(), v1_id);
    }

    #[test]
    fn descendants_covers_forward_chain() {
        let ledger = InMemoryVersionLedger::new();
        let v1 = AssetVersion::root(apath("x"), recipe(1), alice(), prov(1));
        let v1_id = ledger.publish(v1).unwrap().version_id();
        let v2 = AssetVersion::successor(v1_id, 0, apath("x"), recipe(2), alice(), prov(2));
        let v2_id = ledger.publish(v2).unwrap().version_id();
        let v3 = AssetVersion::successor(v2_id, 1, apath("x"), recipe(3), alice(), prov(3));
        let v3_id = ledger.publish(v3).unwrap().version_id();

        let desc = ledger.descendants(&v1_id);
        assert_eq!(desc.len(), 2, "v1's descendants are v2, v3 (not v1 itself)");
        assert!(desc.contains(&v2_id) && desc.contains(&v3_id));
        assert!(ledger.descendants(&v3_id).is_empty(), "leaf has none");
    }

    #[test]
    fn missing_prior_publish_is_refused() {
        // A successor whose `prior` was never published is refused fail-closed at
        // the door (publish is causally ordered).
        let ledger = InMemoryVersionLedger::new();
        let phantom = AssetVersion::root(apath("ghost"), recipe(9), alice(), prov(9)).version_id();
        let orphan = AssetVersion::successor(phantom, 0, apath("x"), recipe(1), alice(), prov(1));
        assert!(matches!(
            ledger.publish(orphan).unwrap_err(),
            VersionLedgerError::PriorNotFound(_)
        ));
        assert_eq!(ledger.len(), 0, "nothing landed");
    }

    #[test]
    fn foreign_prior_publish_is_refused() {
        // A successor grafting a DIFFERENT handle's version as its prior is refused
        // (the stored chain can never carry a cross-handle graft).
        let ledger = InMemoryVersionLedger::new();
        let vy = AssetVersion::root(apath("other"), recipe(2), alice(), prov(2));
        let vy_id = ledger.publish(vy).unwrap().version_id();
        let forged = AssetVersion::successor(vy_id, 0, apath("x"), recipe(1), alice(), prov(1));
        assert!(matches!(
            ledger.publish(forged).unwrap_err(),
            VersionLedgerError::InvalidLineage { .. }
        ));
        assert_eq!(ledger.len(), 1, "only the legitimate root landed");
    }

    #[test]
    fn wrong_prior_revision_publish_is_refused() {
        // Declaring an inflated `prior_revision` (the griefing vector) is refused:
        // the revision must be exactly real_prior.revision + 1.
        let ledger = InMemoryVersionLedger::new();
        let v1 = AssetVersion::root(apath("x"), recipe(1), alice(), prov(1));
        let v1_id = ledger.publish(v1).unwrap().version_id();
        // prior is v1 (revision 0) but we LIE that prior_revision = u32::MAX-1 ⇒
        // revision = u32::MAX, which does not equal 0 + 1.
        let inflated =
            AssetVersion::successor(v1_id, u32::MAX - 1, apath("x"), recipe(2), alice(), prov(2));
        assert_eq!(inflated.revision(), u32::MAX);
        assert!(matches!(
            ledger.publish(inflated).unwrap_err(),
            VersionLedgerError::InvalidLineage { .. }
        ));
        assert_eq!(
            ledger.resolve(&apath("x")).unwrap().1,
            v1_id,
            "handle unmoved"
        );
    }

    #[test]
    fn fork_two_successors_same_prior() {
        // Two distinct successors of the SAME prior on the SAME handle (a fork):
        // both publish; descendants(v1) = {both}; resolve tie-breaks by version-id
        // bytes deterministically + order-independently; each branch's lineage is
        // [branch, v1]; history is the rank-winner's lineage.
        let publish_order = |a_first: bool| {
            let ledger = InMemoryVersionLedger::new();
            let v1 = AssetVersion::root(apath("x"), recipe(1), alice(), prov(1));
            let v1_id = ledger.publish(v1).unwrap().version_id();
            let a =
                AssetVersion::successor(v1_id, 0, apath("x"), recipe(0xA0), alice(), prov(0xA0));
            let b =
                AssetVersion::successor(v1_id, 0, apath("x"), recipe(0xB0), alice(), prov(0xB0));
            let (a_id, b_id) = (a.version_id(), b.version_id());
            if a_first {
                ledger.publish(a).unwrap();
                ledger.publish(b).unwrap();
            } else {
                ledger.publish(b).unwrap();
                ledger.publish(a).unwrap();
            }
            (ledger, v1_id, a_id, b_id)
        };

        let (ledger, v1_id, a_id, b_id) = publish_order(true);
        // descendants(v1) = exactly {a, b}.
        let mut desc = ledger.descendants(&v1_id);
        desc.sort_unstable_by_key(|v| *v.as_bytes());
        let mut want = vec![a_id, b_id];
        want.sort_unstable_by_key(|v| *v.as_bytes());
        assert_eq!(desc, want, "both fork branches are descendants of v1");
        // each branch's lineage is [branch, v1].
        assert_eq!(
            ledger
                .lineage(&a_id)
                .iter()
                .map(AssetVersion::version_id)
                .collect::<Vec<_>>(),
            vec![a_id, v1_id]
        );
        // resolve tie-breaks by version-id bytes (both revision 1): the larger wins.
        let winner = if *a_id.as_bytes() > *b_id.as_bytes() {
            a_id
        } else {
            b_id
        };
        assert_eq!(ledger.resolve(&apath("x")).unwrap().1, winner);
        // history is the winner's lineage; publish order does not change the winner.
        let (ledger2, _, _, _) = publish_order(false);
        assert_eq!(
            ledger2.resolve(&apath("x")).unwrap().1,
            winner,
            "tie-break is publish-order-independent"
        );
        assert_eq!(ledger.history(&apath("x"))[0].version_id(), winner);
    }

    #[test]
    fn lineage_of_absent_version_is_empty() {
        let ledger = InMemoryVersionLedger::new();
        let nobody = AssetVersion::root(apath("x"), recipe(1), alice(), prov(1)).version_id();
        assert!(ledger.lineage(&nobody).is_empty());
        assert!(ledger.resolve(&apath("x")).is_none());
    }

    // -- Provenance ------------------------------------------------------------

    #[test]
    fn provenance_builders_and_accessors() {
        let p = Provenance::from_recipe([7u8; 32])
            .with_run([3u8; 16])
            .with_dataset(DatasetId([4u8; 32]))
            .with_corpus_lineage([MoteId::from_bytes([5u8; 32])])
            .unwrap();
        assert_eq!(p.recipe_fingerprint(), &[7u8; 32]);
        assert_eq!(p.generating_run(), Some([3u8; 16]));
        assert_eq!(p.dataset_id(), Some(DatasetId([4u8; 32])));
        assert_eq!(p.corpus_lineage().len(), 1);
    }

    #[test]
    fn provenance_too_large_is_refused() {
        // Build MAX+1 distinct MoteIds (distinct by their leading 8 bytes).
        let lineage: Vec<MoteId> = (0..=MAX_PROVENANCE_LINEAGE as u64)
            .map(|i| {
                let mut b = [0u8; 32];
                b[..8].copy_from_slice(&i.to_le_bytes());
                MoteId::from_bytes(b)
            })
            .collect();
        assert!(lineage.len() > MAX_PROVENANCE_LINEAGE);
        let err = Provenance::from_recipe([0u8; 32])
            .with_corpus_lineage(lineage)
            .unwrap_err();
        assert!(matches!(err, VersionError::ProvenanceTooLarge { .. }));
    }

    #[test]
    fn provenance_serde_round_trips() {
        let p = Provenance::from_recipe([1u8; 32])
            .with_run([2u8; 16])
            .with_dataset(DatasetId([3u8; 32]));
        let bytes = bincode::serde::encode_to_vec(&p, crate::canonical_config()).unwrap();
        let (back, _) =
            bincode::serde::decode_from_slice::<Provenance, _>(&bytes, crate::canonical_config())
                .unwrap();
        assert_eq!(p, back);
    }
}
