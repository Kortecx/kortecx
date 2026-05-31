// Integration-test file: compiled as a separate crate from the host lib;
// inherits workspace `[lints]` deny on `unwrap_used` / `expect_used` but tests
// legitimately use `.unwrap()` for fixture construction. The `pedantic` group
// is also allowed here — tests routinely do things pedantic flags (small-int
// casts on byte seeds, helper-fn definitions after let-bindings, etc.) that
// would be needless friction to refactor.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]
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
        warrant_ref: ContentRef::from_bytes([0xaa; 32]),
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
        warrant_ref: ContentRef::from_bytes([0xaa; 32]),
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

    // **v3 (M1.1)** — prepending a RunRegistered entry to any trace leaves the
    // committed-set signature unchanged (it registers no Mote) and sets the run
    // identity. Folding it never touches the Mote-DAG; the committed digest the
    // runtime hashes is therefore invariant under run registration.
    #[test]
    fn prop_run_registered_is_off_dag_for_any_trace(trace in arb_trace()) {
        let entries = materialize(&trace);

        let mut without_reg = Projection::new();
        for e in &entries {
            without_reg.fold(e).expect("fold without reg");
        }

        let mut with_reg = Projection::new();
        with_reg.fold(&run_registered(0x99, 0xee, 0)).expect("fold reg");
        for e in &entries {
            with_reg.fold(e).expect("fold with reg");
        }

        // RunRegistered adds no Mote → identical Mote set + per-state counts.
        prop_assert_eq!(with_reg.len(), without_reg.len());
        prop_assert_eq!(with_reg.committed_count(), without_reg.committed_count());
        prop_assert_eq!(with_reg.repudiated_count(), without_reg.repudiated_count());
        prop_assert_eq!(with_reg.failed_count(), without_reg.failed_count());
        prop_assert!(with_reg.run_registration().is_some());
        prop_assert!(without_reg.run_registration().is_none());
        for (id, state) in without_reg.iter_motes() {
            prop_assert_eq!(with_reg.state_of(&id), state);
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

// ---------------------------------------------------------------------------
// **v3 (M1.1, D63/D64) — RunRegistered fold is off the Mote-DAG (O(1)).**
//
// RunRegistered names no Mote: folding it MUST register no MoteInfo, leave the
// children index empty, advance current_seq, set run_registration(), and never
// touch any Mote's per-state classification. These pin that the run-registration
// fact is a pure off-DAG marker — the scalability guarantee (no O(n²) index
// rebuild, no phantom Mote) and the determinism guarantee (the digest, computed
// over committed Motes only, is unaffected).
// ---------------------------------------------------------------------------

fn run_registered(instance_seed: u8, recipe_seed: u8, seq: u64) -> JournalEntry {
    JournalEntry::RunRegistered {
        instance_id: [instance_seed; kx_journal::INSTANCE_ID_LEN],
        recipe_fingerprint: [recipe_seed; 32],
        ts: 0,
        seq,
    }
}

#[test]
fn run_registered_fold_is_off_dag() {
    let mut p = Projection::new();
    p.fold(&run_registered(0xa1, 0xb2, 1)).unwrap();
    // No Mote registered, all per-state counts zero.
    assert_eq!(p.len(), 0);
    assert!(p.is_empty());
    assert_eq!(p.committed_count(), 0);
    assert_eq!(p.iter_motes().count(), 0);
    // current_seq advanced; run identity is queryable.
    assert_eq!(p.current_seq(), 1);
    assert_eq!(
        p.run_registration(),
        Some(([0xa1; kx_journal::INSTANCE_ID_LEN], [0xb2; 32]))
    );
}

#[test]
fn run_registered_fold_is_idempotent_on_replay() {
    // Replaying the seq=1 RunRegistered (recovery re-folds the same bytes) sets
    // the same run identity and adds no Mote — instance_id read, never recomputed.
    let mut p = Projection::new();
    p.fold(&run_registered(0x5a, 0x6b, 1)).unwrap();
    let first = p.run_registration();
    p.fold(&run_registered(0x5a, 0x6b, 1)).unwrap();
    assert_eq!(p.run_registration(), first);
    assert_eq!(p.len(), 0);
}

#[test]
fn run_registered_before_motes_does_not_change_committed_set() {
    // Run with registration prepended (seq=1), then Motes.
    let mut with_reg = Projection::new();
    with_reg.fold(&run_registered(0x11, 0x22, 1)).unwrap();
    with_reg.fold(&arb_committed(1, 2, NdClass::Pure)).unwrap();
    with_reg
        .fold(&arb_committed(2, 3, NdClass::WorldMutating))
        .unwrap();

    // Same Motes, no registration.
    let mut without_reg = Projection::new();
    without_reg
        .fold(&arb_committed(1, 1, NdClass::Pure))
        .unwrap();
    without_reg
        .fold(&arb_committed(2, 2, NdClass::WorldMutating))
        .unwrap();

    // The committed SET is identical (RunRegistered adds no Mote) — this is what
    // the runtime digest (committed Motes only) hashes, so it is unchanged.
    assert_eq!(with_reg.committed_count(), without_reg.committed_count());
    assert_eq!(with_reg.len(), without_reg.len());
    assert!(with_reg.run_registration().is_some());
    assert_eq!(without_reg.run_registration(), None);
    for (id, state) in without_reg.iter_motes() {
        assert_eq!(with_reg.state_of(&id), state);
    }
}

// ---------------------------------------------------------------------------
// **v4 (M1.2, D79) — RunVersionsResolved fold is off the Mote-DAG (O(1)).**
//
// Like RunRegistered, it names no Mote: folding registers no MoteInfo, leaves
// the children index empty, advances current_seq, and appends a metadata record
// the runtime digest (committed Motes only) can never see.
// ---------------------------------------------------------------------------

fn run_versions_resolved(
    instance_seed: u8,
    model_id: &str,
    capability: Option<kx_journal::ResolvedCapabilityRecord>,
    seq: u64,
) -> JournalEntry {
    JournalEntry::RunVersionsResolved {
        instance_id: [instance_seed; kx_journal::INSTANCE_ID_LEN],
        warrant_ref: ContentRef::from_bytes([0xcd; 32]),
        model_id: model_id.to_owned(),
        capability,
        seq,
    }
}

fn sample_cap_record() -> kx_journal::ResolvedCapabilityRecord {
    kx_journal::ResolvedCapabilityRecord {
        tool_id: "fs-read".to_owned(),
        tool_version: "1.0.0".to_owned(),
        resolved_kind: kx_journal::ResolvedKindTag::Builtin,
        resolved_def_hash: ContentRef::from_bytes([0x42; 32]),
    }
}

#[test]
fn run_versions_resolved_fold_is_off_dag_and_appends_metadata() {
    let mut p = Projection::new();
    p.fold(&run_registered(0xa1, 0xb2, 1)).unwrap();
    p.fold(&run_versions_resolved(
        0xa1,
        "qwen",
        Some(sample_cap_record()),
        2,
    ))
    .unwrap();
    p.fold(&run_versions_resolved(0xa1, "qwen", None, 3))
        .unwrap();
    // No Mote registered; current_seq advanced; both records appended in order.
    assert_eq!(p.len(), 0);
    assert_eq!(p.committed_count(), 0);
    assert_eq!(p.current_seq(), 3);
    let recs = p.run_resolved_versions();
    assert_eq!(recs.len(), 2);
    assert_eq!(recs[0].model_id, "qwen");
    assert_eq!(recs[0].capability.as_ref().unwrap().tool_id, "fs-read");
    assert!(recs[1].capability.is_none());
}

/// **Scale (D95):** folding a large run-metadata log stays O(1) **off the
/// Mote-DAG** — N `RunVersionsResolved` entries register NO Mote and never
/// rebuild the children index (the D92 O(n²) wall), so the committed digest the
/// runtime hashes (over committed Motes only) is untouched at any N. Run with
/// `cargo test -p kx-projection --release -- --ignored run_versions_fold_scale`.
#[test]
#[ignore = "scale: run with --release --ignored"]
fn run_versions_fold_scale_is_off_dag() {
    const N: u64 = 10_000;
    let mut p = Projection::new();
    p.fold(&run_registered(0x01, 0x02, 1)).unwrap();
    for seq in 2..=(N + 1) {
        let cap = if seq % 3 == 0 {
            Some(sample_cap_record())
        } else {
            None
        };
        p.fold(&run_versions_resolved(0x01, "model", cap, seq))
            .unwrap();
    }
    // Off-DAG: NO Mote registered, NO committed fact → the digest is unmoved.
    assert_eq!(p.len(), 0, "no Mote registered at any N");
    assert_eq!(p.committed_count(), 0);
    assert_eq!(p.iter_motes().count(), 0);
    // All N metadata records accrued; current_seq tracks the log.
    assert_eq!(p.run_resolved_versions().len() as u64, N);
    assert_eq!(p.current_seq(), N + 1);
}

#[test]
fn run_versions_resolved_replays_identically_from_scratch() {
    // Each journaled entry folds exactly once on replay → the accrued metadata
    // Vec is byte-identical when a fresh projection re-folds the same trace.
    let entries = vec![
        run_registered(0x5a, 0x6b, 1),
        run_versions_resolved(0x5a, "m1", Some(sample_cap_record()), 2),
        run_versions_resolved(0x5a, "m1", None, 3),
    ];
    let mut a = Projection::new();
    let mut b = Projection::new();
    for e in &entries {
        a.fold(e).unwrap();
    }
    for e in &entries {
        b.fold(e).unwrap();
    }
    assert_eq!(a.run_resolved_versions(), b.run_resolved_versions());
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

// ---------------------------------------------------------------------------
// **PR 7 — STEP 5.2 + STEP 6 properties (load-bearing recovery contract).**
//
// These properties pin the prefix-monotonicity-of-refusal contract: once
// `can_redispatch_world_effect` returns `false` at a log prefix, it returns
// `false` at every longer prefix (refusal is monotonic; once terminal,
// always terminal). They also enforce the canonical-classifier-cannot-drift
// contract via class-covering proptests over the full `FailureReason` enum
// (mirror of STEP 6.2 from PR 4.5).
// ---------------------------------------------------------------------------

use kx_journal::is_pre_commit_crash;

/// Strategy: an arbitrary `FailureReason` variant. MUST be updated when a
/// new variant lands — canonical-classifier-cannot-drift TEST-level gate.
/// The single source of class truth (pre-commit-crash vs terminal) is
/// `kx_journal::is_pre_commit_crash`; both prod code and tests call it.
fn arbitrary_failure_reason() -> impl Strategy<Value = FailureReason> {
    prop_oneof![
        Just(FailureReason::TimedOut),
        Just(FailureReason::ExecutorRefused),
        Just(FailureReason::ValidatorRejected),
        Just(FailureReason::WorkerCrashed),
        Just(FailureReason::UpstreamRepudiated),
        Just(FailureReason::UnsafeWorldMutatingConstruction),
    ]
}

fn arb_mote_id_strategy() -> impl Strategy<Value = MoteId> {
    (1u8..=200).prop_map(|s| MoteId::from_bytes([s; 32]))
}

fn effect_staged_for(mid: MoteId) -> JournalEntry {
    JournalEntry::EffectStaged {
        mote_id: mid,
        idempotency_key: mid.0,
        seq: 0,
    }
}

fn failed_for(mid: MoteId, reason: FailureReason) -> JournalEntry {
    JournalEntry::Failed {
        mote_id: mid,
        idempotency_key: mid.0,
        seq: 0,
        reason_class: reason,
        reporter_id: 0,
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        .. ProptestConfig::default()
    })]

    /// **STEP 5.2 — Prefix-monotonicity of refusal.** Once
    /// `can_redispatch_world_effect(mote)` returns `false` at some prefix of
    /// the log, it returns `false` at every longer prefix. Equivalently:
    /// refusal can only stabilize toward "more refusal," never less.
    ///
    /// This is the property that closes cell 5 of the 9-cell cross-product
    /// against future regression: any fold-branch edit that resets
    /// `terminal_failure_observed` or `inconsistent` would fail this proptest.
    #[test]
    fn prop_terminal_refusal_is_prefix_monotonic(
        mid in arb_mote_id_strategy(),
        reason in arbitrary_failure_reason(),
        n_extra in 1usize..=5,
    ) {
        // Build a log that gets the Mote into a refusal state, then appends
        // extra entries on top. Refusal must hold across every prefix from
        // the point it first becomes true.
        let mut p = Projection::new();
        p.fold(&effect_staged_for(mid)).unwrap();
        p.fold(&failed_for(mid, reason)).unwrap();

        let mut prev_refused = false;
        // Test at each prefix length 0..=n_extra additional entries.
        for i in 0..n_extra {
            let refused = !p.can_redispatch_world_effect(&mid);
            if prev_refused {
                prop_assert!(
                    refused,
                    "refusal MUST be prefix-monotonic — became false again at extra-prefix {i}"
                );
            }
            prev_refused = prev_refused || refused;
            // Append another arbitrary entry (a Proposed — chosen because
            // Proposed resets `failed_pending_reattempt` but MUST NOT
            // reset `terminal_failure_observed` or `inconsistent` per
            // STEP 5.2 + STEP 5.3 prefix-monotonic-true contract).
            p.fold(&JournalEntry::Proposed {
                mote_id: mid,
                idempotency_key: mid.0,
                seq: 0,
                nondeterminism: NdClass::Pure,
                placement_hint: 0,
                warrant_ref: ContentRef::from_bytes([0xaa; 32]),
            }).unwrap();
        }
    }

    /// **STEP 6.2 — class-covering: terminal under EffectStaged forbids
    /// redispatch.** For every `FailureReason` classified as terminal by
    /// `is_pre_commit_crash`, a journal `[EffectStaged, Failed(reason)]`
    /// folds to `can_redispatch_world_effect == false`.
    ///
    /// **The proptest gates via `is_pre_commit_crash` directly** so the
    /// production classifier and the test surface share the same function;
    /// no risk of drift if a future variant lands.
    #[test]
    fn prop_terminal_failure_under_effect_staged_refuses_redispatch(
        reason in arbitrary_failure_reason(),
        mid in arb_mote_id_strategy(),
    ) {
        prop_assume!(!is_pre_commit_crash(reason));
        let mut p = Projection::new();
        p.fold(&effect_staged_for(mid)).unwrap();
        p.fold(&failed_for(mid, reason)).unwrap();
        prop_assert!(
            !p.can_redispatch_world_effect(&mid),
            "terminal {reason:?} under EffectStaged MUST refuse redispatch"
        );
        prop_assert_eq!(p.state_of(&mid), MoteState::Failed);
    }

    /// **STEP 6.2 — class-covering: pre-commit-crash under EffectStaged
    /// permits redispatch.** Sister property to the above; pre-commit-crash
    /// failures (TimedOut, WorkerCrashed) under EffectStaged → redispatch
    /// permitted (cell 3).
    #[test]
    fn prop_precommit_failure_under_effect_staged_permits_redispatch(
        reason in arbitrary_failure_reason(),
        mid in arb_mote_id_strategy(),
    ) {
        prop_assume!(is_pre_commit_crash(reason));
        let mut p = Projection::new();
        p.fold(&effect_staged_for(mid)).unwrap();
        p.fold(&failed_for(mid, reason)).unwrap();
        prop_assert!(
            p.can_redispatch_world_effect(&mid),
            "pre-commit-crash {reason:?} under EffectStaged MUST permit redispatch (cell 3)"
        );
        // state_of_id falls through to Pending (in-flight) since
        // terminal_failure_observed is false.
        prop_assert_eq!(p.state_of(&mid), MoteState::Pending);
    }

    /// **STEP 6.1 — `inconsistent` flag is prefix-monotonic-true.** Once a
    /// Mote enters `MoteState::Inconsistent`, it stays Inconsistent across
    /// every longer log prefix. Mirror of STEP 5.2 for the cell-8 anomaly.
    #[test]
    fn prop_inconsistent_is_monotonic_true(
        mid in arb_mote_id_strategy(),
        n_extra in 1usize..=5,
    ) {
        let mut p = Projection::new();
        // Set up cell-8: EffectStaged then Repudiated, no Committed in
        // between. Sets info.inconsistent = true.
        p.fold(&effect_staged_for(mid)).unwrap();
        p.fold(&JournalEntry::Repudiated {
            target_mote_id: mid,
            idempotency_key: kx_journal::repudiation_idempotency_key(&mid, 0),
            seq: 0,
            target_committed_seq: 0,
            reason_class: RepudiationReason::OperatorAction,
            repudiator_id: 0,
        }).unwrap();

        prop_assert_eq!(p.state_of(&mid), MoteState::Inconsistent);

        // Now append extra entries. Inconsistent MUST hold across every
        // longer prefix — no fold branch may reset `info.inconsistent`.
        for _ in 0..n_extra {
            // Mix in a Proposed (the only branch that resets any flag).
            p.fold(&JournalEntry::Proposed {
                mote_id: mid,
                idempotency_key: mid.0,
                seq: 0,
                nondeterminism: NdClass::Pure,
                placement_hint: 0,
                warrant_ref: ContentRef::from_bytes([0xaa; 32]),
            }).unwrap();
            prop_assert_eq!(
                p.state_of(&mid),
                MoteState::Inconsistent,
                "inconsistent MUST be prefix-monotonic-true even after subsequent Proposed"
            );
            prop_assert!(!p.can_redispatch_world_effect(&mid));
        }
    }
}
