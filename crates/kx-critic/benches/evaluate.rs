//! Throughput benchmark for `kx_critic::evaluate` across the four checks at
//! input sizes {1 KiB, 64 KiB, 1 MiB, 16 MiB}. Reports ns/iter with
//! `Throughput::Bytes` so the per-byte cost (and its ~linear scaling) is visible.
//!
//! Run: `cargo bench -p kx-critic`.

// criterion_group!/criterion_main! generate undocumented items + a `main`.
#![allow(missing_docs)]

use std::collections::BTreeSet;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use kx_critic::{
    evaluate, CheckSpec, DedupSpec, PiiClass, PiiSpec, RecordFraming, SchemaSpec, SchemaTag,
    StatBoundsSpec, StatKind,
};

const SIZES: &[usize] = &[1 << 10, 1 << 16, 1 << 20, 1 << 24];

/// A deterministic LCG buffer of ASCII-decimal LF-delimited records, reused so
/// every check sees realistic record-shaped bytes. No RNG crate (determinism).
fn record_buffer(target_len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(target_len + 16);
    let mut x: u64 = 0x1234_5678_9ABC_DEF0;
    while out.len() < target_len {
        x = x
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let v = (x >> 33) % 100_000;
        out.extend_from_slice(v.to_string().as_bytes());
        out.push(b'\n');
    }
    out.truncate(target_len);
    out
}

fn specs() -> Vec<(&'static str, CheckSpec)> {
    vec![
        (
            "schema_text",
            CheckSpec::Schema(SchemaSpec {
                expected: SchemaTag::Text,
            }),
        ),
        (
            "dedup_lines",
            CheckSpec::Dedup(DedupSpec {
                framing: RecordFraming::LinesLf,
                key_range: None,
            }),
        ),
        (
            "statbounds_mean",
            CheckSpec::StatBounds(StatBoundsSpec {
                framing: RecordFraming::LinesLf,
                stat: StatKind::MeanScaled,
                scale: 1,
                lo_scaled: i64::MIN,
                hi_scaled: i64::MAX,
                numeric_field_range: None,
            }),
        ),
        (
            "pii_all",
            CheckSpec::PiiLeak(PiiSpec {
                forbidden: [
                    PiiClass::Email,
                    PiiClass::IpV4,
                    PiiClass::CreditCardLuhn,
                    PiiClass::UsSsn,
                ]
                .into_iter()
                .collect::<BTreeSet<_>>(),
            }),
        ),
    ]
}

fn bench_evaluate(c: &mut Criterion) {
    let mut group = c.benchmark_group("evaluate");
    for &size in SIZES {
        let input = record_buffer(size);
        group.throughput(Throughput::Bytes(size as u64));
        for (name, spec) in specs() {
            group.bench_with_input(BenchmarkId::new(name, size), &input, |b, input| {
                b.iter(|| evaluate(&spec, std::hint::black_box(input)));
            });
        }
    }
    group.finish();
}

criterion_group!(benches, bench_evaluate);
criterion_main!(benches);
