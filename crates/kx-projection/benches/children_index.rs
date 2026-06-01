//! Children-index throughput — the M2.1 incremental-index hot paths (run with
//! `cargo bench -p kx-projection`).
//!
//! Two axes, both on a **hubless** chain DAG (bounded fan-in/out) so the
//! measurement reflects the per-op incremental cost rather than a degenerate
//! star:
//! - `fold_committed` — the recovery re-fold: N `Committed` entries folded into
//!   a fresh `Projection` (the D92 resume path);
//! - `register` — N `register_mote` declarations (the live submission path).
//!
//! Before M2.1 each fold/register ran a full O(n) `rebuild_children_index`, so
//! the whole loop was O(n^2); the incremental update makes it O(n). This bench
//! is the criterion regression guard behind `bench-no-regress` (D95).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::pedantic,
    missing_docs
)]

use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion, Throughput};
use kx_content::ContentRef;
use kx_journal::{JournalEntry, ParentEntry};
use kx_mote::{EdgeMeta, EffectPattern, MoteDefHash, MoteId, NdClass, ParentRef};
use kx_projection::{Projection, RegisterMote};
use smallvec::SmallVec;

fn mid_n(i: u32) -> MoteId {
    let mut bytes = [0u8; 32];
    bytes[..4].copy_from_slice(&i.to_le_bytes());
    MoteId::from_bytes(bytes)
}

/// Hubless adjacency: mote i depends on i-1 (Data) and i-2 (Control) — fan-out
/// bounded at 2, so children-index inserts stay O(1).
fn parents_of(i: u32) -> Vec<ParentRef> {
    let mut ps = Vec::new();
    if i >= 1 {
        ps.push(ParentRef {
            parent_id: mid_n(i - 1),
            edge: EdgeMeta::data(),
        });
    }
    if i >= 2 {
        ps.push(ParentRef {
            parent_id: mid_n(i - 2),
            edge: EdgeMeta::control(),
        });
    }
    ps
}

fn committed(i: u32) -> JournalEntry {
    let pe: SmallVec<[ParentEntry; 4]> = parents_of(i)
        .iter()
        .map(ParentEntry::from_parent_ref)
        .collect();
    JournalEntry::Committed {
        mote_id: mid_n(i),
        idempotency_key: *mid_n(i).as_bytes(),
        seq: u64::from(i) + 1,
        nondeterminism: NdClass::Pure,
        result_ref: ContentRef::from_bytes([7u8; 32]),
        parents: pe,
        warrant_ref: ContentRef::from_bytes([0xaa; 32]),
        mote_def_hash: MoteDefHash::from_bytes([1u8; 32]),
    }
}

fn register(i: u32) -> RegisterMote {
    RegisterMote {
        mote_id: mid_n(i),
        nd_class: NdClass::Pure,
        effect_pattern: EffectPattern::IdempotentByConstruction,
        critic_for: None,
        is_topology_shaper: false,
        parents: parents_of(i).into_iter().collect(),
        warrant_ref: ContentRef::from_bytes([0xaa; 32]),
    }
}

fn bench_children_index(c: &mut Criterion) {
    let mut group = c.benchmark_group("children_index");
    for &n in &[1_000u32, 5_000, 10_000] {
        group.throughput(Throughput::Elements(u64::from(n)));

        group.bench_with_input(BenchmarkId::new("fold_committed", n), &n, |b, &n| {
            let entries: Vec<JournalEntry> = (0..n).map(committed).collect();
            b.iter_batched(
                Projection::new,
                |mut p| {
                    for e in &entries {
                        p.fold(e).unwrap();
                    }
                    p
                },
                BatchSize::SmallInput,
            );
        });

        group.bench_with_input(BenchmarkId::new("register", n), &n, |b, &n| {
            let regs: Vec<RegisterMote> = (0..n).map(register).collect();
            b.iter_batched(
                Projection::new,
                |mut p| {
                    for r in &regs {
                        p.register_mote(r.clone());
                    }
                    p
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

criterion_group!(benches, bench_children_index);
criterion_main!(benches);
