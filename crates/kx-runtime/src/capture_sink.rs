//! [`CaptureSink`] — the D67 on-by-default step-capture seam.
//!
//! The orchestrator ([`crate::run_with_seams`]) owns the live
//! [`kx_projection::Projection`]; immediately AFTER each Mote's `Committed` entry
//! folds in, it records that Mote's **action** (its committed `result_ref`) into a
//! disposable [`kx_capture::InMemoryCaptureStore`] behind this handle. Capture is
//! the reliability/resumability story (D67): an in-memory `MoteId → result_ref`
//! ledger over truth, queryable for resume/compensation without re-reading the
//! whole journal.
//!
//! Like [`crate::SnapshotSink`] it is **OFF THE TRUTH PATH**: capture is never
//! journaled, is never a `MoteId` input, and never gates scheduling / promotion /
//! eviction (the dependency wall in `kx-capture/tests/boundary.rs` is the
//! compiler-enforced tripwire). Turning it ON changes only the *quantity
//! captured*, never its trust status — so the canonical product digest
//! `a6b5c679…` is byte-unchanged.
//!
//! It is an **additive** seam: [`crate::run_with_seams`] takes `Option<&CaptureSink>`;
//! `None` disables capture (the byte-identity-without-overhead path), and the
//! canonical demo [`crate::run`] passes `Some` with [`CaptureScope::ActionsOnly`]
//! (the privacy-safe default — the D67 flip). The runtime seam only ever records
//! the **action** ([`StepRecord::action`]); reasoning/thinking enrichment under
//! [`CaptureScope::Full`] is the real-model harness's job (and PII-gated in M3.2).
//!
//! [`CaptureScope::ActionsOnly`]: kx_capture::CaptureScope::ActionsOnly
//! [`CaptureScope::Full`]: kx_capture::CaptureScope::Full

use std::sync::{Arc, PoisonError, RwLock};

use kx_capture::{CaptureConsent, InMemoryCaptureStore, StepRecord};

/// A cheap, cloneable handle over a disposable [`InMemoryCaptureStore`] (D67).
///
/// Clones share one slot (an `Arc<RwLock<…>>`), so the orchestrator and any
/// reader observe the same store. Consent is fixed at construction
/// ([`kx_capture::CaptureScope::ActionsOnly`] the safe default;
/// [`kx_capture::CaptureScope::Full`] opt-in-with-consent) and re-enforced inside
/// the store on every [`Self::record`].
#[derive(Clone, Debug)]
pub struct CaptureSink {
    store: Arc<RwLock<InMemoryCaptureStore>>,
}

impl CaptureSink {
    /// A new sink retaining at most what `consent` allows.
    #[must_use]
    pub fn new(consent: CaptureConsent) -> Self {
        Self {
            store: Arc::new(RwLock::new(InMemoryCaptureStore::new(consent))),
        }
    }

    /// The on-by-default safe sink: [`kx_capture::CaptureScope::ActionsOnly`]
    /// (retain only each committed action's `result_ref` — a 32-byte join key
    /// back to truth; reasoning/thinking/input are never retained).
    #[must_use]
    pub fn actions_only() -> Self {
        Self::new(CaptureConsent::actions_only())
    }

    /// Record (or overwrite) a Mote's step. The store enforces consent at the
    /// boundary: a non-`Full` session keeps only the action join key, stripping
    /// reasoning/thinking/input even if supplied. Overwrite-by-`MoteId` makes a
    /// re-record of an already-captured Mote idempotent.
    ///
    /// Lock poisoning is recovered (never propagated): capture is non-authoritative
    /// off-truth-path exhaust, so a prior panic-while-recording must not crash the
    /// runtime — the worst case is a missing-by-one record.
    pub fn record(&self, rec: StepRecord) {
        self.store
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .record(rec);
    }

    /// A clone of the current store (a point-in-time snapshot of all captured
    /// steps). The caller holds no lock across iteration. Lock poisoning is
    /// recovered (see [`Self::record`]).
    #[must_use]
    pub fn store(&self) -> InMemoryCaptureStore {
        self.store
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Number of captured steps (cheap; avoids cloning the store). Lock poisoning
    /// is recovered (see [`Self::record`]).
    #[must_use]
    pub fn len(&self) -> usize {
        self.store
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    /// `true` when nothing is captured yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use kx_capture::CaptureScope;
    use kx_content::ContentRef;
    use kx_mote::MoteId;

    use super::*;

    fn mid(b: u8) -> MoteId {
        MoteId::from_bytes([b; 32])
    }
    fn cref(b: u8) -> ContentRef {
        ContentRef::from_bytes([b; 32])
    }

    #[test]
    fn actions_only_sink_records_committed_action() {
        let sink = CaptureSink::actions_only();
        assert!(sink.is_empty());
        sink.record(StepRecord::action(mid(7), cref(9)));
        assert_eq!(sink.len(), 1);
        let store = sink.store();
        let got = store.get(&mid(7)).expect("recorded");
        assert_eq!(got.output_ref, Some(cref(9)));
    }

    #[test]
    fn actions_only_strips_opt_in_fields_at_the_sink() {
        let sink = CaptureSink::actions_only();
        // Supply a full record; the store must strip the opt-in fields because
        // the session did not consent to `Full`.
        sink.record(StepRecord::full(
            mid(3),
            Some(cref(1)),
            Some(cref(2)),
            Some(cref(3)),
            Some(cref(4)),
        ));
        let store = sink.store();
        let got = store.get(&mid(3)).expect("recorded");
        assert_eq!(got.output_ref, Some(cref(2)));
        assert_eq!(got.input_ref, None);
        assert_eq!(got.reasoning_ref, None);
        assert_eq!(got.thinking_ref, None);
    }

    #[test]
    fn full_sink_retains_all_fields() {
        // `Full` is reachable + testable at the seam without a config knob
        // (the CLI/config override is M3.2).
        let sink = CaptureSink::new(CaptureConsent::full());
        assert!(sink.store().consent().captures_steps());
        sink.record(StepRecord::full(
            mid(5),
            Some(cref(1)),
            Some(cref(2)),
            Some(cref(3)),
            Some(cref(4)),
        ));
        let store = sink.store();
        let got = store.get(&mid(5)).expect("recorded");
        assert_eq!(got.input_ref, Some(cref(1)));
        assert_eq!(got.output_ref, Some(cref(2)));
        assert_eq!(got.reasoning_ref, Some(cref(3)));
        assert_eq!(got.thinking_ref, Some(cref(4)));
    }

    #[test]
    fn new_uses_the_supplied_scope() {
        let sink = CaptureSink::new(CaptureConsent {
            scope: CaptureScope::ActionsOnly,
        });
        assert!(!sink.store().consent().captures_steps());
    }

    #[test]
    fn record_overwrites_by_mote_id() {
        let sink = CaptureSink::actions_only();
        sink.record(StepRecord::action(mid(2), cref(1)));
        sink.record(StepRecord::action(mid(2), cref(8)));
        assert_eq!(sink.len(), 1);
        assert_eq!(
            sink.store().get(&mid(2)).expect("recorded").output_ref,
            Some(cref(8))
        );
    }

    #[test]
    fn clone_shares_one_slot() {
        let sink = CaptureSink::actions_only();
        let clone = sink.clone();
        clone.record(StepRecord::action(mid(1), cref(1)));
        // The original observes the clone's write (shared Arc slot).
        assert_eq!(sink.len(), 1);
        assert!(sink.store().get(&mid(1)).is_some());
    }

    #[test]
    fn poisoning_is_recovered() {
        let sink = CaptureSink::actions_only();
        let clone = sink.clone();
        // Poison the lock from a panicking thread holding the write guard.
        let _ = std::thread::spawn(move || {
            let _guard = clone.store.write().unwrap();
            panic!("poison the capture lock");
        })
        .join();
        // Subsequent record/read still succeed (poison recovered, never propagated).
        sink.record(StepRecord::action(mid(4), cref(4)));
        assert_eq!(sink.len(), 1);
    }
}
