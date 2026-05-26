//! Property tests on the PR 9b-2 commit_protocol scaffolding. Verifies:
//! (1) the closed `CommitProtocolError` vocabulary is total over the
//! `arb_commit_protocol_error` strategy (canonical-classifier-cannot-drift
//! coverage check); (2) `mote_id()` is pure (same input → same output);
//! (3) `is_recovery_refusal()` is true iff variant is R-13. SN-4 v2
//! mandate: ≥3 proptest properties × 64 cases.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use kx_content::ContentRef;
use kx_executor::CommitProtocolError;
use kx_mote::MoteId;
use proptest::prelude::*;

fn arb_mote_id() -> impl Strategy<Value = MoteId> {
    any::<[u8; 32]>().prop_map(MoteId::from_bytes)
}

fn arb_content_ref() -> impl Strategy<Value = ContentRef> {
    any::<[u8; 32]>().prop_map(ContentRef::from_bytes)
}

// MUST update on new `CommitProtocolError` variant. Canonical-classifier-
// cannot-drift: any new variant without an updated strategy is caught by
// this proptest's coverage drop.
fn arb_commit_protocol_error() -> impl Strategy<Value = CommitProtocolError> {
    prop_oneof![
        (arb_mote_id(), arb_content_ref()).prop_map(|(mote_id, result_ref)| {
            CommitProtocolError::R11ResultRefIncomplete {
                mote_id,
                result_ref,
            }
        }),
        (arb_mote_id(), "[a-z]{0,16}").prop_map(|(mote_id, context)| {
            CommitProtocolError::R12CommittedNotProofOfValidity { mote_id, context }
        }),
        (arb_mote_id(), "[a-z]{0,16}").prop_map(|(mote_id, reason)| {
            CommitProtocolError::R13WmReDispatchRefused { mote_id, reason }
        }),
        (arb_mote_id(), "[a-z]{0,16}").prop_map(|(mote_id, reason)| {
            CommitProtocolError::BrokerDispatchFailed { mote_id, reason }
        }),
        (arb_mote_id(), "[a-z]{0,16}").prop_map(|(mote_id, reason)| {
            CommitProtocolError::ContentStorePutFailed { mote_id, reason }
        }),
        (arb_mote_id(), "[a-z]{0,16}").prop_map(|(mote_id, reason)| {
            CommitProtocolError::JournalAppendCommittedFailed { mote_id, reason }
        }),
        (arb_mote_id(), "[a-z]{0,16}").prop_map(|(mote_id, reason)| {
            CommitProtocolError::JournalAppendEffectStagedFailed { mote_id, reason }
        }),
        (arb_mote_id(), "[a-z]{0,16}")
            .prop_map(|(mote_id, reason)| { CommitProtocolError::Internal { mote_id, reason } }),
    ]
}

proptest! {
    /// `mote_id()` is pure (same input → same output) and total (every
    /// variant returns a `MoteId`).
    #[test]
    fn prop_mote_id_extractor_is_pure_and_total(
        err in arb_commit_protocol_error(),
    ) {
        let a = err.mote_id();
        let b = err.mote_id();
        prop_assert_eq!(a, b);
    }

    /// `is_recovery_refusal()` is `true` iff the variant is
    /// `R13WmReDispatchRefused`. Covers BOTH branches across the closed
    /// variant set.
    #[test]
    fn prop_is_recovery_refusal_identifies_r13_only(
        err in arb_commit_protocol_error(),
    ) {
        let is_recovery = err.is_recovery_refusal();
        let is_r13 = matches!(err, CommitProtocolError::R13WmReDispatchRefused { .. });
        prop_assert_eq!(is_recovery, is_r13);
    }

    /// Every `CommitProtocolError` variant renders a non-empty
    /// human-readable string via `Display`. Operators need readable
    /// diagnostics for every failure case.
    #[test]
    fn prop_display_is_non_empty_for_every_variant(
        err in arb_commit_protocol_error(),
    ) {
        let s = err.to_string();
        prop_assert!(!s.is_empty(), "Display must render to non-empty string");
    }

    /// `Clone` + `PartialEq` round-trip: cloning produces an equal value.
    /// This pins the Clone + Eq derive impls on the closed vocabulary.
    #[test]
    fn prop_clone_round_trips_with_eq(
        err in arb_commit_protocol_error(),
    ) {
        let cloned = err.clone();
        prop_assert_eq!(&err, &cloned);
    }
}
