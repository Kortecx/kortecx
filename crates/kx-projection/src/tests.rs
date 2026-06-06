//! Inline unit tests for kx-projection. Extracted per Rule 3 with bodies
//! unchanged.

use kx_content::ContentRef;
use kx_journal::{FailureReason, JournalEntry, RepudiationReason};
use kx_mote::{EffectPattern, MoteDefHash, MoteId, NdClass};
use smallvec::SmallVec;

use super::*;

fn mid(b: u8) -> MoteId {
    MoteId::from_bytes([b; 32])
}

fn cref(b: u8) -> ContentRef {
    ContentRef::from_bytes([b; 32])
}

fn dh(b: u8) -> MoteDefHash {
    MoteDefHash::from_bytes([b; 32])
}

fn proposed_entry(mote_byte: u8, seq: u64) -> JournalEntry {
    JournalEntry::Proposed {
        mote_id: mid(mote_byte),
        idempotency_key: [mote_byte; 32],
        seq,
        nondeterminism: NdClass::Pure,
        placement_hint: 0,
        warrant_ref: ContentRef::from_bytes([0xaa; 32]),
    }
}

fn committed_entry(mote_byte: u8, seq: u64, nd: NdClass) -> JournalEntry {
    JournalEntry::Committed {
        mote_id: mid(mote_byte),
        idempotency_key: [mote_byte; 32],
        seq,
        nondeterminism: nd,
        result_ref: cref(mote_byte),
        parents: SmallVec::new(),
        warrant_ref: ContentRef::from_bytes([0xaa; 32]),
        mote_def_hash: dh(mote_byte),
    }
}

fn failed_entry(mote_byte: u8, seq: u64) -> JournalEntry {
    JournalEntry::Failed {
        mote_id: mid(mote_byte),
        idempotency_key: [mote_byte; 32],
        seq,
        reason_class: FailureReason::TimedOut,
        reporter_id: 0,
    }
}

fn repudiated_entry(target_byte: u8, target_seq: u64, seq: u64) -> JournalEntry {
    JournalEntry::Repudiated {
        target_mote_id: mid(target_byte),
        idempotency_key: [0u8; 32], // would be derived; for in-memory fold the byte content doesn't matter
        seq,
        target_committed_seq: target_seq,
        reason_class: RepudiationReason::OperatorAction,
        repudiator_id: 0,
    }
}

#[test]
fn empty_projection_is_pending_for_unknown_motes() {
    let p = Projection::new();
    assert_eq!(p.state_of(&mid(1)), MoteState::Pending);
    assert!(p.is_empty());
}

#[test]
fn proposed_then_committed_collapses_to_committed() {
    let mut p = Projection::new();
    p.fold(&proposed_entry(1, 1)).unwrap();
    assert_eq!(p.state_of(&mid(1)), MoteState::Scheduled);
    p.fold(&committed_entry(1, 2, NdClass::Pure)).unwrap();
    assert_eq!(p.state_of(&mid(1)), MoteState::Committed);
}

#[test]
fn failed_then_proposed_resets_to_scheduled() {
    let mut p = Projection::new();
    p.fold(&proposed_entry(1, 1)).unwrap();
    p.fold(&failed_entry(1, 2)).unwrap();
    assert_eq!(p.state_of(&mid(1)), MoteState::Failed);
    p.fold(&proposed_entry(1, 3)).unwrap();
    assert_eq!(p.state_of(&mid(1)), MoteState::Scheduled);
}

// PR-3 (AL2): the read-side failure-reason channel a model-driven re-plan reads.
fn failed_entry_with_reason(mote_byte: u8, seq: u64, reason: FailureReason) -> JournalEntry {
    JournalEntry::Failed {
        mote_id: mid(mote_byte),
        idempotency_key: [mote_byte; 32],
        seq,
        reason_class: reason,
        reporter_id: 0,
    }
}

#[test]
fn failure_reason_of_exposes_terminal_reason_and_defaults_none() {
    let mut p = Projection::new();
    // A terminal-logic dead-letter (NOT a pre-commit-crash) retains its reason.
    p.fold(&proposed_entry(1, 1)).unwrap();
    p.fold(&failed_entry_with_reason(
        1,
        2,
        FailureReason::ExecutorRefused,
    ))
    .unwrap();
    assert_eq!(p.state_of(&mid(1)), MoteState::Failed);
    assert_eq!(
        p.failure_reason_of(&mid(1)),
        Some(FailureReason::ExecutorRefused)
    );

    // A pre-commit-crash reason (TimedOut/WorkerCrashed) is NOT a terminal
    // dead-letter reason — it stays None (matches `terminal_failure_observed`).
    p.fold(&proposed_entry(2, 3)).unwrap();
    p.fold(&failed_entry_with_reason(2, 4, FailureReason::TimedOut))
        .unwrap();
    assert_eq!(p.failure_reason_of(&mid(2)), None);

    // A never-failed / unknown Mote is None; a committed Mote is None.
    assert_eq!(p.failure_reason_of(&mid(9)), None);
    p.fold(&committed_entry(3, 5, NdClass::Pure)).unwrap();
    assert_eq!(p.failure_reason_of(&mid(3)), None);
}

#[test]
fn failure_reason_is_prefix_monotonic_first_terminal_wins() {
    // Two terminal Failed entries: the FIRST terminal reason is retained.
    let mut p = Projection::new();
    p.fold(&proposed_entry(1, 1)).unwrap();
    p.fold(&failed_entry_with_reason(
        1,
        2,
        FailureReason::ExecutorRefused,
    ))
    .unwrap();
    p.fold(&failed_entry_with_reason(
        1,
        3,
        FailureReason::ValidatorRejected,
    ))
    .unwrap();
    assert_eq!(
        p.failure_reason_of(&mid(1)),
        Some(FailureReason::ExecutorRefused)
    );
}

#[test]
fn repudiated_only_applies_when_target_committed_seq_matches() {
    let mut p = Projection::new();
    p.fold(&committed_entry(1, 5, NdClass::Pure)).unwrap();
    // Wrong target_committed_seq — projection ignores
    p.fold(&repudiated_entry(1, 99, 6)).unwrap();
    assert_eq!(p.state_of(&mid(1)), MoteState::Committed);
    // Correct target_committed_seq
    p.fold(&repudiated_entry(1, 5, 7)).unwrap();
    assert_eq!(p.state_of(&mid(1)), MoteState::Repudiated);
}

#[test]
fn duplicate_committed_for_same_mote_id_surfaces_loudly() {
    let mut p = Projection::new();
    p.fold(&committed_entry(1, 1, NdClass::Pure)).unwrap();
    let result = p.fold(&committed_entry(1, 2, NdClass::Pure));
    assert!(matches!(
        result,
        Err(ProjectionError::DuplicateCommitted(_))
    ));
}

#[test]
fn last_seq_advances_monotonically() {
    let mut p = Projection::new();
    p.fold(&proposed_entry(1, 1)).unwrap();
    p.fold(&proposed_entry(2, 2)).unwrap();
    p.fold(&committed_entry(1, 3, NdClass::Pure)).unwrap();
    assert_eq!(p.current_seq(), 3);
}

#[test]
fn register_mote_makes_it_pending() {
    let mut p = Projection::new();
    p.register_mote(RegisterMote {
        mote_id: mid(1),
        nd_class: NdClass::Pure,
        effect_pattern: EffectPattern::IdempotentByConstruction,
        critic_for: None,
        is_topology_shaper: false,
        parents: SmallVec::new(),
        warrant_ref: kx_content::ContentRef::from_bytes([0xaa; 32]),
    });
    assert_eq!(p.state_of(&mid(1)), MoteState::Pending);
}

#[test]
fn snapshot_is_immutable_under_subsequent_folds() {
    let mut p = Projection::new();
    p.fold(&committed_entry(1, 1, NdClass::Pure)).unwrap();
    let snap = p.snapshot();
    assert_eq!(snap.state_of(&mid(1)), MoteState::Committed);
    // Mutate the projection — snapshot must NOT change
    p.fold(&repudiated_entry(1, 1, 2)).unwrap();
    assert_eq!(snap.state_of(&mid(1)), MoteState::Committed); // unchanged
    assert_eq!(p.state_of(&mid(1)), MoteState::Repudiated); // updated
}

#[test]
fn promotion_state_is_not_applicable_in_p1() {
    let mut p = Projection::new();
    p.fold(&committed_entry(1, 1, NdClass::WorldMutating))
        .unwrap();
    // Per D18 P1 default — even WM motes are NotApplicable until the
    // executor (P1.9) wires the MoteDef registry.
    assert_eq!(p.promotion_state(&mid(1)), PromotionState::NotApplicable);
}

#[test]
fn state_of_for_non_existent_target_of_repudiation_remains_pending() {
    let mut p = Projection::new();
    // Repudiate a MoteId that was never committed — projection records nothing
    // observable via state_of (per projection.md §5).
    p.fold(&repudiated_entry(1, 99, 1)).unwrap();
    assert_eq!(p.state_of(&mid(1)), MoteState::Pending);
}
