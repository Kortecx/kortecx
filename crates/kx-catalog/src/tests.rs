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
