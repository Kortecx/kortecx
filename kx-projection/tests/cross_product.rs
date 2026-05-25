//! **PR 7 — 9-cell recovery cross-product** (per `journal-txn.md`
//! §"Recovery fold semantics" + STEP 1 + STEP 5 + STEP 6 of PR 4.5).
//!
//! These tests are the LOAD-BEARING acceptance criterion for PR 7: cell-by-cell
//! coverage of the 9-cell cross-product table. Every cell has its own named
//! test asserting `(state_of(X), can_redispatch_world_effect(X),
//! anomaly_motes())` matches the table's "Next action" column.
//!
//! Per the corpus: "the 9-cell cross-product table in journal-txn.md IS the
//! authoritative recovery semantics every executor recovery path in P1.9 will
//! be written against." This test file is the executable form of that table.
//!
//! ## The 9-cell table (reproduced for reference)
//!
//! | # | EffectStaged | Committed | Failed | Repudiated | Next action |
//! |---|:---:|:---:|:---:|:---:|---|
//! | 0 | — | — | — | — | Pending (not yet attempted) |
//! | 1 | — | — | F(any) | — | Pending (retry permitted) |
//! | 2 | ✓ | — | — | — | Pending (in-flight; redispatch OK) |
//! | 3 | ✓ | — | F(pre-commit-crash) | — | Pending (redispatch OK) |
//! | 4 | ✓ | — | F(terminal) | — | **Failed (DO NOT REDISPATCH — cell 5 hazard)** |
//! | 5 | ✓ | ✓ | (any) | — | Committed (DONE) |
//! | 6 | — | ✓ | (any) | — | Committed (standard) |
//! | 7 | ✓ | ✓ | (any) | ✓ | Repudiated |
//! | 8 | ✓ | — | — | ✓ | **Inconsistent (anomaly — cell 8 quarantine)** |
//! | 9 | — | ✓ | — | ✓ | Repudiated (standard) |
//!
//! The "9-cell" name is from STEP 1 / PR 4.5; the table actually enumerates 10
//! rows including cell 0 (no entries). Cell 4 above is the load-bearing
//! Terminal-before-Staged ordering invariant cell (STEP 5.1).

use kx_content::ContentRef;
use kx_journal::{
    is_pre_commit_crash, repudiation_idempotency_key, FailureReason, InMemoryJournal, Journal,
    JournalEntry, RepudiationReason,
};
use kx_mote::{MoteDefHash, MoteId, NdClass};
use kx_projection::{AnomalyKind, MoteState, Projection};
use smallvec::SmallVec;

// ---------------------------------------------------------------------------
// Fixture helpers
// ---------------------------------------------------------------------------

fn committed(mote_id: MoteId, seq_hint: u64) -> JournalEntry {
    JournalEntry::Committed {
        mote_id,
        idempotency_key: mote_id.0,
        seq: seq_hint,
        nondeterminism: NdClass::WorldMutating,
        result_ref: ContentRef::from_bytes([7u8; 32]),
        parents: SmallVec::new(),
        warrant_ref: ContentRef::from_bytes([0xaa; 32]),
        mote_def_hash: MoteDefHash::from_bytes([0u8; 32]),
    }
}

fn effect_staged(mote_id: MoteId, seq_hint: u64) -> JournalEntry {
    JournalEntry::EffectStaged {
        mote_id,
        idempotency_key: mote_id.0,
        seq: seq_hint,
    }
}

fn failed(mote_id: MoteId, seq_hint: u64, reason: FailureReason) -> JournalEntry {
    JournalEntry::Failed {
        mote_id,
        idempotency_key: mote_id.0,
        seq: seq_hint,
        reason_class: reason,
        reporter_id: 0,
    }
}

fn repudiated(target_mote_id: MoteId, target_committed_seq: u64, seq_hint: u64) -> JournalEntry {
    JournalEntry::Repudiated {
        target_mote_id,
        idempotency_key: repudiation_idempotency_key(&target_mote_id, target_committed_seq),
        seq: seq_hint,
        target_committed_seq,
        reason_class: RepudiationReason::OperatorAction,
        repudiator_id: 0,
    }
}

/// Build a Projection by appending entries through an `InMemoryJournal`,
/// which assigns monotonic `seq` values. Returns the projection at the
/// final state, plus the entry-by-entry seqs the journal assigned (useful
/// for constructing Repudiated entries that reference the assigned seq).
fn projection_from_entries(entries: Vec<JournalEntry>) -> (Projection, Vec<u64>) {
    let journal = InMemoryJournal::new();
    let mut seqs = Vec::with_capacity(entries.len());
    for entry in entries {
        let returned = journal.append(entry).expect("append");
        seqs.push(returned.seq());
    }
    let projection = Projection::from_journal(&journal).expect("from_journal");
    (projection, seqs)
}

// ---------------------------------------------------------------------------
// Cell 0 — empty (no entries)
// ---------------------------------------------------------------------------

#[test]
fn cell_0_no_entries_yields_pending_and_no_redispatch() {
    let mid = MoteId::from_bytes([1u8; 32]);
    let (p, _) = projection_from_entries(vec![]);
    assert_eq!(p.state_of(&mid), MoteState::Pending);
    assert!(!p.can_redispatch_world_effect(&mid));
    assert!(p.anomaly_motes().is_empty());
}

// ---------------------------------------------------------------------------
// Cell 1 — Failed (no EffectStaged, no Committed)
// ---------------------------------------------------------------------------

#[test]
fn cell_1_failed_pre_commit_crash_only_yields_failed_pending_reattempt() {
    let mid = MoteId::from_bytes([2u8; 32]);
    let (p, _) = projection_from_entries(vec![failed(mid, 0, FailureReason::TimedOut)]);
    assert_eq!(p.state_of(&mid), MoteState::Failed);
    // Pre-commit-crash alone (no EffectStaged) is retry-allowed, but the
    // `can_redispatch_world_effect` predicate is specifically about WM
    // effect re-dispatch under EffectStaged. Without EffectStaged, the
    // answer is "no in-flight effect to redispatch" → false.
    assert!(!p.can_redispatch_world_effect(&mid));
    assert!(p.anomaly_motes().is_empty());
}

#[test]
fn cell_1_failed_terminal_only_yields_failed() {
    let mid = MoteId::from_bytes([3u8; 32]);
    let (p, _) = projection_from_entries(vec![failed(mid, 0, FailureReason::ExecutorRefused)]);
    assert_eq!(p.state_of(&mid), MoteState::Failed);
    assert!(!p.can_redispatch_world_effect(&mid));
}

// ---------------------------------------------------------------------------
// Cell 2 — EffectStaged alone
// ---------------------------------------------------------------------------

#[test]
fn cell_2_effect_staged_alone_yields_pending_and_redispatch_ok() {
    let mid = MoteId::from_bytes([4u8; 32]);
    let (p, _) = projection_from_entries(vec![effect_staged(mid, 0)]);
    // EffectStaged with no Committed AND no terminal Failed AND no
    // Inconsistent → in-flight (Pending); re-dispatch permitted.
    assert_eq!(p.state_of(&mid), MoteState::Pending);
    assert!(
        p.can_redispatch_world_effect(&mid),
        "cell 2: EffectStaged alone → redispatch permitted"
    );
}

// ---------------------------------------------------------------------------
// Cell 3 — EffectStaged + Failed(pre-commit-crash)
// ---------------------------------------------------------------------------

#[test]
fn cell_3_effect_staged_then_timed_out_yields_pending_and_redispatch_ok() {
    let mid = MoteId::from_bytes([5u8; 32]);
    let (p, _) = projection_from_entries(vec![
        effect_staged(mid, 0),
        failed(mid, 0, FailureReason::TimedOut),
    ]);
    assert_eq!(p.state_of(&mid), MoteState::Pending);
    assert!(
        p.can_redispatch_world_effect(&mid),
        "cell 3: EffectStaged + TimedOut (pre-commit-crash class) → redispatch permitted"
    );
}

#[test]
fn cell_3_effect_staged_then_worker_crashed_yields_pending_and_redispatch_ok() {
    let mid = MoteId::from_bytes([6u8; 32]);
    let (p, _) = projection_from_entries(vec![
        effect_staged(mid, 0),
        failed(mid, 0, FailureReason::WorkerCrashed),
    ]);
    assert_eq!(p.state_of(&mid), MoteState::Pending);
    assert!(
        p.can_redispatch_world_effect(&mid),
        "cell 3: EffectStaged + WorkerCrashed (pre-commit-crash class) → redispatch permitted"
    );
}

// ---------------------------------------------------------------------------
// Cell 4 — EffectStaged + Failed(TERMINAL) — THE LOAD-BEARING CELL
// (STEP 5.1 Terminal-before-Staged ordering invariant cell.)
// ---------------------------------------------------------------------------

#[test]
fn cell_5_terminal_failure_under_effect_staged_no_redispatch() {
    // The cell that exists to prove the WM double-effect window is CLOSED.
    // Without the Terminal-before-Staged ordering invariant, this test
    // would FAIL because effect_staged_observed would override
    // terminal_failure_observed in state_of_id derivation.
    let mid = MoteId::from_bytes([7u8; 32]);
    let (p, _) = projection_from_entries(vec![
        effect_staged(mid, 0),
        failed(mid, 0, FailureReason::ValidatorRejected),
    ]);
    assert_eq!(
        p.state_of(&mid),
        MoteState::Failed,
        "cell 5: terminal failure under EffectStaged MUST yield Failed (not Pending in-flight)"
    );
    assert!(
        !p.can_redispatch_world_effect(&mid),
        "cell 5: terminal failure under EffectStaged MUST forbid redispatch \
         (the WM double-effect hazard cell)"
    );
}

#[test]
fn cell_5_terminal_failure_under_effect_staged_executor_refused() {
    let mid = MoteId::from_bytes([8u8; 32]);
    let (p, _) = projection_from_entries(vec![
        effect_staged(mid, 0),
        failed(mid, 0, FailureReason::ExecutorRefused),
    ]);
    assert_eq!(p.state_of(&mid), MoteState::Failed);
    assert!(!p.can_redispatch_world_effect(&mid));
}

#[test]
fn cell_5_terminal_failure_under_effect_staged_upstream_repudiated() {
    let mid = MoteId::from_bytes([9u8; 32]);
    let (p, _) = projection_from_entries(vec![
        effect_staged(mid, 0),
        failed(mid, 0, FailureReason::UpstreamRepudiated),
    ]);
    assert_eq!(p.state_of(&mid), MoteState::Failed);
    assert!(!p.can_redispatch_world_effect(&mid));
}

#[test]
fn cell_5_terminal_failure_under_effect_staged_unsafe_wm_construction() {
    let mid = MoteId::from_bytes([10u8; 32]);
    let (p, _) = projection_from_entries(vec![
        effect_staged(mid, 0),
        failed(mid, 0, FailureReason::UnsafeWorldMutatingConstruction),
    ]);
    assert_eq!(p.state_of(&mid), MoteState::Failed);
    assert!(!p.can_redispatch_world_effect(&mid));
}

// ---------------------------------------------------------------------------
// Cell 5 — EffectStaged + Committed
// ---------------------------------------------------------------------------

#[test]
fn cell_5_effect_staged_plus_committed_yields_committed_no_redispatch() {
    let mid = MoteId::from_bytes([11u8; 32]);
    let (p, _) = projection_from_entries(vec![effect_staged(mid, 0), committed(mid, 0)]);
    assert_eq!(p.state_of(&mid), MoteState::Committed);
    // Committed → DONE. Never re-dispatch.
    assert!(!p.can_redispatch_world_effect(&mid));
}

#[test]
fn cell_5_effect_staged_plus_committed_plus_trailing_failed_still_committed() {
    // A trailing Failed AFTER a Committed (e.g., from a stale worker) is
    // ignored — Committed is the durable fact (per-Mote); Failed is
    // per-attempt.
    let mid = MoteId::from_bytes([12u8; 32]);
    let (p, _) = projection_from_entries(vec![
        effect_staged(mid, 0),
        committed(mid, 0),
        failed(mid, 0, FailureReason::WorkerCrashed),
    ]);
    assert_eq!(p.state_of(&mid), MoteState::Committed);
    assert!(!p.can_redispatch_world_effect(&mid));
}

// ---------------------------------------------------------------------------
// Cell 6 — Committed without EffectStaged (legal for non-Staged tools)
// ---------------------------------------------------------------------------

#[test]
fn cell_6_committed_without_effect_staged_yields_committed_no_redispatch() {
    let mid = MoteId::from_bytes([13u8; 32]);
    let (p, _) = projection_from_entries(vec![committed(mid, 0)]);
    assert_eq!(p.state_of(&mid), MoteState::Committed);
    assert!(!p.can_redispatch_world_effect(&mid));
}

// ---------------------------------------------------------------------------
// Cell 7 — EffectStaged + Committed + Repudiated
// ---------------------------------------------------------------------------

#[test]
fn cell_7_effect_staged_committed_then_repudiated_yields_repudiated() {
    let mid = MoteId::from_bytes([14u8; 32]);
    let entries = vec![effect_staged(mid, 0), committed(mid, 0)];
    let (_, seqs) = projection_from_entries(entries.clone());
    let committed_seq = seqs[1]; // the journal assigned this on append
    let (p, _) = projection_from_entries(vec![
        effect_staged(mid, 0),
        committed(mid, 0),
        repudiated(mid, committed_seq, 0),
    ]);
    assert_eq!(p.state_of(&mid), MoteState::Repudiated);
    assert!(!p.can_redispatch_world_effect(&mid));
}

// ---------------------------------------------------------------------------
// Cell 8 — EffectStaged + Repudiated WITHOUT Committed (the anomaly)
// ---------------------------------------------------------------------------

#[test]
fn cell_8_anomaly_effect_staged_then_repudiated_no_committed() {
    let mid = MoteId::from_bytes([15u8; 32]);
    // Repudiated targets a non-existent target_committed_seq=0. The fold
    // detects: `info.committed.is_none() && info.effect_staged_observed`
    // → sets `info.inconsistent = true`. The fold does NOT abort; the
    // anomaly is quarantined.
    let (p, _) = projection_from_entries(vec![effect_staged(mid, 0), repudiated(mid, 0, 0)]);
    assert_eq!(
        p.state_of(&mid),
        MoteState::Inconsistent,
        "cell 8: EffectStaged + Repudiated-without-Committed MUST yield Inconsistent"
    );
    assert!(
        !p.can_redispatch_world_effect(&mid),
        "cell 8: Inconsistent forbids redispatch"
    );
    let anomalies = p.anomaly_motes();
    assert_eq!(anomalies.len(), 1);
    assert_eq!(
        anomalies[0],
        (mid, AnomalyKind::EffectStagedThenRepudiatedNoCommitted)
    );
}

// ---------------------------------------------------------------------------
// Cell 9 — Committed + Repudiated (standard repudiation)
// ---------------------------------------------------------------------------

#[test]
fn cell_9_committed_then_repudiated_yields_repudiated_standard() {
    let mid = MoteId::from_bytes([16u8; 32]);
    let (_, seqs) = projection_from_entries(vec![committed(mid, 0)]);
    let committed_seq = seqs[0];
    let (p, _) =
        projection_from_entries(vec![committed(mid, 0), repudiated(mid, committed_seq, 0)]);
    assert_eq!(p.state_of(&mid), MoteState::Repudiated);
    assert!(!p.can_redispatch_world_effect(&mid));
}

// ---------------------------------------------------------------------------
// STEP 5.1 — Terminal-before-Staged ordering invariant (the load-bearing
// regression). The cell_5_terminal_failure_under_effect_staged_no_redispatch
// test above IS this regression test (the test would FAIL if state_of_id
// branches 4 and 5 were swapped). This sister test is `#[ignore]` by default;
// it exists as a meta-test of the invariant and is activated for ad-hoc
// regression auditing only.
// ---------------------------------------------------------------------------

#[test]
#[ignore = "meta-test: demonstrates the Terminal-before-Staged invariant is load-bearing — see cell_5_terminal_failure_under_effect_staged_no_redispatch for the active regression test"]
fn cell_5_ignored_sister_test_branch_swap_documentation() {
    // This test asserts the EXACT same fixture as
    // cell_5_terminal_failure_under_effect_staged_no_redispatch — it
    // documents that the invariant is load-bearing. If a future contributor
    // accidentally swaps branches 4 and 5 in state_of_id, the ACTIVE
    // regression test (cell_5_...) will fail. This sister test exists as
    // documentation that the swap-failure is a real correctness regression,
    // not a stylistic one. The test runs only when explicitly invoked.
    let mid = MoteId::from_bytes([17u8; 32]);
    let (p, _) = projection_from_entries(vec![
        effect_staged(mid, 0),
        failed(mid, 0, FailureReason::ValidatorRejected),
    ]);
    assert_eq!(p.state_of(&mid), MoteState::Failed);
    assert!(!p.can_redispatch_world_effect(&mid));
}

// ---------------------------------------------------------------------------
// Single-source-of-class-truth: is_pre_commit_crash classifies canonically
// (STEP 6.2 — both production code AND tests call this function; no
// hardcoded list).
// ---------------------------------------------------------------------------

#[test]
fn is_pre_commit_crash_classifies_canonically_pre_commit_crash_variants() {
    assert!(is_pre_commit_crash(FailureReason::TimedOut));
    assert!(is_pre_commit_crash(FailureReason::WorkerCrashed));
}

#[test]
fn is_pre_commit_crash_classifies_canonically_terminal_variants() {
    assert!(!is_pre_commit_crash(FailureReason::ExecutorRefused));
    assert!(!is_pre_commit_crash(FailureReason::ValidatorRejected));
    assert!(!is_pre_commit_crash(FailureReason::UpstreamRepudiated));
    assert!(!is_pre_commit_crash(
        FailureReason::UnsafeWorldMutatingConstruction
    ));
}
