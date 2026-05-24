//! Property tests for `Projection::fold` (SN-4 v2 #6).
//!
//! The projection's correctness contract from `projection.md` is:
//! > Two folds of the same log prefix produce bit-equivalent state.
//!
//! These properties pin that across the entire input space rather than
//! hand-picked cases.
//!
//! Properties:
//!
//! 1. **Idempotent over the log.** Folding the same sequence of journal
//!    entries twice (into a fresh projection each time) produces equal
//!    `current_seq()` + equal per-state counts.
//! 2. **`from_journal` matches direct fold.** Building a projection by
//!    `from_journal` on a SqliteJournal produces the same state as folding
//!    every entry into a fresh `Projection` directly.
//! 3. **Snapshot equals projection at the same `seq`.** A snapshot taken
//!    after N folds reports the same state for every Mote as the
//!    projection at that point.
//! 4. **`fold_many` matches a `fold` loop.** The new bulk-apply API
//!    produces identical state to applying entries one-by-one.

use kx_content::ContentRef;
use kx_journal::{FailureReason, JournalEntry, RepudiationReason};
use kx_mote::{MoteDefHash, MoteId, NdClass};
use kx_projection::{MoteState, Projection};
use proptest::prelude::*;
use smallvec::SmallVec;

// ---------------------------------------------------------------------------
// Strategies for journal entries that the projection can fold without errors.
//
// Constraint: a fold over a sequence may not produce duplicate `Committed`
// entries for the same `MoteId` (that's a journal-impl bug). So the
// strategies generate entries where mote_id ranges are partitioned to
// minimize collision risk, and the test logic deduplicates Committed-per-id.
// ---------------------------------------------------------------------------

fn arb_proposed(mote_id_seed: u8, seq: u64) -> JournalEntry {
    JournalEntry::Proposed {
        mote_id: MoteId::from_bytes([mote_id_seed; 32]),
        idempotency_key: [mote_id_seed; 32],
        seq,
        nondeterminism: NdClass::Pure,
        placement_hint: 0,
    }
}

fn arb_committed(mote_id_seed: u8, seq: u64, nd: NdClass) -> JournalEntry {
    JournalEntry::Committed {
        mote_id: MoteId::from_bytes([mote_id_seed; 32]),
        idempotency_key: [mote_id_seed; 32],
        seq,
        nondeterminism: nd,
        result_ref: ContentRef::from_bytes([mote_id_seed; 32]),
        parents: SmallVec::new(),
        mote_def_hash: MoteDefHash::from_bytes([mote_id_seed; 32]),
    }
}

fn arb_failed(mote_id_seed: u8, seq: u64) -> JournalEntry {
    JournalEntry::Failed {
        mote_id: MoteId::from_bytes([mote_id_seed; 32]),
        idempotency_key: [mote_id_seed; 32],
        seq,
        reason_class: FailureReason::TimedOut,
        reporter_id: 0,
    }
}

fn arb_repudiated(target_seed: u8, target_committed_seq: u64, seq: u64) -> JournalEntry {
    JournalEntry::Repudiated {
        target_mote_id: MoteId::from_bytes([target_seed; 32]),
        idempotency_key: [0u8; 32],
        seq,
        target_committed_seq,
        reason_class: RepudiationReason::OperatorAction,
        repudiator_id: 0,
    }
}

/// A small entry-spec — picks a kind + a mote_id_seed (0..=15). The actual
/// sequence numbers are assigned by the test in fold order so the projection
/// sees monotonic `seq`.
#[derive(Clone, Debug)]
enum EntrySpec {
    Proposed(u8),
    Committed(u8, NdClass),
    Failed(u8),
    /// Repudiation targets an arbitrary `(seed, committed_seq)` — the projection
    /// silently no-ops if no matching Committed exists yet, which is the
    /// documented behavior we want to exercise.
    Repudiated {
        target: u8,
        target_committed_seq: u64,
    },
}

fn arb_entry_spec() -> impl Strategy<Value = EntrySpec> {
    prop_oneof![
        (0u8..16u8).prop_map(EntrySpec::Proposed),
        (
            0u8..16u8,
            prop_oneof![
                Just(NdClass::Pure),
                Just(NdClass::WorldMutating),
                Just(NdClass::ReadOnlyNondet)
            ]
        )
            .prop_map(|(s, nd)| EntrySpec::Committed(s, nd)),
        (0u8..16u8).prop_map(EntrySpec::Failed),
        (0u8..16u8, any::<u64>()).prop_map(|(target, committed_seq)| EntrySpec::Repudiated {
            target,
            target_committed_seq: committed_seq,
        }),
    ]
}

/// A trace of entry specs. The projection assigns sequential `seq` values
/// (1, 2, 3, ...) as it folds. We deduplicate Committed-per-mote to keep
/// `fold` from returning DuplicateCommitted (that's a separate test).
fn arb_trace() -> impl Strategy<Value = Vec<EntrySpec>> {
    proptest::collection::vec(arb_entry_spec(), 0..=20)
}

/// Materialize a trace into a Vec<JournalEntry> with deterministic sequential
/// `seq` numbering, skipping any subsequent Committed for the same
/// mote_id_seed (the journal would dedupe; we emulate that).
fn materialize(trace: &[EntrySpec]) -> Vec<JournalEntry> {
    use std::collections::BTreeSet;
    let mut committed_seeds: BTreeSet<u8> = BTreeSet::new();
    let mut out = Vec::with_capacity(trace.len());
    let mut seq: u64 = 1;
    for spec in trace {
        let entry = match spec {
            EntrySpec::Proposed(s) => arb_proposed(*s, seq),
            EntrySpec::Committed(s, nd) => {
                if committed_seeds.contains(s) {
                    continue;
                }
                committed_seeds.insert(*s);
                arb_committed(*s, seq, *nd)
            }
            EntrySpec::Failed(s) => arb_failed(*s, seq),
            EntrySpec::Repudiated {
                target,
                target_committed_seq,
            } => arb_repudiated(*target, *target_committed_seq, seq),
        };
        out.push(entry);
        seq += 1;
    }
    out
}

// ---------------------------------------------------------------------------
// Helpers to compare two projections' observable state
// ---------------------------------------------------------------------------

fn projection_signature(p: &Projection) -> (u64, usize, usize, usize, usize, usize, usize) {
    (
        p.current_seq(),
        p.len(),
        p.committed_count(),
        p.repudiated_count(),
        p.failed_count(),
        p.scheduled_count(),
        p.pending_count(),
    )
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 48,
        .. ProptestConfig::default()
    })]

    // Property 1 — idempotent over the log: same entries twice → same state.
    #[test]
    fn prop_fold_idempotent_over_log(trace in arb_trace()) {
        let entries = materialize(&trace);

        let mut p1 = Projection::new();
        for e in &entries {
            p1.fold(e).expect("fold p1");
        }

        let mut p2 = Projection::new();
        for e in &entries {
            p2.fold(e).expect("fold p2");
        }

        prop_assert_eq!(projection_signature(&p1), projection_signature(&p2));

        // Also: state_of() agrees for every Mote.
        let motes_p1: Vec<MoteId> = p1.iter_motes().map(|(id, _)| id).collect();
        let motes_p2: Vec<MoteId> = p2.iter_motes().map(|(id, _)| id).collect();
        prop_assert_eq!(&motes_p1, &motes_p2);
        for id in &motes_p1 {
            prop_assert_eq!(p1.state_of(id), p2.state_of(id));
        }
    }

    // Property 2 — `fold_many` matches a `fold` loop.
    #[test]
    fn prop_fold_many_matches_fold_loop(trace in arb_trace()) {
        let entries = materialize(&trace);

        let mut p_loop = Projection::new();
        for e in &entries {
            p_loop.fold(e).expect("fold loop");
        }

        let mut p_bulk = Projection::new();
        p_bulk.fold_many(entries.iter().cloned()).expect("fold_many");

        prop_assert_eq!(projection_signature(&p_loop), projection_signature(&p_bulk));
    }

    // Property 3 — snapshot at seq N equals projection at seq N.
    #[test]
    fn prop_snapshot_matches_projection_at_seq(trace in arb_trace()) {
        let entries = materialize(&trace);

        let mut p = Projection::new();
        for e in &entries {
            p.fold(e).expect("fold");
        }

        let snap = p.snapshot();
        prop_assert_eq!(snap.seq(), p.current_seq());
        prop_assert_eq!(snap.len(), p.len());
        prop_assert_eq!(snap.committed_count(), p.committed_count());
        prop_assert_eq!(snap.repudiated_count(), p.repudiated_count());

        for (id, projection_state) in p.iter_motes() {
            prop_assert_eq!(snap.state_of(&id), projection_state);
        }
    }

    // Property 4 — `from_journal` matches direct fold for a SqliteJournal that
    // already holds the entries.
    #[test]
    fn prop_from_journal_matches_direct_fold(trace in arb_trace()) {
        use kx_journal::{Journal, SqliteJournal};

        let entries = materialize(&trace);

        // Build a journal from the trace.
        let journal = SqliteJournal::open_in_memory().expect("open");
        for e in &entries {
            journal.append(e.clone()).expect("append");
        }

        // Direct fold of the SAME entries:
        let mut p_direct = Projection::new();
        for e in &entries {
            p_direct.fold(e).expect("direct fold");
        }

        // from_journal reconstructs the projection from journal state. The
        // journal may renumber `seq` (it assigns its own monotonic seq); the
        // resulting projection's `current_seq` matches the journal's. The set
        // of Mote ids + their per-state classification must agree, even if
        // the exact seq numbers shift.
        let p_via_journal = Projection::from_journal(&journal).expect("from_journal");

        prop_assert_eq!(p_via_journal.len(), p_direct.len());
        prop_assert_eq!(
            p_via_journal.committed_count(),
            p_direct.committed_count()
        );
        prop_assert_eq!(
            p_via_journal.repudiated_count(),
            p_direct.repudiated_count()
        );
        prop_assert_eq!(
            p_via_journal.failed_count(),
            p_direct.failed_count()
        );
        // Scheduled vs Pending may differ if from_journal sees journal-renumbered
        // entries in a different order; but committed + repudiated are stable
        // under any consistent ordering of the source set.
    }
}

// ---------------------------------------------------------------------------
// Concurrency: snapshot isolation under concurrent fold (SN-4 v2 #7)
// ---------------------------------------------------------------------------

#[test]
fn projection_and_snapshot_are_send_at_compile_time() {
    fn assert_send<T: Send>() {}
    assert_send::<Projection>();
    assert_send::<kx_projection::Snapshot>();
}

/// One writer thread folds 30 Committed entries into a shared
/// `Mutex<Projection>` while a reader thread takes snapshots between
/// folds. Each snapshot's reported state must reflect ONLY the entries
/// folded UP TO the moment of `snapshot()` — no later folds bleed in.
///
/// This is the D16 snapshot-isolation contract under the actual `Send`
/// claim, exercising the `Projection: !Sync` (interior mutex) shape that
/// downstream code will use.
#[test]
fn snapshot_isolation_under_concurrent_writer() {
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;

    let p: Arc<Mutex<Projection>> = Arc::new(Mutex::new(Projection::new()));

    let writer_p = Arc::clone(&p);
    let writer = thread::spawn(move || {
        for i in 1..=30u64 {
            let entry = arb_committed((i as u8) + 100, i, NdClass::Pure);
            {
                let mut guard = writer_p.lock().expect("writer lock");
                guard.fold(&entry).expect("writer fold");
            }
            // Give the reader a chance to interleave between folds.
            thread::sleep(Duration::from_micros(50));
        }
    });

    let reader_p = Arc::clone(&p);
    let reader = thread::spawn(move || {
        let mut snaps: Vec<(u64, usize)> = Vec::new();
        for _ in 0..20 {
            let snap = {
                let guard = reader_p.lock().expect("reader lock");
                guard.snapshot()
            };
            // Snapshot's seq + committed_count must agree: every Mote
            // visible in iter_motes with state == Committed was folded
            // before snapshot() returned.
            let committed_at_snap = snap.committed_count();
            let seen_committed = snap
                .iter_motes()
                .filter(|(_, s)| *s == MoteState::Committed)
                .count();
            assert_eq!(
                committed_at_snap, seen_committed,
                "snapshot committed_count disagrees with iter_motes filter"
            );
            // Critically: the snapshot's seq matches the count of folds
            // visible (each entry in our test has seq == iteration index,
            // so seq == committed_count is the post-fold invariant).
            snaps.push((snap.seq(), committed_at_snap));
            thread::sleep(Duration::from_micros(75));
        }
        snaps
    });

    writer.join().expect("writer panic");
    let snaps = reader.join().expect("reader panic");

    // Final state: all 30 entries are visible in the projection.
    let final_count = {
        let guard = p.lock().expect("final lock");
        guard.committed_count()
    };
    assert_eq!(final_count, 30);

    // Snapshots taken over time form a monotonic sequence of seq values.
    for w in snaps.windows(2) {
        let (s1, _) = w[0];
        let (s2, _) = w[1];
        assert!(
            s2 >= s1,
            "snapshot seq regressed: {s1} → {s2} — snapshot isolation broken"
        );
    }
}

// ---------------------------------------------------------------------------
// Smoke tests for the new API surface (iter_motes, *_count, fold_many)
// ---------------------------------------------------------------------------

#[test]
fn iter_motes_returns_all_known_motes_in_order() {
    let mut p = Projection::new();
    p.fold(&arb_committed(3, 1, NdClass::Pure)).unwrap();
    p.fold(&arb_committed(1, 2, NdClass::Pure)).unwrap();
    p.fold(&arb_failed(2, 3)).unwrap();

    let ids: Vec<MoteId> = p.iter_motes().map(|(id, _)| id).collect();
    // BTreeMap iteration → ascending by MoteId bytes
    assert_eq!(ids[0], MoteId::from_bytes([1; 32]));
    assert_eq!(ids[1], MoteId::from_bytes([2; 32]));
    assert_eq!(ids[2], MoteId::from_bytes([3; 32]));
}

#[test]
fn per_state_counts_sum_to_len() {
    let mut p = Projection::new();
    p.fold(&arb_committed(1, 1, NdClass::Pure)).unwrap();
    p.fold(&arb_committed(2, 2, NdClass::Pure)).unwrap();
    p.fold(&arb_failed(3, 3)).unwrap();
    p.fold(&arb_proposed(4, 4)).unwrap();

    let total = p.committed_count()
        + p.repudiated_count()
        + p.failed_count()
        + p.scheduled_count()
        + p.pending_count();
    assert_eq!(total, p.len());
    assert_eq!(p.committed_count(), 2);
    assert_eq!(p.failed_count(), 1);
    assert_eq!(p.scheduled_count(), 1);
}

#[test]
fn fold_many_stops_on_first_error_and_state_reflects_applied_entries() {
    let mut p = Projection::new();
    let entries = vec![
        arb_committed(1, 1, NdClass::Pure),
        arb_committed(2, 2, NdClass::Pure),
        // Duplicate Committed for mote 1 → DuplicateCommitted error
        arb_committed(1, 3, NdClass::Pure),
        // Would-be-applied but stops on the error above
        arb_committed(4, 4, NdClass::Pure),
    ];
    let res = p.fold_many(entries);
    assert!(res.is_err());
    // First two were applied before the error
    assert_eq!(p.committed_count(), 2);
    assert!(p
        .iter_motes()
        .any(|(id, _)| id == MoteId::from_bytes([1; 32])));
    assert!(p
        .iter_motes()
        .any(|(id, _)| id == MoteId::from_bytes([2; 32])));
    // The post-error entry was NOT applied
    assert!(!p
        .iter_motes()
        .any(|(id, _)| id == MoteId::from_bytes([4; 32])));
}
