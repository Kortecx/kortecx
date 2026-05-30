//! Unit + property tests for the deterministic-critic vocabulary: canonical
//! verdict encode/decode round-trips and `CheckSpec::hash_into`
//! determinism + identity-discrimination.

use std::collections::BTreeSet;

use proptest::prelude::*;
use smallvec::smallvec;

use crate::{
    CheckSpec, CriticReason, CriticVerdict, DedupSpec, PiiClass, PiiSpec, RecordFraming,
    SchemaFault, SchemaSpec, SchemaTag, StatBoundsSpec, StatKind, TensorDTypeTag,
    VerdictDecodeError, CRITIC_SCHEMA_VERSION,
};

fn all_reason_samples() -> Vec<CriticReason> {
    vec![
        CriticReason::SchemaMismatch {
            expected: SchemaTag::Json,
            detail: SchemaFault::TagMismatch,
        },
        CriticReason::SchemaMismatch {
            expected: SchemaTag::Tensor {
                dtype: TensorDTypeTag::F32,
                shape: smallvec![2, 3],
            },
            detail: SchemaFault::ShapeMismatch {
                expected_elems: 6,
                actual_bytes: 20,
            },
        },
        CriticReason::SchemaMismatch {
            expected: SchemaTag::Text,
            detail: SchemaFault::NotUtf8 { at_offset: 7 },
        },
        CriticReason::SchemaMismatch {
            expected: SchemaTag::Json,
            detail: SchemaFault::NotJson { at_offset: 3 },
        },
        CriticReason::DuplicateDetected {
            duplicate_count: 4,
            first_duplicate_index: 9,
        },
        CriticReason::StatOutOfBounds {
            stat: StatKind::MeanScaled,
            observed_scaled: -12,
            lo_scaled: 0,
            hi_scaled: 100,
        },
        CriticReason::PiiLeak {
            class: PiiClass::CreditCardLuhn,
            match_offset: 42,
            match_len: 16,
        },
        CriticReason::Unparseable {
            check: crate::CheckKind::Dedup,
            at_offset: 11,
        },
    ]
}

#[test]
fn verdict_encode_carries_version_prefix() {
    let bytes = CriticVerdict::Valid.encode();
    assert_eq!(&bytes[0..2], &CRITIC_SCHEMA_VERSION.to_le_bytes());
}

#[test]
fn verdict_round_trip_all_variants() {
    let mut verdicts = vec![CriticVerdict::Valid];
    for reason in all_reason_samples() {
        verdicts.push(CriticVerdict::Invalid { reason });
    }
    for v in verdicts {
        let bytes = v.encode();
        let back = CriticVerdict::decode(&bytes).expect("decode");
        assert_eq!(v, back);
    }
}

#[test]
fn verdict_encode_is_deterministic() {
    let v = CriticVerdict::Invalid {
        reason: CriticReason::DuplicateDetected {
            duplicate_count: 1,
            first_duplicate_index: 0,
        },
    };
    assert_eq!(v.encode(), v.encode());
    // An independently constructed equal value encodes identically.
    let v2 = CriticVerdict::Invalid {
        reason: CriticReason::DuplicateDetected {
            duplicate_count: 1,
            first_duplicate_index: 0,
        },
    };
    assert_eq!(v.encode(), v2.encode());
    assert_eq!(v.content_ref_bytes(), v2.content_ref_bytes());
}

#[test]
fn decode_rejects_unknown_version() {
    let mut bytes = CriticVerdict::Valid.encode();
    bytes[0] = 0xFF;
    bytes[1] = 0xFF;
    assert_eq!(
        CriticVerdict::decode(&bytes),
        Err(VerdictDecodeError::UnknownSchemaVersion(0xFFFF))
    );
}

#[test]
fn decode_rejects_short_and_trailing_garbage() {
    assert_eq!(
        CriticVerdict::decode(&[]),
        Err(VerdictDecodeError::Malformed)
    );
    assert_eq!(
        CriticVerdict::decode(&[0]),
        Err(VerdictDecodeError::Malformed)
    );
    let mut bytes = CriticVerdict::Valid.encode();
    bytes.push(0xAB); // trailing garbage
    assert_eq!(
        CriticVerdict::decode(&bytes),
        Err(VerdictDecodeError::Malformed)
    );
}

#[test]
fn valid_distinct_from_invalid_ref() {
    let valid = CriticVerdict::Valid;
    let invalid = CriticVerdict::Invalid {
        reason: CriticReason::Unparseable {
            check: crate::CheckKind::Schema,
            at_offset: 0,
        },
    };
    assert_ne!(valid.content_ref_bytes(), invalid.content_ref_bytes());
    assert!(valid.is_valid());
    assert!(!invalid.is_valid());
}

fn digest(spec: &CheckSpec) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    spec.hash_into(&mut h);
    *h.finalize().as_bytes()
}

#[test]
fn hash_into_is_deterministic() {
    let spec = CheckSpec::StatBounds(StatBoundsSpec {
        framing: RecordFraming::LinesLf,
        stat: StatKind::MeanScaled,
        scale: 1000,
        lo_scaled: -5,
        hi_scaled: 5,
        numeric_field_range: Some((0, 4)),
    });
    assert_eq!(digest(&spec), digest(&spec));
}

#[test]
fn hash_into_discriminates_specs() {
    let base = CheckSpec::Dedup(DedupSpec {
        framing: RecordFraming::LinesLf,
        key_range: None,
    });
    let other_framing = CheckSpec::Dedup(DedupSpec {
        framing: RecordFraming::LengthPrefixedU32,
        key_range: None,
    });
    let other_key = CheckSpec::Dedup(DedupSpec {
        framing: RecordFraming::LinesLf,
        key_range: Some((0, 4)),
    });
    let other_kind = CheckSpec::Schema(SchemaSpec {
        expected: SchemaTag::Blob,
    });
    assert_ne!(digest(&base), digest(&other_framing));
    assert_ne!(digest(&base), digest(&other_key));
    assert_ne!(digest(&base), digest(&other_kind));
}

#[test]
fn pii_set_order_is_canonical() {
    // BTreeSet ordering means the digest is independent of insertion order.
    let mut a = BTreeSet::new();
    a.insert(PiiClass::UsSsn);
    a.insert(PiiClass::Email);
    let mut b = BTreeSet::new();
    b.insert(PiiClass::Email);
    b.insert(PiiClass::UsSsn);
    let sa = CheckSpec::PiiLeak(PiiSpec { forbidden: a });
    let sb = CheckSpec::PiiLeak(PiiSpec { forbidden: b });
    assert_eq!(digest(&sa), digest(&sb));
}

// --- proptest -------------------------------------------------------------

prop_compose! {
    fn arb_schema_tag()(
        tag in 0u8..7,
        dim in 0u32..512,
        d0 in 0u64..8, d1 in 0u64..8,
    ) -> SchemaTag {
        match tag {
            0 => SchemaTag::Blob,
            1 => SchemaTag::Text,
            2 => SchemaTag::Json,
            3 => SchemaTag::Tensor { dtype: TensorDTypeTag::F32, shape: smallvec![d0, d1] },
            4 => SchemaTag::Vector { dim },
            5 => SchemaTag::Image,
            _ => SchemaTag::Audio,
        }
    }
}

fn arb_reason() -> impl Strategy<Value = CriticReason> {
    prop_oneof![
        (arb_schema_tag()).prop_map(|expected| CriticReason::SchemaMismatch {
            expected,
            detail: SchemaFault::TagMismatch
        }),
        (any::<u64>(), any::<u64>()).prop_map(|(duplicate_count, first_duplicate_index)| {
            CriticReason::DuplicateDetected {
                duplicate_count,
                first_duplicate_index,
            }
        }),
        (any::<i64>(), any::<i64>(), any::<i64>()).prop_map(
            |(observed_scaled, lo_scaled, hi_scaled)| CriticReason::StatOutOfBounds {
                stat: StatKind::MeanScaled,
                observed_scaled,
                lo_scaled,
                hi_scaled,
            }
        ),
        (any::<u64>(), any::<u64>()).prop_map(|(match_offset, match_len)| {
            CriticReason::PiiLeak {
                class: PiiClass::Email,
                match_offset,
                match_len,
            }
        }),
    ]
}

fn arb_verdict() -> impl Strategy<Value = CriticVerdict> {
    prop_oneof![
        Just(CriticVerdict::Valid),
        arb_reason().prop_map(|reason| CriticVerdict::Invalid { reason }),
    ]
}

proptest! {
    #[test]
    fn prop_verdict_round_trip(v in arb_verdict()) {
        let bytes = v.encode();
        let back = CriticVerdict::decode(&bytes).expect("decode");
        prop_assert_eq!(v, back);
    }

    #[test]
    fn prop_verdict_encode_deterministic(v in arb_verdict()) {
        prop_assert_eq!(v.encode(), v.encode());
        prop_assert_eq!(v.content_ref_bytes(), v.content_ref_bytes());
    }

    #[test]
    fn prop_decode_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..256)) {
        // Total: decode returns a Result for ALL inputs — never panics.
        let _ = CriticVerdict::decode(&bytes);
    }
}
