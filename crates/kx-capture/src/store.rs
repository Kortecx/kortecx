//! The capture store — a disposable, rebuildable projection keyed by `MoteId`.
//!
//! Dropping it loses only the opt-in analysis exhaust; every committed **action**
//! still lives on the journal + content store. A persistent / cloud-backed store
//! is a clean forward seam (the same shape, a different backend); the in-memory
//! store is the single-node default.

use std::collections::BTreeMap;

use kx_mote::MoteId;

use crate::record::StepRecord;
use crate::scope::CaptureConsent;

/// An in-memory step-capture projection. Deterministic iteration (`BTreeMap`),
/// `&mut self` mutation (mirrors `kx_dataset::AnnotationStore` — no interior
/// mutability, no lock). Consent is fixed per session and enforced on every
/// `record`: under [`crate::CaptureScope::ActionsOnly`] the opt-in fields
/// (input/reasoning/thinking) are stripped before insertion, so disabling
/// consent cannot retain reasoning/thinking even if a caller supplies it.
#[derive(Debug, Clone)]
pub struct InMemoryCaptureStore {
    consent: CaptureConsent,
    by_mote: BTreeMap<MoteId, StepRecord>,
}

impl InMemoryCaptureStore {
    /// A new store retaining at most what `consent` allows.
    #[must_use]
    pub fn new(consent: CaptureConsent) -> Self {
        Self {
            consent,
            by_mote: BTreeMap::new(),
        }
    }

    /// The session's consent scope.
    #[must_use]
    pub const fn consent(&self) -> CaptureConsent {
        self.consent
    }

    /// Record (or overwrite) a Mote's step, enforcing consent: a non-`Full`
    /// session retains only the action join key (the opt-in fields are stripped
    /// at the boundary, never stored).
    pub fn record(&mut self, rec: StepRecord) {
        let stored = if self.consent.captures_steps() {
            rec
        } else {
            rec.actions_only()
        };
        self.by_mote.insert(stored.mote_id, stored);
    }

    /// The captured step for `mote_id`, if any.
    #[must_use]
    pub fn get(&self, mote_id: &MoteId) -> Option<&StepRecord> {
        self.by_mote.get(mote_id)
    }

    /// Drop a Mote's captured step (e.g. on a retention-TTL sweep or a
    /// user-initiated erase). Returns the removed record.
    pub fn forget(&mut self, mote_id: &MoteId) -> Option<StepRecord> {
        self.by_mote.remove(mote_id)
    }

    /// Number of captured steps.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_mote.len()
    }

    /// `true` when nothing is captured.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_mote.is_empty()
    }

    /// Deterministic iteration over captured steps (by `MoteId` order).
    pub fn iter(&self) -> impl Iterator<Item = (&MoteId, &StepRecord)> {
        self.by_mote.iter()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use kx_content::ContentRef;

    use super::*;

    fn mid(b: u8) -> MoteId {
        MoteId::from_bytes([b; 32])
    }
    fn cref(b: u8) -> ContentRef {
        ContentRef::from_bytes([b; 32])
    }

    fn full_rec(b: u8) -> StepRecord {
        StepRecord::full(
            mid(b),
            Some(cref(1)),
            Some(cref(2)),
            Some(cref(3)),
            Some(cref(4)),
        )
    }

    #[test]
    fn actions_only_strips_reasoning_and_thinking() {
        let mut s = InMemoryCaptureStore::new(CaptureConsent::actions_only());
        s.record(full_rec(7));
        let got = s.get(&mid(7)).expect("recorded");
        // The action join key is retained; the opt-in fields are stripped.
        assert_eq!(got.output_ref, Some(cref(2)));
        assert_eq!(got.input_ref, None);
        assert_eq!(got.reasoning_ref, None);
        assert_eq!(got.thinking_ref, None);
    }

    #[test]
    fn full_consent_retains_all_fields() {
        let mut s = InMemoryCaptureStore::new(CaptureConsent::full());
        s.record(full_rec(7));
        let got = s.get(&mid(7)).expect("recorded");
        assert_eq!(got.input_ref, Some(cref(1)));
        assert_eq!(got.output_ref, Some(cref(2)));
        assert_eq!(got.reasoning_ref, Some(cref(3)));
        assert_eq!(got.thinking_ref, Some(cref(4)));
    }

    #[test]
    fn default_scope_is_actions_only() {
        assert_eq!(CaptureConsent::default(), CaptureConsent::actions_only());
        assert!(!CaptureConsent::default().captures_steps());
    }

    #[test]
    fn forget_erases_a_step() {
        let mut s = InMemoryCaptureStore::new(CaptureConsent::full());
        s.record(full_rec(7));
        assert_eq!(s.len(), 1);
        let removed = s.forget(&mid(7)).expect("present");
        assert_eq!(removed.mote_id, mid(7));
        assert!(s.is_empty());
    }

    #[test]
    fn action_helper_holds_only_the_action() {
        let r = StepRecord::action(mid(1), cref(9));
        assert_eq!(r.output_ref, Some(cref(9)));
        assert_eq!(r.input_ref, None);
        assert_eq!(r.reasoning_ref, None);
        assert_eq!(r.thinking_ref, None);
    }
}
