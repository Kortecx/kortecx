// SPDX-License-Identifier: Apache-2.0
//! Property tests for the M7.1 registry: idempotency, immutability,
//! lookup totality, and deterministic hash-ordered enumeration.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use kx_catalog::{
    CatalogError, CatalogRegistry, InMemoryCatalog, RecipeSnapshot, SignatureAxis, SignatureEntry,
    TaskSignature,
};
use kx_mote::MoteDefHash;
use kx_workflow::ManifestId;
use proptest::prelude::*;

fn axis_strategy() -> impl Strategy<Value = SignatureAxis> {
    prop_oneof![
        Just(SignatureAxis::CiterModelId),
        Just(SignatureAxis::CiterPromptTemplateHash),
        Just(SignatureAxis::CiterToolContractEntry),
        Just(SignatureAxis::CiterInferenceParams),
        Just(SignatureAxis::CiterConfigKey),
    ]
}

fn sig_strategy() -> impl Strategy<Value = TaskSignature> {
    (
        any::<[u8; 32]>(),
        prop::collection::btree_set(axis_strategy(), 0..=5),
    )
        .prop_map(|(h, axes)| TaskSignature::scoped(MoteDefHash::from_bytes(h), axes))
}

fn entry_strategy() -> impl Strategy<Value = SignatureEntry> {
    (
        sig_strategy(),
        any::<[u8; 32]>(),
        any::<[u8; 32]>(),
        prop::collection::vec(any::<[u8; 32]>(), 0..=4),
    )
        .prop_map(|(sig, manifest, fingerprint, skills)| {
            SignatureEntry::new(sig, ManifestId(manifest), RecipeSnapshot::new(fingerprint))
                .with_pinned_skills(skills.into_iter().map(MoteDefHash::from_bytes))
        })
}

proptest! {
    /// Registering the same entry any number of times stores exactly one copy;
    /// only the first is an insert.
    #[test]
    fn registration_is_idempotent(entry in entry_strategy(), reps in 1usize..6) {
        let catalog = InMemoryCatalog::new();
        prop_assert!(catalog.register_signature(entry.clone()).unwrap().is_inserted());
        for _ in 0..reps {
            let again = catalog.register_signature(entry.clone()).unwrap();
            prop_assert!(!again.is_inserted());
        }
        prop_assert_eq!(catalog.len(), 1);
    }

    /// get(absent) is None; get(register(e)) is Some(e).
    #[test]
    fn lookup_is_total(entry in entry_strategy()) {
        let catalog = InMemoryCatalog::new();
        let hash = entry.hash();
        prop_assert_eq!(catalog.lookup(&hash), None);
        catalog.register_signature(entry.clone()).unwrap();
        prop_assert_eq!(catalog.lookup(&hash), Some(entry));
    }

    /// Two entries with the SAME signature but DIFFERENT bodies collide on the
    /// key; the second registration is refused and does not land.
    #[test]
    fn registration_is_immutable(
        sig in sig_strategy(),
        skills_a in prop::collection::vec(any::<[u8; 32]>(), 0..=3),
        skills_b in prop::collection::vec(any::<[u8; 32]>(), 0..=3),
    ) {
        let a = SignatureEntry::new(sig.clone(), ManifestId([0u8; 32]), RecipeSnapshot::new([0u8; 32]))
            .with_pinned_skills(skills_a.into_iter().map(MoteDefHash::from_bytes));
        let b = SignatureEntry::new(sig, ManifestId([0u8; 32]), RecipeSnapshot::new([0u8; 32]))
            .with_pinned_skills(skills_b.into_iter().map(MoteDefHash::from_bytes));
        prop_assume!(a != b);
        prop_assert_eq!(a.hash(), b.hash());

        let catalog = InMemoryCatalog::new();
        catalog.register_signature(a).unwrap();
        let err = catalog.register_signature(b).unwrap_err();
        prop_assert!(matches!(err, CatalogError::ImmutabilityConflict(_)));
        prop_assert_eq!(catalog.len(), 1);
    }

    /// Enumeration is in hash order regardless of insertion order.
    #[test]
    fn list_is_hash_ordered(entries in prop::collection::vec(entry_strategy(), 0..12)) {
        let catalog = InMemoryCatalog::new();
        for e in &entries {
            // A duplicate-signature collision returns Err and is intentionally skipped.
            let _ = catalog.register_signature(e.clone());
        }
        let listed: Vec<_> = catalog.list_signatures().map(|e| e.hash()).collect();
        let mut sorted = listed.clone();
        sorted.sort_unstable();
        prop_assert_eq!(listed, sorted);
    }
}
