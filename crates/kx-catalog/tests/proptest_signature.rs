// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! Property tests for the M7.0 [`TaskSignature`] foundation: canonical
//! round-trip, hash determinism + order-independence, schema-version pinning,
//! and a collision-resistance sanity check.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeSet;

use kx_catalog::{canonical_config, SignatureAxis, TaskSignature, TASK_SIGNATURE_SCHEMA_VERSION};
use kx_mote::MoteDefHash;
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

proptest! {
    /// Canonical bincode is an exact, lossless round-trip (no trailing garbage).
    #[test]
    fn bincode_round_trip(sig in sig_strategy()) {
        let bytes = bincode::serde::encode_to_vec(&sig, canonical_config()).unwrap();
        let (back, consumed) =
            bincode::serde::decode_from_slice::<TaskSignature, _>(&bytes, canonical_config())
                .unwrap();
        prop_assert_eq!(consumed, bytes.len());
        prop_assert_eq!(sig, back);
    }

    /// The signature hash is a deterministic pure function of the value.
    #[test]
    fn hash_is_deterministic(sig in sig_strategy()) {
        prop_assert_eq!(sig.task_signature_hash(), sig.task_signature_hash());
    }

    /// The narrowing is a set: the order axes are inserted never affects the hash.
    #[test]
    fn hash_is_order_independent(
        h in any::<[u8; 32]>(),
        axes in prop::collection::vec(axis_strategy(), 0..=8),
    ) {
        let forward: BTreeSet<SignatureAxis> = axes.iter().copied().collect();
        let backward: BTreeSet<SignatureAxis> = axes.into_iter().rev().collect();
        let a = TaskSignature::scoped(MoteDefHash::from_bytes(h), forward);
        let b = TaskSignature::scoped(MoteDefHash::from_bytes(h), backward);
        prop_assert_eq!(a.task_signature_hash(), b.task_signature_hash());
    }

    /// Every constructed signature carries the current schema version.
    #[test]
    fn schema_version_is_pinned(sig in sig_strategy()) {
        prop_assert_eq!(sig.schema_version(), TASK_SIGNATURE_SCHEMA_VERSION);
    }

    /// Distinct signatures hash to distinct ids (collision-resistance sanity).
    #[test]
    fn distinct_signatures_have_distinct_hashes(a in sig_strategy(), b in sig_strategy()) {
        prop_assume!(a != b);
        prop_assert_ne!(a.task_signature_hash(), b.task_signature_hash());
    }
}
