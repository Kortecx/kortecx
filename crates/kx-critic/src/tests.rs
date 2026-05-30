//! Unit + property tests for the four deterministic checks: correctness,
//! determinism, and total-on-adversarial-input (no panic) coverage.

use std::collections::BTreeSet;

use proptest::prelude::*;
use smallvec::smallvec;

use kx_critic_types::{
    CheckSpec, CriticReason, CriticVerdict, DedupSpec, PiiClass, PiiSpec, RecordFraming,
    SchemaFault, SchemaSpec, SchemaTag, StatBoundsSpec, StatKind, TensorDTypeTag,
};

use crate::evaluate;

fn schema(tag: SchemaTag) -> CheckSpec {
    CheckSpec::Schema(SchemaSpec { expected: tag })
}

// --- schema check ---------------------------------------------------------

#[test]
fn schema_blob_always_valid() {
    assert_eq!(
        evaluate(&schema(SchemaTag::Blob), &[0xFF, 0x00, 0x13]),
        CriticVerdict::Valid
    );
    assert_eq!(
        evaluate(&schema(SchemaTag::Blob), &[]),
        CriticVerdict::Valid
    );
}

#[test]
fn schema_text_rejects_non_utf8() {
    assert_eq!(
        evaluate(&schema(SchemaTag::Text), b"hello"),
        CriticVerdict::Valid
    );
    let v = evaluate(&schema(SchemaTag::Text), &[0x68, 0xFF, 0x69]);
    match v {
        CriticVerdict::Invalid {
            reason:
                CriticReason::SchemaMismatch {
                    detail: SchemaFault::NotUtf8 { at_offset },
                    ..
                },
        } => assert_eq!(at_offset, 1),
        other => panic!("expected NotUtf8, got {other:?}"),
    }
}

#[test]
fn schema_json_valid_and_invalid() {
    assert_eq!(
        evaluate(&schema(SchemaTag::Json), br#"{"a":1,"b":[2,3]}"#),
        CriticVerdict::Valid
    );
    let v = evaluate(&schema(SchemaTag::Json), b"{not json");
    assert!(matches!(
        v,
        CriticVerdict::Invalid {
            reason: CriticReason::SchemaMismatch {
                detail: SchemaFault::NotJson { .. },
                ..
            }
        }
    ));
}

#[test]
fn schema_tensor_shape_mismatch() {
    // 2x3 f32 = 24 bytes expected.
    let tag = SchemaTag::Tensor {
        dtype: TensorDTypeTag::F32,
        shape: smallvec![2, 3],
    };
    assert_eq!(
        evaluate(&schema(tag.clone()), &[0u8; 24]),
        CriticVerdict::Valid
    );
    let v = evaluate(&schema(tag), &[0u8; 20]);
    match v {
        CriticVerdict::Invalid {
            reason:
                CriticReason::SchemaMismatch {
                    detail:
                        SchemaFault::ShapeMismatch {
                            expected_elems,
                            actual_bytes,
                        },
                    ..
                },
        } => {
            assert_eq!(expected_elems, 6);
            assert_eq!(actual_bytes, 20);
        }
        other => panic!("expected ShapeMismatch, got {other:?}"),
    }
}

#[test]
fn schema_vector_dim_bytes() {
    let tag = SchemaTag::Vector { dim: 4 }; // 4 * 4 = 16 bytes
    assert_eq!(
        evaluate(&schema(tag.clone()), &[0u8; 16]),
        CriticVerdict::Valid
    );
    assert!(matches!(
        evaluate(&schema(tag), &[0u8; 12]),
        CriticVerdict::Invalid { .. }
    ));
}

// --- dedup check ----------------------------------------------------------

fn dedup_lines(key_range: Option<(u32, u32)>) -> CheckSpec {
    CheckSpec::Dedup(DedupSpec {
        framing: RecordFraming::LinesLf,
        key_range,
    })
}

#[test]
fn dedup_no_duplicates_valid() {
    assert_eq!(
        evaluate(&dedup_lines(None), b"a\nb\nc\n"),
        CriticVerdict::Valid
    );
    assert_eq!(evaluate(&dedup_lines(None), b""), CriticVerdict::Valid);
}

#[test]
fn dedup_detects_first_duplicate_index() {
    // records: a(0) b(1) a(2) b(3) — first dup at index 2, count 2.
    let v = evaluate(&dedup_lines(None), b"a\nb\na\nb");
    match v {
        CriticVerdict::Invalid {
            reason:
                CriticReason::DuplicateDetected {
                    duplicate_count,
                    first_duplicate_index,
                },
        } => {
            assert_eq!(first_duplicate_index, 2);
            assert_eq!(duplicate_count, 2);
        }
        other => panic!("expected DuplicateDetected, got {other:?}"),
    }
}

#[test]
fn dedup_key_subrange() {
    // Key on bytes [0,2): "ab" rows collide on prefix even with distinct suffix.
    let v = evaluate(&dedup_lines(Some((0, 2))), b"abXX\nabYY\n");
    assert!(matches!(
        v,
        CriticVerdict::Invalid {
            reason: CriticReason::DuplicateDetected {
                first_duplicate_index: 1,
                ..
            }
        }
    ));
}

#[test]
fn dedup_length_prefixed_framing() {
    // Two length-prefixed records "aa","aa" → duplicate.
    let mut bytes = Vec::new();
    for _ in 0..2 {
        bytes.extend_from_slice(&2u32.to_le_bytes());
        bytes.extend_from_slice(b"aa");
    }
    let spec = CheckSpec::Dedup(DedupSpec {
        framing: RecordFraming::LengthPrefixedU32,
        key_range: None,
    });
    assert!(matches!(
        evaluate(&spec, &bytes),
        CriticVerdict::Invalid {
            reason: CriticReason::DuplicateDetected { .. }
        }
    ));
}

#[test]
fn dedup_malformed_framing_is_unparseable_not_panic() {
    // Truncated length prefix (only 2 bytes where 4 are needed).
    let spec = CheckSpec::Dedup(DedupSpec {
        framing: RecordFraming::LengthPrefixedU32,
        key_range: None,
    });
    let v = evaluate(&spec, &[0x05, 0x00]);
    assert!(matches!(
        v,
        CriticVerdict::Invalid {
            reason: CriticReason::Unparseable {
                check: kx_critic_types::CheckKind::Dedup,
                ..
            }
        }
    ));
}

// --- stat-bounds check ----------------------------------------------------

fn stat(stat: StatKind, lo: i64, hi: i64, field: Option<(u32, u32)>) -> CheckSpec {
    CheckSpec::StatBounds(StatBoundsSpec {
        framing: RecordFraming::LinesLf,
        stat,
        scale: 1,
        lo_scaled: lo,
        hi_scaled: hi,
        numeric_field_range: field,
    })
}

#[test]
fn statbounds_record_count_in_range() {
    assert_eq!(
        evaluate(&stat(StatKind::RecordCount, 1, 3, None), b"x\ny\n"),
        CriticVerdict::Valid
    );
    assert!(matches!(
        evaluate(&stat(StatKind::RecordCount, 0, 1, None), b"x\ny\nz\n"),
        CriticVerdict::Invalid {
            reason: CriticReason::StatOutOfBounds {
                observed_scaled: 3,
                ..
            }
        }
    ));
}

#[test]
fn statbounds_empty_input_count_zero() {
    assert_eq!(
        evaluate(&stat(StatKind::RecordCount, 0, 0, None), b""),
        CriticVerdict::Valid
    );
    // Mean of empty is defined as 0.
    assert_eq!(
        evaluate(&stat(StatKind::MeanScaled, 0, 0, None), b""),
        CriticVerdict::Valid
    );
}

#[test]
fn statbounds_mean_integer_truncation() {
    // values 1,2 → mean = 3/2 = 1 (truncated). In [1,1] → Valid.
    assert_eq!(
        evaluate(&stat(StatKind::MeanScaled, 1, 1, None), b"1\n2\n"),
        CriticVerdict::Valid
    );
    // values 1,2,3 → mean 2; bound [3,9] → Invalid observed 2.
    assert!(matches!(
        evaluate(&stat(StatKind::MeanScaled, 3, 9, None), b"1\n2\n3\n"),
        CriticVerdict::Invalid {
            reason: CriticReason::StatOutOfBounds {
                observed_scaled: 2,
                ..
            }
        }
    ));
}

#[test]
fn statbounds_min_max_and_negatives() {
    assert!(matches!(
        evaluate(&stat(StatKind::MinScaled, 0, 100, None), b"5\n-3\n10\n"),
        CriticVerdict::Invalid {
            reason: CriticReason::StatOutOfBounds {
                observed_scaled: -3,
                ..
            }
        }
    ));
    assert_eq!(
        evaluate(&stat(StatKind::MaxScaled, 0, 10, None), b"5\n-3\n10\n"),
        CriticVerdict::Valid
    );
}

#[test]
fn statbounds_non_numeric_is_unparseable() {
    let v = evaluate(&stat(StatKind::MeanScaled, 0, 10, None), b"1\nNaN\n");
    assert!(matches!(
        v,
        CriticVerdict::Invalid {
            reason: CriticReason::Unparseable {
                check: kx_critic_types::CheckKind::StatBounds,
                at_offset: 1
            }
        }
    ));
}

// --- PII check ------------------------------------------------------------

fn pii(classes: &[PiiClass]) -> CheckSpec {
    CheckSpec::PiiLeak(PiiSpec {
        forbidden: classes.iter().copied().collect::<BTreeSet<_>>(),
    })
}

#[test]
fn pii_email_match_offset() {
    let v = evaluate(&pii(&[PiiClass::Email]), b"contact me at a@b.com please");
    match v {
        CriticVerdict::Invalid {
            reason:
                CriticReason::PiiLeak {
                    class: PiiClass::Email,
                    match_offset,
                    ..
                },
        } => assert_eq!(match_offset, 14),
        other => panic!("expected Email leak, got {other:?}"),
    }
}

#[test]
fn pii_ipv4() {
    assert!(matches!(
        evaluate(&pii(&[PiiClass::IpV4]), b"host 192.168.0.1 down"),
        CriticVerdict::Invalid {
            reason: CriticReason::PiiLeak {
                class: PiiClass::IpV4,
                ..
            }
        }
    ));
    // 999.x is not a valid octet → no match.
    assert_eq!(
        evaluate(&pii(&[PiiClass::IpV4]), b"999.1.1.1"),
        CriticVerdict::Valid
    );
}

#[test]
fn pii_credit_card_luhn_true_and_false() {
    // 4111111111111111 is a canonical Luhn-valid test PAN.
    assert!(matches!(
        evaluate(
            &pii(&[PiiClass::CreditCardLuhn]),
            b"card 4111111111111111 end"
        ),
        CriticVerdict::Invalid {
            reason: CriticReason::PiiLeak {
                class: PiiClass::CreditCardLuhn,
                ..
            }
        }
    ));
    // Same length, fails Luhn.
    assert_eq!(
        evaluate(&pii(&[PiiClass::CreditCardLuhn]), b"4111111111111112"),
        CriticVerdict::Valid
    );
}

#[test]
fn pii_ssn() {
    assert!(matches!(
        evaluate(&pii(&[PiiClass::UsSsn]), b"ssn 123-45-6789"),
        CriticVerdict::Invalid {
            reason: CriticReason::PiiLeak {
                class: PiiClass::UsSsn,
                ..
            }
        }
    ));
}

#[test]
fn pii_no_match_valid() {
    assert_eq!(
        evaluate(
            &pii(&[PiiClass::Email, PiiClass::IpV4]),
            b"nothing sensitive here"
        ),
        CriticVerdict::Valid
    );
}

#[test]
fn pii_earliest_offset_wins_across_classes() {
    // SSN at offset 0, email later — SSN (earlier offset) wins.
    let input = b"123-45-6789 then a@b.com";
    let v = evaluate(&pii(&[PiiClass::Email, PiiClass::UsSsn]), input);
    assert!(matches!(
        v,
        CriticVerdict::Invalid {
            reason: CriticReason::PiiLeak {
                class: PiiClass::UsSsn,
                match_offset: 0,
                ..
            }
        }
    ));
}

// --- determinism + totality (adversarial sweep) ---------------------------

fn all_specs() -> Vec<CheckSpec> {
    vec![
        schema(SchemaTag::Json),
        schema(SchemaTag::Text),
        schema(SchemaTag::Tensor {
            dtype: TensorDTypeTag::F32,
            shape: smallvec![2, 2],
        }),
        dedup_lines(None),
        dedup_lines(Some((0, 3))),
        CheckSpec::Dedup(DedupSpec {
            framing: RecordFraming::LengthPrefixedU32,
            key_range: None,
        }),
        CheckSpec::Dedup(DedupSpec {
            framing: RecordFraming::FixedWidth { width: 4 },
            key_range: None,
        }),
        stat(StatKind::RecordCount, 0, 5, None),
        stat(StatKind::MeanScaled, -10, 10, None),
        pii(&[
            PiiClass::Email,
            PiiClass::IpV4,
            PiiClass::CreditCardLuhn,
            PiiClass::UsSsn,
        ]),
    ]
}

#[test]
fn adversarial_inputs_never_panic_and_are_deterministic() {
    let mut inputs: Vec<Vec<u8>> = vec![
        vec![],
        vec![0xFF; 64 * 1024],
        b"\n\n\n".to_vec(),
        vec![0x00, 0x00, 0x00], // truncated length prefix
        b"\xff\xfe non utf8 \x00".to_vec(),
        b"123".to_vec(),
    ];
    // A fixed pseudo-random buffer (no RNG — a deterministic LCG) to stress
    // the byte scanners without nondeterminism.
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut buf = Vec::with_capacity(4096);
    for _ in 0..4096 {
        x = x
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        buf.push((x >> 33) as u8);
    }
    inputs.push(buf);

    for spec in all_specs() {
        for input in &inputs {
            let a = evaluate(&spec, input);
            let b = evaluate(&spec, input);
            // Determinism: identical (spec, input) → byte-identical verdict.
            assert_eq!(
                a.encode(),
                b.encode(),
                "nondeterministic verdict for {spec:?}"
            );
        }
    }
}

// --- proptest -------------------------------------------------------------

fn arb_spec() -> impl Strategy<Value = CheckSpec> {
    prop_oneof![
        Just(schema(SchemaTag::Json)),
        Just(schema(SchemaTag::Text)),
        Just(schema(SchemaTag::Blob)),
        Just(dedup_lines(None)),
        Just(CheckSpec::Dedup(DedupSpec {
            framing: RecordFraming::LengthPrefixedU32,
            key_range: None,
        })),
        Just(CheckSpec::Dedup(DedupSpec {
            framing: RecordFraming::FixedWidth { width: 3 },
            key_range: Some((0, 2)),
        })),
        Just(stat(StatKind::RecordCount, 0, 4, None)),
        Just(stat(StatKind::MeanScaled, -5, 5, None)),
        Just(pii(&[PiiClass::Email, PiiClass::UsSsn])),
    ]
}

proptest! {
    #[test]
    fn prop_evaluate_total_no_panic(
        spec in arb_spec(),
        input in proptest::collection::vec(any::<u8>(), 0..1024),
    ) {
        // Total: returns a verdict for ALL inputs, never panics.
        let _ = evaluate(&spec, &input);
    }

    #[test]
    fn prop_evaluate_deterministic(
        spec in arb_spec(),
        input in proptest::collection::vec(any::<u8>(), 0..1024),
    ) {
        let a = evaluate(&spec, &input);
        let b = evaluate(&spec, &input);
        prop_assert_eq!(a.encode(), b.encode());
    }
}

// --- conversion -----------------------------------------------------------

#[test]
fn content_schema_converts_to_tag() {
    use kx_dataset::{ContentSchema, TensorDType};
    assert_eq!(crate::schema_tag_of(&ContentSchema::Json), SchemaTag::Json);
    assert_eq!(
        crate::schema_tag_of(&ContentSchema::Vector { dim: 8 }),
        SchemaTag::Vector { dim: 8 }
    );
    assert_eq!(
        crate::schema_tag_of(&ContentSchema::Tensor {
            dtype: TensorDType::I64,
            shape: smallvec![3],
        }),
        SchemaTag::Tensor {
            dtype: TensorDTypeTag::I64,
            shape: smallvec![3],
        }
    );
}
