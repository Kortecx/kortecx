// Integration-test file: compiled as a separate crate from the host lib;
// inherits the workspace `[lints]` deny on `unwrap_used` / `expect_used` but
// tests legitimately use `.unwrap()` for fixture construction. The `pedantic`
// group is allowed for the usual small-int-cast / helper-after-let friction.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]
//! M2.1 — incremental children-index (D92): differential equivalence,
//! recovery-parity, and sub-linear re-fold.
//!
//! The crate-internal `debug_assert!` in `State::reindex_child_edges` compares
//! the incrementally-maintained children index against a from-scratch
//! `rebuild_children_index` on **every** mutation, and it is active in every
//! debug test build. So the tests below — which drive the public
//! `register_mote` + `fold` surface over diverse, colliding traces — exercise
//! that equivalence oracle for free: any divergence between the incremental
//! path and the full rebuild panics the test. On top of it they assert the
//! public-API invariants the cascade walk and recovery depend on:
//!
//! 1. children lists stay **sorted by child `MoteId`** (the D22 poison-cascade
//!    BFS order),
//! 2. a pure-recovery re-fold is **deterministic** (re-folding the same
//!    committed entries yields a byte-identical index),
//! 3. the live (register-then-commit) path and the pure-recovery (fold-only)
//!    path build the **same committed-edge index**,
//! 4. (`#[ignore]`, `--release`) the re-fold is **sub-linear** — the D92
//!    resume-availability invariant: a super-linear resume is an outage.

use std::time::Instant;

use kx_content::ContentRef;
use kx_journal::{JournalEntry, ParentEntry};
use kx_mote::{EdgeMeta, EffectPattern, MoteDefHash, MoteId, NdClass, ParentRef};
use kx_projection::{Projection, RegisterMote};
use proptest::prelude::*;
use smallvec::SmallVec;

// ---------------------------------------------------------------------------
// Fixture helpers
// ---------------------------------------------------------------------------

fn mid(b: u8) -> MoteId {
    MoteId::from_bytes([b; 32])
}

/// A distinct `MoteId` per `u32` (the first 4 bytes carry `i`), for the
/// large-N scale test where a single byte would collide.
fn mid_n(i: u32) -> MoteId {
    let mut bytes = [0u8; 32];
    bytes[..4].copy_from_slice(&i.to_le_bytes());
    MoteId::from_bytes(bytes)
}

fn pref(id: MoteId, edge: EdgeMeta) -> ParentRef {
    ParentRef {
        parent_id: id,
        edge,
    }
}

fn register(id: MoteId, parents: &[ParentRef]) -> RegisterMote {
    RegisterMote {
        mote_id: id,
        nd_class: NdClass::Pure,
        effect_pattern: EffectPattern::IdempotentByConstruction,
        critic_for: None,
        is_topology_shaper: false,
        parents: parents.iter().copied().collect(),
        warrant_ref: ContentRef::from_bytes([0xaa; 32]),
    }
}

fn committed_with(id: MoteId, seq: u64, parents: &[ParentRef]) -> JournalEntry {
    let pe: SmallVec<[ParentEntry; 4]> = parents.iter().map(ParentEntry::from_parent_ref).collect();
    JournalEntry::Committed {
        mote_id: id,
        idempotency_key: *id.as_bytes(),
        seq,
        nondeterminism: NdClass::Pure,
        result_ref: ContentRef::from_bytes([7u8; 32]),
        parents: pe,
        warrant_ref: ContentRef::from_bytes([0xaa; 32]),
        mote_def_hash: MoteDefHash::from_bytes([1u8; 32]),
    }
}

// ---------------------------------------------------------------------------
// Targeted cases — the requirement matrix from the M2.1 plan.
// ---------------------------------------------------------------------------

#[test]
fn declared_then_committed_no_duplicate() {
    let mut p = Projection::new();
    p.register_mote(register(mid(10), &[pref(mid(1), EdgeMeta::data())]));
    p.fold(&committed_with(
        mid(10),
        1,
        &[pref(mid(1), EdgeMeta::data())],
    ))
    .unwrap();
    assert_eq!(
        p.children_of(&mid(1)),
        vec![(mid(10), EdgeMeta::data())],
        "declared-then-committed must not duplicate the edge"
    );
}

#[test]
fn committed_without_declare_recovery_shape() {
    // Pure-recovery shape: a Committed folded with no prior register (the
    // `from_journal` path never calls register_mote).
    let mut p = Projection::new();
    p.fold(&committed_with(
        mid(10),
        1,
        &[pref(mid(1), EdgeMeta::data())],
    ))
    .unwrap();
    assert_eq!(p.children_of(&mid(1)), vec![(mid(10), EdgeMeta::data())]);
}

#[test]
fn re_registration_changing_parents_removes_stale_edge() {
    let mut p = Projection::new();
    p.register_mote(register(mid(10), &[pref(mid(1), EdgeMeta::data())]));
    assert_eq!(p.children_of(&mid(1)), vec![(mid(10), EdgeMeta::data())]);
    // Re-register child 10 with a DIFFERENT parent.
    p.register_mote(register(mid(10), &[pref(mid(2), EdgeMeta::data())]));
    assert!(
        p.children_of(&mid(1)).is_empty(),
        "stale edge under the dropped parent must be removed"
    );
    assert_eq!(p.children_of(&mid(2)), vec![(mid(10), EdgeMeta::data())]);
}

#[test]
fn duplicate_parent_two_edges_kept_stable() {
    // The ONLY equal-child case: a child declaring the same parent twice with
    // different edge meta (a Data and a Control edge). The full rebuild keeps
    // both (push + stable sort), so the incremental path must too.
    let mut p = Projection::new();
    p.register_mote(register(
        mid(10),
        &[
            pref(mid(1), EdgeMeta::data()),
            pref(mid(1), EdgeMeta::control()),
        ],
    ));
    assert_eq!(
        p.children_of(&mid(1)),
        vec![(mid(10), EdgeMeta::data()), (mid(10), EdgeMeta::control()),],
        "both edges kept, in parents-list order (stable)"
    );
}

#[test]
fn live_and_recovery_paths_build_identical_index() {
    // Build a small DAG two ways and compare children_of for every parent:
    //   LIVE:     register each child (declared parents) then fold its Committed.
    //   RECOVERY: fold only the Committed entries (parents_in_entry).
    let edges: &[(u8, &[(u8, EdgeMeta)])] = &[
        (1, &[]),
        (2, &[(1, EdgeMeta::data())]),
        (3, &[(1, EdgeMeta::control())]),
        (4, &[(2, EdgeMeta::data()), (3, EdgeMeta::data())]),
    ];
    let mut live = Projection::new();
    let mut rec = Projection::new();
    for (seq, (child, ps)) in (1u64..).zip(edges.iter()) {
        let prefs: Vec<ParentRef> = ps.iter().map(|(pp, e)| pref(mid(*pp), *e)).collect();
        live.register_mote(register(mid(*child), &prefs));
        live.fold(&committed_with(mid(*child), seq, &prefs))
            .unwrap();
        rec.fold(&committed_with(mid(*child), seq, &prefs)).unwrap();
    }
    for parent in [1u8, 2, 3, 4] {
        assert_eq!(
            live.children_of(&mid(parent)),
            rec.children_of(&mid(parent)),
            "live vs recovery children_of disagree for parent {parent}"
        );
    }
}

// ---------------------------------------------------------------------------
// Property test — diverse traces drive the internal differential oracle.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum Op {
    Register { child: u8, parents: Vec<ParentRef> },
    Commit { child: u8, parents: Vec<ParentRef> },
}

fn arb_parent() -> impl Strategy<Value = ParentRef> {
    (0u8..8, 0u8..3).prop_map(|(pid, kind)| {
        let edge = match kind {
            0 => EdgeMeta::data(),
            1 => EdgeMeta::control(),
            _ => EdgeMeta::control_non_cascading(),
        };
        pref(mid(pid), edge)
    })
}

fn arb_op() -> impl Strategy<Value = Op> {
    prop_oneof![
        (8u8..16, prop::collection::vec(arb_parent(), 0..3))
            .prop_map(|(child, parents)| Op::Register { child, parents }),
        (8u8..16, prop::collection::vec(arb_parent(), 0..3))
            .prop_map(|(child, parents)| Op::Commit { child, parents }),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

    /// Drive arbitrary {register, re-register-with-changed-parents,
    /// commit-with-declare, commit-without-declare} traces. The internal
    /// `debug_assert!` (incremental == full rebuild) fires on every op; on top
    /// of it we assert children lists stay sorted and recovery is deterministic.
    #[test]
    fn prop_index_sorted_and_recovery_deterministic(ops in prop::collection::vec(arb_op(), 0..40)) {
        // child ids live in 8..16, parents in 0..8 — disjoint, so a child is
        // never its own parent and parent/child collide across motes.
        let mut p = Projection::new();
        let mut committed: Vec<JournalEntry> = Vec::new();
        let mut committed_ids: std::collections::BTreeSet<u8> = std::collections::BTreeSet::new();
        let mut seq = 1u64;
        for op in &ops {
            match op {
                Op::Register { child, parents } => {
                    p.register_mote(register(mid(*child), parents));
                }
                Op::Commit { child, parents } => {
                    // At most one Committed per id (a second is a journal-impl
                    // bug — DuplicateCommitted — not the index's concern).
                    if committed_ids.insert(*child) {
                        let e = committed_with(mid(*child), seq, parents);
                        p.fold(&e).unwrap();
                        committed.push(e);
                        seq += 1;
                    }
                }
            }
        }

        // INVARIANT 1: every children_of list is sorted by child MoteId.
        for parent in 0u8..16 {
            let kids = p.children_of(&mid(parent));
            let ids: Vec<MoteId> = kids.iter().map(|(c, _)| *c).collect();
            let mut sorted = ids.clone();
            sorted.sort_unstable();
            prop_assert_eq!(ids, sorted, "children_of must be sorted by child MoteId for parent {}", parent);
        }

        // INVARIANT 2: a pure-recovery re-fold of the committed entries is
        // deterministic (byte-identical index across two independent folds).
        let mut rec_a = Projection::new();
        let mut rec_b = Projection::new();
        for e in &committed { rec_a.fold(e).unwrap(); }
        for e in &committed { rec_b.fold(e).unwrap(); }
        for parent in 0u8..16 {
            prop_assert_eq!(
                rec_a.children_of(&mid(parent)),
                rec_b.children_of(&mid(parent)),
                "recovery re-fold must be deterministic for parent {}", parent
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Scale test (`#[ignore]`) — the D92 resume-availability gate.
//   cargo test -p kx-projection --release --test incremental_children_index \
//     -- --ignored --nocapture --test-threads=1
// MUST run `--release`: in a debug build the differential `debug_assert!`
// re-imposes the O(n^2) full rebuild on every fold, so the ratio assertion is
// skipped (and only printed) under `debug_assertions`.
// ---------------------------------------------------------------------------

#[test]
#[ignore = "scale: run --release --test incremental_children_index -- --ignored --nocapture"]
fn scale_refold_is_sub_linear() {
    const SIZES: &[u32] = &[1_000, 5_000, 10_000, 25_000];
    let mut per_mote_us: Vec<f64> = Vec::with_capacity(SIZES.len());

    for &n in SIZES {
        // A wide-but-HUBLESS DAG: mote i depends on i-1 (Data) and, for i>=2,
        // also i-2 (Control). Bounded fan-in (<=2) and fan-out (<=2) — no hub,
        // so each children-index insert is O(1) and a correct incremental fold
        // is O(n). (A hub/star is the known residual addressed by Option B in a
        // follow-up; this test pins the common-case linear scaling.)
        let mut entries: Vec<JournalEntry> = Vec::with_capacity(n as usize);
        for i in 0..n {
            let mut ps: Vec<ParentRef> = Vec::new();
            if i >= 1 {
                ps.push(pref(mid_n(i - 1), EdgeMeta::data()));
            }
            if i >= 2 {
                ps.push(pref(mid_n(i - 2), EdgeMeta::control()));
            }
            entries.push(committed_with(mid_n(i), u64::from(i) + 1, &ps));
        }

        let start = Instant::now();
        let mut p = Projection::new();
        for e in &entries {
            p.fold(e).unwrap();
        }
        let elapsed = start.elapsed();
        assert_eq!(p.committed_count(), n as usize, "all motes committed");

        let us = elapsed.as_secs_f64() * 1e6;
        let per = us / f64::from(n);
        per_mote_us.push(per);
        eprintln!(
            "n={n:>6}  fold={:>9.2}ms  per_mote={per:>7.3}us",
            us / 1000.0
        );
    }

    let ratio = per_mote_us.last().unwrap() / per_mote_us.first().unwrap();
    eprintln!("per-Mote re-fold cost ratio (25k/1k) = {ratio:.2}  (quadratic would be ~25x)");

    if cfg!(debug_assertions) {
        eprintln!(
            "NOTE: debug build — the differential oracle makes the fold O(n^2); \
             ratio assertion skipped. Re-run with --release for the real gate."
        );
    } else {
        // Per-Mote cost must stay ~flat (linear total). A quadratic fold makes
        // per-Mote grow ~25x from 1k->25k; allow a generous 8x for the log
        // factor + cache effects and still catch quadratic by a wide margin.
        assert!(
            ratio < 8.0,
            "re-fold per-Mote cost grew {ratio:.1}x (1k->25k) — super-linear; \
             the D92 resume-availability invariant is violated"
        );
    }
}
