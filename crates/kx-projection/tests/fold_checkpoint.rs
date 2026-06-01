// Integration-test file: compiled as a separate crate from the host lib;
// inherits the workspace `[lints]` deny on `unwrap_used` / `expect_used` but
// tests legitimately `.unwrap()` for fixture construction. `pedantic` is allowed
// for the usual small-int-cast / helper-after-let friction.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]
//! M2.2 — discardable `FoldCheckpoint` (D92(b)): resume-from-checkpoint
//! equivalence, fail-safe fallback, durable-byte round-trip, determinism, and
//! the materializer no-refire property.
//!
//! The load-bearing theorem is fold associativity:
//! `fold(0,N] == fold(K,N] ∘ fold(0,K]`. The checkpoint stores `fold(0,K]`; a
//! resume folds the tail on top and MUST reproduce the full fold **for every
//! offset K**. The checkpoint is never authoritative — every corruption /
//! staleness / wrong-run path falls back to a full fold and still matches.

use std::sync::{Arc, Mutex};

use kx_content::ContentRef;
use kx_journal::{
    repudiation_idempotency_key, FailureReason, InMemoryJournal, Journal, JournalEntry,
    ParentEntry, RepudiationReason, SqliteJournal, INSTANCE_ID_LEN,
};
use kx_mote::{EdgeMeta, EffectPattern, MoteDefHash, MoteId, NdClass, ParentRef};
use kx_projection::{
    CheckpointError, CheckpointOutcome, FoldCheckpoint, FullFoldReason, Projection,
    ProjectionError, RegisterMote, TopologyMaterializer,
};
use proptest::prelude::*;
use smallvec::SmallVec;

// ---------------------------------------------------------------------------
// Fixture helpers
// ---------------------------------------------------------------------------

fn mid(b: u8) -> MoteId {
    MoteId::from_bytes([b; 32])
}

/// A distinct idempotency key per call-counter, so appended entries never dedup
/// (the journal would collapse duplicate (key,kind) pairs and skip a `seq`).
fn ukey(n: u64) -> [u8; 32] {
    let mut k = [0xEEu8; 32];
    k[..8].copy_from_slice(&n.to_le_bytes());
    k
}

fn war() -> ContentRef {
    ContentRef::from_bytes([0xaa; 32])
}

fn pref(id: MoteId, edge: EdgeMeta) -> ParentRef {
    ParentRef {
        parent_id: id,
        edge,
    }
}

fn committed_entry(m: u8, parents: &[ParentRef]) -> JournalEntry {
    let pe: SmallVec<[ParentEntry; 4]> = parents.iter().map(ParentEntry::from_parent_ref).collect();
    JournalEntry::Committed {
        mote_id: mid(m),
        idempotency_key: *mid(m).as_bytes(),
        seq: 0,
        nondeterminism: NdClass::Pure,
        result_ref: ContentRef::from_bytes([7u8; 32]),
        parents: pe,
        warrant_ref: war(),
        mote_def_hash: MoteDefHash::from_bytes([1u8; 32]),
    }
}

/// Fold the journal prefix `[1, k]` into a fresh (materializer-less) projection —
/// the contiguous-prefix precondition `fold_checkpoint` requires.
fn seed_through(journal: &InMemoryJournal, k: u64) -> Projection {
    let mut p = Projection::new();
    for e in journal.read_entries_by_seq(0..(k + 1)).unwrap() {
        p.fold(&e).unwrap();
    }
    p
}

/// Strong structural equality of two projections via the full-state digest, plus
/// a public-API spot-check (every known Mote's state + the seq frontier).
fn assert_projection_eq(a: &Projection, b: &Projection, ctx: &str) {
    assert_eq!(
        a.state_digest(),
        b.state_digest(),
        "{ctx}: full-state digest mismatch"
    );
    assert_eq!(
        a.current_seq(),
        b.current_seq(),
        "{ctx}: seq frontier mismatch"
    );
    let am: Vec<_> = a.iter_motes().collect();
    let bm: Vec<_> = b.iter_motes().collect();
    assert_eq!(am, bm, "{ctx}: (MoteId, MoteState) sets differ");
}

// ---------------------------------------------------------------------------
// Proptest generator — arbitrary journal-entry traces (no register_mote, so the
// comparison target is `from_journal`, which never replays declarations).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum TraceOp {
    Propose(u8),
    Commit(u8),
    Fail(u8, bool), // (mote, terminal?)
    Stage(u8),
    Repudiate(u8),
}

fn arb_op() -> impl Strategy<Value = TraceOp> {
    prop_oneof![
        (0u8..10).prop_map(TraceOp::Propose),
        (0u8..10).prop_map(TraceOp::Commit),
        (0u8..10, any::<bool>()).prop_map(|(m, t)| TraceOp::Fail(m, t)),
        (0u8..10).prop_map(TraceOp::Stage),
        (0u8..10).prop_map(TraceOp::Repudiate),
    ]
}

/// Interpret an op trace against a live `InMemoryJournal`, which assigns dense
/// monotonic seqs. Returns the populated journal. Commits are deduped per Mote
/// (a second is a journal bug, not the checkpoint's concern); Repudiate only
/// fires against an already-committed, not-yet-repudiated target.
fn build_journal(ops: &[TraceOp]) -> InMemoryJournal {
    let journal = InMemoryJournal::new();
    let mut committed: std::collections::BTreeMap<u8, u64> = std::collections::BTreeMap::new();
    let mut repudiated: std::collections::BTreeSet<u8> = std::collections::BTreeSet::new();
    let mut ctr = 0u64;
    for op in ops {
        ctr += 1;
        match op {
            TraceOp::Propose(m) => {
                journal
                    .append(JournalEntry::Proposed {
                        mote_id: mid(*m),
                        idempotency_key: ukey(ctr),
                        seq: 0,
                        nondeterminism: NdClass::Pure,
                        placement_hint: 0,
                        warrant_ref: war(),
                    })
                    .unwrap();
            }
            TraceOp::Commit(m) => {
                if !committed.contains_key(m) {
                    let parents = if *m > 0 {
                        vec![pref(mid(m - 1), EdgeMeta::data())]
                    } else {
                        vec![]
                    };
                    let r = journal.append(committed_entry(*m, &parents)).unwrap();
                    committed.insert(*m, r.seq());
                }
            }
            TraceOp::Fail(m, terminal) => {
                let reason = if *terminal {
                    FailureReason::ExecutorRefused
                } else {
                    FailureReason::TimedOut
                };
                journal
                    .append(JournalEntry::Failed {
                        mote_id: mid(*m),
                        idempotency_key: ukey(ctr),
                        seq: 0,
                        reason_class: reason,
                        reporter_id: 0,
                    })
                    .unwrap();
            }
            TraceOp::Stage(m) => {
                journal
                    .append(JournalEntry::EffectStaged {
                        mote_id: mid(*m),
                        idempotency_key: ukey(ctr),
                        seq: 0,
                    })
                    .unwrap();
            }
            TraceOp::Repudiate(m) => {
                if let Some(cs) = committed.get(m) {
                    if repudiated.insert(*m) {
                        journal
                            .append(JournalEntry::Repudiated {
                                target_mote_id: mid(*m),
                                idempotency_key: repudiation_idempotency_key(&mid(*m), *cs),
                                seq: 0,
                                target_committed_seq: *cs,
                                reason_class: RepudiationReason::OperatorAction,
                                repudiator_id: 0,
                            })
                            .unwrap();
                    }
                }
            }
        }
    }
    journal
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 192, ..ProptestConfig::default() })]

    /// **The DoD line: cold-fold-from-offset == cold-fold-from-zero.** For an
    /// arbitrary trace, resume from a checkpoint taken at EVERY offset K in
    /// `[0, last_seq]` and assert the resumed projection equals the full fold.
    #[test]
    fn prop_resume_from_every_offset(ops in prop::collection::vec(arb_op(), 0..16)) {
        let journal = build_journal(&ops);
        let n = journal.current_seq().unwrap();
        let full = Projection::from_journal(&journal).unwrap();

        for k in 0..=n {
            let cp = seed_through(&journal, k).fold_checkpoint();
            prop_assert_eq!(cp.journal_offset(), k, "checkpoint offset must equal the prefix frontier");
            // Round-trip through the durable bytes too — the real recovery path.
            let cp = FoldCheckpoint::from_bytes(&cp.to_bytes()).unwrap();
            let resumed = Projection::from_journal_with_checkpoint(&journal, Some(&cp)).unwrap();
            prop_assert_eq!(
                resumed.state_digest(), full.state_digest(),
                "resume from offset {} diverged from the full fold", k
            );
            prop_assert_eq!(resumed.current_seq(), full.current_seq());
        }
    }

    /// **Discardability:** `to_bytes -> from_bytes -> resume (empty tail)` equals
    /// the source projection (drop/recreate the checkpoint -> identical state).
    #[test]
    fn prop_roundtrip_is_lossless(ops in prop::collection::vec(arb_op(), 0..16)) {
        let journal = build_journal(&ops);
        let full = Projection::from_journal(&journal).unwrap();
        let cp = FoldCheckpoint::from_bytes(&full.fold_checkpoint().to_bytes()).unwrap();
        // Offset == head, so the tail is empty; the resume is pure decode.
        let resumed = Projection::from_journal_with_checkpoint(&journal, Some(&cp)).unwrap();
        prop_assert_eq!(resumed.state_digest(), full.state_digest());
    }

    /// **Re-fold + canonical-encoding determinism:** two independent journals
    /// built from the same trace produce byte-identical checkpoints.
    #[test]
    fn prop_bytes_are_deterministic(ops in prop::collection::vec(arb_op(), 0..16)) {
        let a = Projection::from_journal(&build_journal(&ops)).unwrap().fold_checkpoint().to_bytes();
        let b = Projection::from_journal(&build_journal(&ops)).unwrap().fold_checkpoint().to_bytes();
        prop_assert_eq!(a, b, "checkpoint bytes must be deterministic for an identical trace");
    }

    /// **Fail-safe:** any single-byte flip OR truncation of the durable blob is a
    /// graceful `Err` (digest / length check), never a panic, never silent trust.
    #[test]
    fn prop_corrupt_or_truncated_fails_safe(
        ops in prop::collection::vec(arb_op(), 1..12),
        flip_idx in any::<usize>(),
        flip_mask in 1u8..=255,
        trunc in any::<usize>(),
    ) {
        let journal = build_journal(&ops);
        let bytes = Projection::from_journal(&journal).unwrap().fold_checkpoint().to_bytes();

        // A flipped byte must be rejected (no panic).
        let mut flipped = bytes.clone();
        let i = flip_idx % flipped.len();
        flipped[i] ^= flip_mask;
        prop_assert!(
            FoldCheckpoint::from_bytes(&flipped).is_err(),
            "a corrupted checkpoint must be rejected, not trusted"
        );

        // A truncation (any length < full) must be rejected (no panic).
        let cut = trunc % bytes.len();
        prop_assert!(FoldCheckpoint::from_bytes(&bytes[..cut]).is_err());
    }
}

// ---------------------------------------------------------------------------
// Deterministic edge-case tests
// ---------------------------------------------------------------------------

/// A trace deliberately hitting committed / repudiated / failed-terminal /
/// failed-pending / proposed / effect-staged cells, checkpointed at every
/// offset — a fixed counterpart to the proptest's breadth.
#[test]
fn resume_equivalence_over_mixed_cells() {
    let ops = vec![
        TraceOp::Propose(1),
        TraceOp::Commit(1),
        TraceOp::Stage(2),
        TraceOp::Commit(2),
        TraceOp::Fail(3, false), // pre-commit (retry permitted)
        TraceOp::Fail(4, true),  // terminal
        TraceOp::Commit(5),
        TraceOp::Repudiate(5),
        TraceOp::Propose(6),
    ];
    let journal = build_journal(&ops);
    let n = journal.current_seq().unwrap();
    let full = Projection::from_journal(&journal).unwrap();
    for k in 0..=n {
        let cp = seed_through(&journal, k).fold_checkpoint();
        let resumed = Projection::from_journal_with_checkpoint(&journal, Some(&cp)).unwrap();
        assert_projection_eq(&resumed, &full, &format!("offset {k}"));
    }
}

/// `None` checkpoint and an empty journal both resolve to the full fold.
#[test]
fn none_checkpoint_is_full_fold() {
    let journal = build_journal(&[TraceOp::Commit(1), TraceOp::Commit(2)]);
    let full = Projection::from_journal(&journal).unwrap();
    let resumed = Projection::from_journal_with_checkpoint(&journal, None).unwrap();
    assert_projection_eq(&resumed, &full, "None checkpoint");
}

/// **Fail-safe fallback:** a checkpoint whose offset is past the (truncated)
/// journal head is discarded; recovery silently full-folds the short journal.
#[test]
fn stale_offset_falls_back_to_full_fold() {
    // A 5-entry journal -> a checkpoint at offset 5.
    let long = build_journal(&[
        TraceOp::Commit(1),
        TraceOp::Commit(2),
        TraceOp::Commit(3),
        TraceOp::Commit(4),
        TraceOp::Commit(5),
    ]);
    let cp = Projection::from_journal(&long).unwrap().fold_checkpoint();
    assert_eq!(cp.journal_offset(), 5);

    // A different, SHORTER journal (head = 3) — the stale checkpoint must not seed it.
    let short = build_journal(&[TraceOp::Commit(1), TraceOp::Commit(2), TraceOp::Commit(3)]);
    let resumed = Projection::from_journal_with_checkpoint(&short, Some(&cp)).unwrap();
    let full_short = Projection::from_journal(&short).unwrap();
    assert_projection_eq(&resumed, &full_short, "stale-offset fallback");
}

/// **Wrong-run guard:** a checkpoint carrying run A's `instance_id` must not seed
/// a journal registered under run B; recovery falls back to the full fold.
#[test]
fn wrong_run_checkpoint_falls_back() {
    fn run_journal(instance: u8) -> InMemoryJournal {
        let j = InMemoryJournal::new();
        j.append(JournalEntry::RunRegistered {
            instance_id: [instance; INSTANCE_ID_LEN],
            recipe_fingerprint: [0xab; 32],
            ts: 0,
            seq: 0,
        })
        .unwrap();
        j.append(committed_entry(1, &[])).unwrap();
        j
    }
    let run_a = run_journal(0xAA);
    let cp_a = Projection::from_journal(&run_a).unwrap().fold_checkpoint();

    let run_b = run_journal(0xBB);
    let resumed = Projection::from_journal_with_checkpoint(&run_b, Some(&cp_a)).unwrap();
    let full_b = Projection::from_journal(&run_b).unwrap();
    assert_projection_eq(&resumed, &full_b, "wrong-run fallback");
}

// ---------------------------------------------------------------------------
// CheckpointOutcome — the structured, testable recovery reason (M2.2b).
// The folded state is bit-identical regardless; these pin the *reported reason*
// so the runtime's recovery observability (and operators) can distinguish a
// happy resume from each discard cause.
// ---------------------------------------------------------------------------

/// A happy resume reports `Seeded { offset, tail_entries }` with the exact tail
/// length, and the seeded projection equals the full fold.
#[test]
fn reported_happy_resume_is_seeded_with_tail_len() {
    // Build a journal that interleaves the M2.2c digest seal at the checkpoint
    // frontier (exactly as the live runtime does): Commit(1), Commit(2), then the
    // seal anchoring frontier 2, then Commit(3..5). The seal lands at seq 3, so
    // the tail (2, 6] is four entries (the seal + three commits).
    let journal = InMemoryJournal::new();
    journal.append(committed_entry(1, &[])).unwrap();
    journal.append(committed_entry(2, &[])).unwrap();
    let seed_digest = seed_through(&journal, 2).state_digest();
    journal
        .append(JournalEntry::DigestSealed {
            through_seq: 2,
            state_digest: seed_digest,
            seq: 0,
        })
        .unwrap();
    journal.append(committed_entry(3, &[])).unwrap();
    journal.append(committed_entry(4, &[])).unwrap();
    journal.append(committed_entry(5, &[])).unwrap();

    // Seed at offset 2; the tail (2, 6] is four entries (seal + three commits).
    let cp = seed_through(&journal, 2).fold_checkpoint();
    assert_eq!(cp.journal_offset(), 2);
    let (resumed, outcome) =
        Projection::from_journal_with_checkpoint_reported(&journal, Some(&cp)).unwrap();
    assert_eq!(
        outcome,
        CheckpointOutcome::Seeded {
            offset: 2,
            tail_entries: 4
        }
    );
    let full = Projection::from_journal(&journal).unwrap();
    assert_projection_eq(&resumed, &full, "reported seeded resume");
}

/// `None` reports `FullFold { NoCheckpoint }`.
#[test]
fn reported_none_is_full_fold_no_checkpoint() {
    let journal = build_journal(&[TraceOp::Commit(1), TraceOp::Commit(2)]);
    let (_p, outcome) = Projection::from_journal_with_checkpoint_reported(&journal, None).unwrap();
    assert_eq!(
        outcome,
        CheckpointOutcome::FullFold {
            reason: FullFoldReason::NoCheckpoint
        }
    );
}

/// A checkpoint whose offset is past the (shorter) journal head reports
/// `FullFold { OffsetAheadOfHead }`.
#[test]
fn reported_stale_offset_is_offset_ahead() {
    let long = build_journal(&[
        TraceOp::Commit(1),
        TraceOp::Commit(2),
        TraceOp::Commit(3),
        TraceOp::Commit(4),
        TraceOp::Commit(5),
    ]);
    let cp = Projection::from_journal(&long).unwrap().fold_checkpoint();
    let short = build_journal(&[TraceOp::Commit(1), TraceOp::Commit(2), TraceOp::Commit(3)]);
    let (_p, outcome) =
        Projection::from_journal_with_checkpoint_reported(&short, Some(&cp)).unwrap();
    assert_eq!(
        outcome,
        CheckpointOutcome::FullFold {
            reason: FullFoldReason::OffsetAheadOfHead
        }
    );
}

/// A checkpoint carrying run A's instance-id, used to seed run B, reports
/// `FullFold { WrongRun }`.
#[test]
fn reported_wrong_run_is_wrong_run() {
    fn run_journal(instance: u8) -> InMemoryJournal {
        let j = InMemoryJournal::new();
        j.append(JournalEntry::RunRegistered {
            instance_id: [instance; INSTANCE_ID_LEN],
            recipe_fingerprint: [0xab; 32],
            ts: 0,
            seq: 0,
        })
        .unwrap();
        j.append(committed_entry(1, &[])).unwrap();
        j
    }
    let cp_a = Projection::from_journal(&run_journal(0xAA))
        .unwrap()
        .fold_checkpoint();
    let run_b = run_journal(0xBB);
    let (_p, outcome) =
        Projection::from_journal_with_checkpoint_reported(&run_b, Some(&cp_a)).unwrap();
    assert_eq!(
        outcome,
        CheckpointOutcome::FullFold {
            reason: FullFoldReason::WrongRun
        }
    );
}

// ---------------------------------------------------------------------------
// Materializer path — seeded shaper children + tail-fold, no re-materialization
// ---------------------------------------------------------------------------

/// A deterministic stub: the Mote `shaper` materializes `children` (each a PURE
/// Mote with a Control edge back to the shaper). Records every shaper id it
/// *actually materializes*, so a resume can assert the ≤offset shaper is not
/// re-fired.
struct StubMaterializer {
    shaper: MoteId,
    children: Vec<MoteId>,
    fired: Arc<Mutex<Vec<MoteId>>>,
}

impl TopologyMaterializer for StubMaterializer {
    fn try_materialize(
        &self,
        shaper_mote_id: MoteId,
        _def_hash: MoteDefHash,
        _result_ref: ContentRef,
        _warrant_ref: ContentRef,
    ) -> Result<Option<Vec<RegisterMote>>, ProjectionError> {
        if shaper_mote_id != self.shaper {
            return Ok(None); // not a shaper — the common fast path
        }
        self.fired.lock().unwrap().push(shaper_mote_id);
        let regs = self
            .children
            .iter()
            .map(|c| RegisterMote {
                mote_id: *c,
                nd_class: NdClass::Pure,
                effect_pattern: EffectPattern::IdempotentByConstruction,
                critic_for: None,
                is_topology_shaper: false,
                parents: SmallVec::from_vec(vec![pref(self.shaper, EdgeMeta::control())]),
                warrant_ref: ContentRef::from_bytes([0xcc; 32]),
            })
            .collect();
        Ok(Some(regs))
    }
}

fn stub(fired: &Arc<Mutex<Vec<MoteId>>>) -> Box<StubMaterializer> {
    Box::new(StubMaterializer {
        shaper: mid(100),
        children: vec![mid(101), mid(102)],
        fired: Arc::clone(fired),
    })
}

#[test]
fn checkpoint_resume_with_materializer_matches_full_and_does_not_refire() {
    // Journal: shaper commits (seq 1) -> materializes 101, 102; the M2.2c seal
    // anchoring frontier 1 lands at seq 2; then each child commits (seq 3, 4)
    // carrying its Control edge back to the shaper. The seal is interleaved at the
    // checkpoint frontier exactly as the live runtime emits it.
    let journal = InMemoryJournal::new();
    journal.append(committed_entry(100, &[])).unwrap();

    // Seed at offset 1 (after the shaper commit + its materialization) and capture
    // the checkpoint BEFORE the seal/children land.
    let seed_fired = Arc::new(Mutex::new(Vec::new()));
    let mut seed = Projection::with_materializer(stub(&seed_fired));
    for e in journal.read_entries_by_seq(0..2).unwrap() {
        seed.fold(&e).unwrap();
    }
    let cp = FoldCheckpoint::from_bytes(&seed.fold_checkpoint().to_bytes()).unwrap();
    assert_eq!(cp.journal_offset(), 1);

    // M2.2c: co-commit the journaled seal anchoring the seeded digest at frontier 1,
    // then the tail child commits.
    journal
        .append(JournalEntry::DigestSealed {
            through_seq: 1,
            state_digest: seed.state_digest(),
            seq: 0,
        })
        .unwrap();
    journal
        .append(committed_entry(101, &[pref(mid(100), EdgeMeta::control())]))
        .unwrap();
    journal
        .append(committed_entry(102, &[pref(mid(100), EdgeMeta::control())]))
        .unwrap();

    // Full materializer fold (the comparison target). The seal folds as a no-op, so
    // the shaper is still materialized exactly once.
    let full_fired = Arc::new(Mutex::new(Vec::new()));
    let full = Projection::from_journal_with_checkpoint_with_materializer(
        &journal,
        stub(&full_fired),
        None,
    )
    .unwrap();
    assert_eq!(
        *full_fired.lock().unwrap(),
        vec![mid(100)],
        "full fold materializes the shaper exactly once"
    );

    // Resume: the ≤offset shaper must NOT be re-materialized; the tail child
    // commits fold against the seeded (already-declared) children.
    let resume_fired = Arc::new(Mutex::new(Vec::new()));
    let resumed = Projection::from_journal_with_checkpoint_with_materializer(
        &journal,
        stub(&resume_fired),
        Some(&cp),
    )
    .unwrap();

    assert!(
        resume_fired.lock().unwrap().is_empty(),
        "the checkpointed shaper must NOT be re-materialized on resume"
    );
    assert_projection_eq(&resumed, &full, "materializer resume");
    // Spot-check the materialized topology survived the checkpoint round-trip.
    assert_eq!(
        resumed.children_of(&mid(100)).len(),
        2,
        "shaper keeps 2 children"
    );
    assert_eq!(
        resumed.parents_of(&mid(101)),
        full.parents_of(&mid(101)),
        "child 101's narrowed parent edge is preserved"
    );
}

// ---------------------------------------------------------------------------
// Scale / perf (`#[ignore]`) — the HONEST checkpoint win. Run:
//   cargo test -p kx-projection --release --test fold_checkpoint \
//     -- --ignored --nocapture --test-threads=1
//
// What this measures (and what it deliberately does NOT claim):
//
//   * The win is NOT "a bincode decode beats a re-fold" — M2.1 already made the
//     in-memory fold ~0.5us/Mote, and a full-state decode of a *clean* log is
//     roughly a wash (the rkyv zero-copy codec, roadmap M2.2d, is what makes the
//     clean case a win). We do not pretend otherwise.
//   * The REAL win is decoupling resume cost from journal CHURN: under a
//     crash-loopy workload (the Risk #1 scenario — many Proposed/Failed attempts
//     per Mote before a commit) total entries >> distinct Motes. A full fold pays
//     for every entry; the checkpoint decode pays only for live state. So resume
//     time is bounded by live-state size, not journal length.
// ---------------------------------------------------------------------------

/// A distinct `MoteId` per `u32` (a single byte would collide at scale).
fn mid_n(i: u32) -> MoteId {
    let mut b = [0u8; 32];
    b[..4].copy_from_slice(&i.to_le_bytes());
    MoteId::from_bytes(b)
}

#[test]
#[ignore = "scale: run --release --test fold_checkpoint -- --ignored --nocapture"]
fn scale_resume_is_bounded_by_live_state_not_churn() {
    use std::time::Instant;

    const M: u32 = 10_000; // distinct Motes (live state)
    const CHURN: u32 = 10; // Proposed+Failed(pre-commit) cycles before each commit

    // High-churn journal: each Mote retries CHURN times, then commits. Total
    // entries = M*(2*CHURN+1); distinct Motes = M. Pre-commit (`TimedOut`)
    // failures keep every Mote terminally clean -> all commit.
    let journal = InMemoryJournal::new();
    let mut ctr = 0u64;
    for i in 0..M {
        for _ in 0..CHURN {
            ctr += 2;
            journal
                .append(JournalEntry::Proposed {
                    mote_id: mid_n(i),
                    idempotency_key: ukey(ctr),
                    seq: 0,
                    nondeterminism: NdClass::Pure,
                    placement_hint: 0,
                    warrant_ref: war(),
                })
                .unwrap();
            journal
                .append(JournalEntry::Failed {
                    mote_id: mid_n(i),
                    idempotency_key: ukey(ctr + 1),
                    seq: 0,
                    reason_class: FailureReason::TimedOut,
                    reporter_id: 0,
                })
                .unwrap();
        }
        let parents = if i >= 1 {
            vec![pref(mid_n(i - 1), EdgeMeta::data())]
        } else {
            vec![]
        };
        let pe: SmallVec<[ParentEntry; 4]> =
            parents.iter().map(ParentEntry::from_parent_ref).collect();
        journal
            .append(JournalEntry::Committed {
                mote_id: mid_n(i),
                idempotency_key: *mid_n(i).as_bytes(),
                seq: 0,
                nondeterminism: NdClass::Pure,
                result_ref: ContentRef::from_bytes([7u8; 32]),
                parents: pe,
                warrant_ref: war(),
                mote_def_hash: MoteDefHash::from_bytes([1u8; 32]),
            })
            .unwrap();
    }
    let total = journal.current_seq().unwrap();

    // Capture the head checkpoint at frontier `total` (the digest the seal anchors).
    let pre_seal = Projection::from_journal(&journal).unwrap();
    assert_eq!(pre_seal.committed_count(), M as usize);
    let seed_digest = pre_seal.state_digest();
    let bytes = pre_seal.fold_checkpoint().to_bytes();
    drop(pre_seal);

    // M2.2c: co-commit the journaled seal at the checkpoint frontier (as the runtime
    // does) so the resume can anchor + seed; without it recovery full-folds.
    journal
        .append(JournalEntry::DigestSealed {
            through_seq: total,
            state_digest: seed_digest,
            seq: 0,
        })
        .unwrap();

    // Baseline: a full cold re-fold of the whole churned log (now `total + 1` entries).
    let t0 = Instant::now();
    let full = Projection::from_journal(&journal).unwrap();
    let full_us = t0.elapsed().as_secs_f64() * 1e6;

    // Resume: read one sidecar blob + decode live state + verify the seal + fold the
    // (1-entry: the seal) tail.
    let t1 = Instant::now();
    let cp = FoldCheckpoint::from_bytes(&bytes).unwrap();
    let (resumed, outcome) =
        Projection::from_journal_with_checkpoint_reported(&journal, Some(&cp)).unwrap();
    let resume_us = t1.elapsed().as_secs_f64() * 1e6;
    assert!(
        matches!(outcome, CheckpointOutcome::Seeded { offset, .. } if offset == total),
        "resume must anchor on the journaled seal; got {outcome:?}"
    );
    assert_eq!(
        resumed.state_digest(),
        full.state_digest(),
        "resume must reproduce the full fold exactly"
    );

    let speedup = full_us / resume_us;
    eprintln!(
        "total_entries={total} live_motes={M} churn={CHURN}x  \
         full_refold={:.2}ms  resume={:.2}ms  speedup={speedup:.1}x",
        full_us / 1000.0,
        resume_us / 1000.0
    );
    // Under churn, resume (bounded by live state) must beat a full re-fold
    // (bounded by total entries). Conservative gate to avoid CI flake while
    // catching a regression that re-couples resume to journal length.
    assert!(
        speedup > 1.5,
        "resume should be bounded by live state, not journal churn; got \
         {speedup:.1}x (full={full_us:.0}us, resume={resume_us:.0}us, total_entries={total})"
    );
}

/// Correctness-at-scale (no perf gate): a clean 25k-distinct-Mote journal
/// resumes to a bit-identical projection. The clean case is a perf *wash* for
/// the bincode codec (see the churn test above) — this pins correctness only.
#[test]
#[ignore = "scale: run --release --test fold_checkpoint -- --ignored --nocapture"]
fn scale_resume_matches_full_fold_at_25k() {
    const N: u32 = 25_000;
    let journal = InMemoryJournal::new();
    for i in 0..N {
        let parents = if i >= 1 {
            vec![pref(mid_n(i - 1), EdgeMeta::data())]
        } else {
            vec![]
        };
        let pe: SmallVec<[ParentEntry; 4]> =
            parents.iter().map(ParentEntry::from_parent_ref).collect();
        journal
            .append(JournalEntry::Committed {
                mote_id: mid_n(i),
                idempotency_key: *mid_n(i).as_bytes(),
                seq: 0,
                nondeterminism: NdClass::Pure,
                result_ref: ContentRef::from_bytes([7u8; 32]),
                parents: pe,
                warrant_ref: war(),
                mote_def_hash: MoteDefHash::from_bytes([1u8; 32]),
            })
            .unwrap();
    }
    let full = Projection::from_journal(&journal).unwrap();
    // Resume from a mid-log checkpoint (offset = 20k), folding a 5k tail.
    let cp = seed_through(&journal, 20_000).fold_checkpoint();
    let resumed = Projection::from_journal_with_checkpoint(&journal, Some(&cp)).unwrap();
    assert_eq!(resumed.state_digest(), full.state_digest());
    assert_eq!(resumed.committed_count(), N as usize);
}

/// M2.2b — the SAME churn-bounded resume property, proven **end-to-end through a
/// real disk-backed SQLite journal + an on-disk checkpoint sidecar** (not the
/// in-memory journal double). A full fold reads + folds every SQLite row; a seeded
/// resume reads one sidecar blob + decodes live state + folds an (empty) tail.
///
/// Lives here in `kx-projection` (not `kx-runtime`) on purpose: `kx-runtime`
/// transitively links the `kx-llamacpp` C++ FFI, whose cmake build is not
/// provisioned in the lean `scale-smoke` CI job; `kx-projection`'s tree
/// (`kx-journal`/`rusqlite`, no llamacpp) is. The functional `checkpoint_io`
/// atomic-sidecar wiring is covered by `kx-runtime`'s own test job.
#[test]
#[ignore = "scale: run --release --test fold_checkpoint -- --ignored --nocapture"]
fn scale_resume_through_sqlite_is_bounded_by_live_state() {
    use std::time::Instant;

    const M: u32 = 5_000; // distinct Motes (live state)
    const CHURN: u32 = 10; // Proposed+Failed(pre-commit) cycles before each commit
    const BATCH: usize = 4_000; // group-commit chunk so setup stays fast

    let dir = tempfile::tempdir().unwrap();
    let jpath = dir.path().join("scale.sqlite");
    {
        let j = SqliteJournal::open(&jpath).unwrap();
        let mut buf: Vec<JournalEntry> = Vec::with_capacity(BATCH);
        let mut ctr = 0u64;
        for i in 0..M {
            for _ in 0..CHURN {
                ctr += 2;
                buf.push(JournalEntry::Proposed {
                    mote_id: mid_n(i),
                    idempotency_key: ukey(ctr),
                    seq: 0,
                    nondeterminism: NdClass::Pure,
                    placement_hint: 0,
                    warrant_ref: war(),
                });
                buf.push(JournalEntry::Failed {
                    mote_id: mid_n(i),
                    idempotency_key: ukey(ctr + 1),
                    seq: 0,
                    reason_class: FailureReason::TimedOut,
                    reporter_id: 0,
                });
                if buf.len() >= BATCH {
                    j.append_batch(std::mem::take(&mut buf)).unwrap();
                }
            }
            let parents = if i >= 1 {
                vec![pref(mid_n(i - 1), EdgeMeta::data())]
            } else {
                vec![]
            };
            let pe: SmallVec<[ParentEntry; 4]> =
                parents.iter().map(ParentEntry::from_parent_ref).collect();
            buf.push(JournalEntry::Committed {
                mote_id: mid_n(i),
                idempotency_key: *mid_n(i).as_bytes(),
                seq: 0,
                nondeterminism: NdClass::Pure,
                result_ref: ContentRef::from_bytes([7u8; 32]),
                parents: pe,
                warrant_ref: war(),
                mote_def_hash: MoteDefHash::from_bytes([1u8; 32]),
            });
            if buf.len() >= BATCH {
                j.append_batch(std::mem::take(&mut buf)).unwrap();
            }
        }
        if !buf.is_empty() {
            j.append_batch(buf).unwrap();
        }
    }
    let total = SqliteJournal::open(&jpath).unwrap().current_seq().unwrap();

    // Capture the head checkpoint at frontier `total` and persist it as an on-disk
    // sidecar blob (`seed_digest` is the digest the M2.2c seal will anchor).
    let pre_seal = Projection::from_journal(&SqliteJournal::open(&jpath).unwrap()).unwrap();
    let seed_digest = pre_seal.state_digest();
    let sidecar = dir.path().join("scale.sqlite.ckpt");
    std::fs::write(&sidecar, pre_seal.fold_checkpoint().to_bytes()).unwrap();
    drop(pre_seal);

    // M2.2c: co-commit the journaled digest seal at the checkpoint frontier (exactly
    // as the live runtime does right after writing the sidecar). Without it, recovery
    // refuses to seed (`SealMissing`) and full-folds — so this also guards the
    // unforgeability-gate-vs-perf interaction: an anchored seed MUST stay bounded by
    // live state, not journal length.
    SqliteJournal::open(&jpath)
        .unwrap()
        .append(JournalEntry::DigestSealed {
            through_seq: total,
            state_digest: seed_digest,
            seq: 0,
        })
        .unwrap();

    // Baseline: a full cold re-fold reading every row (now `total + 1`, incl. the seal).
    let t0 = Instant::now();
    let full = Projection::from_journal(&SqliteJournal::open(&jpath).unwrap()).unwrap();
    let full_us = t0.elapsed().as_secs_f64() * 1e6;
    let reference = full.state_digest();

    // Seeded recovery: read the sidecar, verify it against the journaled seal, fold
    // the (1-entry: the seal) tail. The seed MUST anchor on the seal (`Seeded`).
    let t1 = Instant::now();
    let cp = FoldCheckpoint::from_bytes(&std::fs::read(&sidecar).unwrap()).unwrap();
    let (seeded, outcome) = Projection::from_journal_with_checkpoint_reported(
        &SqliteJournal::open(&jpath).unwrap(),
        Some(&cp),
    )
    .unwrap();
    let seeded_us = t1.elapsed().as_secs_f64() * 1e6;
    assert!(
        matches!(outcome, CheckpointOutcome::Seeded { offset, .. } if offset == total),
        "seeded recovery must anchor on the journaled seal; got {outcome:?}"
    );
    assert_eq!(
        seeded.state_digest(),
        reference,
        "seeded recovery must reproduce the full fold exactly"
    );

    let speedup = full_us / seeded_us;
    eprintln!(
        "sqlite total_entries={total} live_motes={M} churn={CHURN}x  \
         full_refold={:.2}ms  seeded_resume={:.2}ms  speedup={speedup:.1}x",
        full_us / 1000.0,
        seeded_us / 1000.0
    );
    // Conservative gate (matches the in-memory churn test) to catch a regression
    // that re-couples resume to journal length without flaking on slow CI runners.
    assert!(
        speedup > 1.5,
        "seeded recovery should be bounded by live state, not journal length; got \
         {speedup:.1}x (full={full_us:.0}us, seeded={seeded_us:.0}us, total_entries={total})"
    );
}

// A compile-time + smoke check that the public error type is matchable by
// downstream recovery code.
#[test]
fn checkpoint_error_is_public_and_matchable() {
    let err = FoldCheckpoint::from_bytes(&[0u8; 4]).unwrap_err();
    assert!(matches!(err, CheckpointError::TooShort { .. }));
}
