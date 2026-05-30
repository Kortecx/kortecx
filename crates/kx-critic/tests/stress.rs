//! Scale + reproducibility stress for `kx_critic::evaluate`.
//!
//! `#[ignore]` (release-mode, opt-in like the kx-workflow stress suite): run via
//! `cargo test -p kx-critic --release -- --ignored`. Asserts the two properties
//! that matter at scale: (1) the evaluator stays total + fast as record counts
//! grow across a scaling table, and (2) verdicts are byte-identical across two
//! independent evaluations of the same input (the SN-8 reproducibility guarantee
//! the runtime relies on — identical input ⇒ identical committed verdict ref).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeSet;

use kx_critic::{
    evaluate, CheckSpec, CriticVerdict, DedupSpec, PiiClass, PiiSpec, RecordFraming, SchemaSpec,
    SchemaTag, StatBoundsSpec, StatKind,
};

/// Record counts to scale through (mirrors the kx-workflow stress `POINTS` table).
const POINTS: &[usize] = &[1_000, 10_000, 100_000, 1_000_000];

/// Build `n` LF-delimited ASCII-decimal records deterministically (no RNG).
fn records(n: usize, unique: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(n * 8);
    let mut x: u64 = 0xDEAD_BEEF_CAFE_F00D;
    for i in 0..n {
        let v = if unique {
            i as u64
        } else {
            // Force collisions: a small value domain so duplicates appear early.
            x = x.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            (x >> 40) % 16
        };
        out.extend_from_slice(v.to_string().as_bytes());
        out.push(b'\n');
    }
    out
}

fn specs() -> Vec<CheckSpec> {
    vec![
        CheckSpec::Schema(SchemaSpec {
            expected: SchemaTag::Text,
        }),
        CheckSpec::Dedup(DedupSpec {
            framing: RecordFraming::LinesLf,
            key_range: None,
        }),
        CheckSpec::StatBounds(StatBoundsSpec {
            framing: RecordFraming::LinesLf,
            stat: StatKind::MeanScaled,
            scale: 1,
            lo_scaled: i64::MIN,
            hi_scaled: i64::MAX,
            numeric_field_range: None,
        }),
        CheckSpec::PiiLeak(PiiSpec {
            forbidden: [PiiClass::Email, PiiClass::UsSsn]
                .into_iter()
                .collect::<BTreeSet<_>>(),
        }),
    ]
}

#[test]
#[ignore = "scale stress; run with --release -- --ignored"]
fn evaluate_scales_and_is_reproducible() {
    for &n in POINTS {
        let unique_input = records(n, true);
        let dup_input = records(n, false);
        for spec in specs() {
            for input in [&unique_input, &dup_input] {
                // Reproducibility: two independent evaluations agree byte-for-byte.
                let v1 = evaluate(&spec, input);
                let v2 = evaluate(&spec, input);
                assert_eq!(
                    v1.content_ref_bytes(),
                    v2.content_ref_bytes(),
                    "verdict ref drifted at n={n} for {spec:?}"
                );
                // Totality at scale: a verdict is always produced.
                let _ = matches!(v1, CriticVerdict::Valid | CriticVerdict::Invalid { .. });
            }
        }
    }
}

#[test]
#[ignore = "scale stress; run with --release -- --ignored"]
fn dedup_detects_collision_at_scale() {
    // A duplicate-heavy 1M-record stream must be rejected, deterministically.
    let input = records(1_000_000, false);
    let spec = CheckSpec::Dedup(DedupSpec {
        framing: RecordFraming::LinesLf,
        key_range: None,
    });
    let v = evaluate(&spec, &input);
    assert!(
        matches!(
            v,
            CriticVerdict::Invalid {
                reason: kx_critic::CriticReason::DuplicateDetected { .. }
            }
        ),
        "expected a duplicate detection on a 16-value-domain 1M stream"
    );
}
