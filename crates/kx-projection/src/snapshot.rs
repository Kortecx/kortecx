//! [`Snapshot`] — an immutable point-in-time view of the projection. Implements
//! the same 7-method read API as [`crate::Projection`] with stable snapshot
//! semantics.

use kx_content::ContentRef;
use kx_mote::{EdgeMeta, MoteId, NdClass};
use smallvec::SmallVec;

use crate::enums::{AnomalyKind, MoteState, PromotionState};
use crate::helpers::{promotion_state_impl, ready_set_impl, transitive_consumers_impl};
use crate::state::State;

/// An immutable point-in-time view of a [`crate::Projection`].
///
/// Returned by [`crate::Projection::snapshot`]. Subsequent folds against the source
/// projection do not affect this snapshot — the snapshot-isolation contract from
/// D16 / `projection.md` §6 is provided by cloning the underlying state.
///
/// # Examples
///
/// A snapshot remains stable while the projection mutates underneath:
///
/// ```
/// use kx_journal::{FailureReason, JournalEntry, RepudiationReason};
/// use kx_mote::{MoteDefHash, MoteId, NdClass};
/// use kx_projection::{MoteState, Projection};
/// use kx_content::ContentRef;
/// use smallvec::SmallVec;
///
/// let mut p = Projection::new();
/// p.fold(&JournalEntry::Committed {
///     mote_id: MoteId::from_bytes([1u8; 32]),
///     idempotency_key: [1u8; 32],
///     seq: 1,
///     nondeterminism: NdClass::Pure,
///     result_ref: ContentRef::from_bytes([7u8; 32]),
///     parents: SmallVec::new(),
///     warrant_ref: ContentRef::from_bytes([0xaa; 32]),
///     mote_def_hash: MoteDefHash::from_bytes([1u8; 32]),
/// }).unwrap();
///
/// let snap = p.snapshot();
/// assert_eq!(snap.state_of(&MoteId::from_bytes([1u8; 32])), MoteState::Committed);
/// assert_eq!(snap.seq(), 1);
///
/// // Mutate the projection; snapshot stays at seq 1.
/// p.fold(&JournalEntry::Repudiated {
///     target_mote_id: MoteId::from_bytes([1u8; 32]),
///     idempotency_key: [9u8; 32],
///     seq: 2,
///     target_committed_seq: 1,
///     reason_class: RepudiationReason::OperatorAction,
///     repudiator_id: 0,
/// }).unwrap();
///
/// assert_eq!(snap.state_of(&MoteId::from_bytes([1u8; 32])), MoteState::Committed);
/// assert_eq!(p.state_of(&MoteId::from_bytes([1u8; 32])), MoteState::Repudiated);
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub(crate) state: State,
}

impl Snapshot {
    /// The `seq` at which this snapshot was captured.
    #[inline]
    #[must_use]
    pub fn seq(&self) -> u64 {
        self.state.last_seq
    }

    /// Number of Motes in this snapshot.
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.state.motes.len()
    }

    /// `true` when the snapshot is empty.
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.state.motes.is_empty()
    }

    /// Per-identity state. See [`crate::Projection::state_of`].
    #[must_use]
    pub fn state_of(&self, mote_id: &MoteId) -> MoteState {
        self.state.state_of_id(mote_id)
    }

    /// The registered run identity (D64). See
    /// [`crate::Projection::run_registration`].
    #[must_use]
    pub fn run_registration(&self) -> Option<([u8; kx_journal::INSTANCE_ID_LEN], [u8; 32])> {
        self.state
            .run_registration
            .map(|r| (r.instance_id, r.recipe_fingerprint))
    }

    /// Direct parents. See [`crate::Projection::parents_of`].
    #[must_use]
    pub fn parents_of(&self, mote_id: &MoteId) -> SmallVec<[(MoteId, EdgeMeta); 4]> {
        self.state.parents_of_id(mote_id)
    }

    /// Direct children. See [`crate::Projection::children_of`].
    #[must_use]
    pub fn children_of(&self, mote_id: &MoteId) -> Vec<(MoteId, EdgeMeta)> {
        self.state
            .children
            .get(mote_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Transitive consumers via data + non-opted-out control edges.
    #[must_use]
    pub fn transitive_consumers(&self, mote_id: &MoteId) -> Vec<MoteId> {
        transitive_consumers_impl(&self.state, mote_id)
    }

    /// Committed result_ref. See [`crate::Projection::result_ref_of`].
    #[must_use]
    pub fn result_ref_of(&self, mote_id: &MoteId) -> Option<ContentRef> {
        self.state
            .motes
            .get(mote_id)
            .and_then(|i| i.committed.as_ref().map(|c| c.result_ref))
    }

    /// Committed non-determinism tag. See
    /// [`crate::Projection::nondeterminism_of`].
    #[must_use]
    pub fn nondeterminism_of(&self, mote_id: &MoteId) -> Option<NdClass> {
        self.state
            .motes
            .get(mote_id)
            .and_then(|i| i.committed.as_ref().map(|c| c.nondeterminism))
    }

    /// Committed-entry `seq`. See [`crate::Projection::committed_seq_of`].
    #[must_use]
    pub fn committed_seq_of(&self, mote_id: &MoteId) -> Option<u64> {
        self.state
            .motes
            .get(mote_id)
            .and_then(|i| i.committed.as_ref().map(|c| c.seq))
    }

    /// Declared-or-committed warrant_ref for the Mote. See
    /// [`crate::Projection::warrant_ref_of`].
    #[must_use]
    pub fn warrant_ref_of(&self, mote_id: &MoteId) -> Option<ContentRef> {
        self.state.motes.get(mote_id).and_then(|i| {
            i.declared
                .as_ref()
                .map(|d| d.warrant_ref)
                .or_else(|| i.committed.as_ref().map(|c| c.warrant_ref))
        })
    }

    /// The ready set at the snapshot's `seq`.
    #[must_use]
    pub fn ready_set(&self) -> Vec<MoteId> {
        ready_set_impl(&self.state)
    }

    /// Promotion state (P1 default: `NotApplicable`).
    #[must_use]
    pub fn promotion_state(&self, mote_id: &MoteId) -> PromotionState {
        promotion_state_impl(&self.state, mote_id)
    }

    /// `true` when the Mote's committed entry has been Repudiated by `seq`.
    #[must_use]
    pub fn is_repudiated(&self, mote_id: &MoteId) -> bool {
        matches!(self.state_of(mote_id), MoteState::Repudiated)
    }

    /// **v2 (PR 7, STEP 5.3 + R-13).** Snapshot mirror of
    /// [`crate::Projection::can_redispatch_world_effect`].
    #[must_use]
    pub fn can_redispatch_world_effect(&self, mote_id: &MoteId) -> bool {
        self.state.can_redispatch_world_effect_id(mote_id)
    }

    /// **v2 (PR 7, STEP 5.3).** Snapshot mirror of [`crate::Projection::anomaly_motes`].
    #[must_use]
    pub fn anomaly_motes(&self) -> Vec<(MoteId, AnomalyKind)> {
        self.state.anomaly_motes_iter()
    }

    /// Iterate every Mote known at snapshot time with its state.
    pub fn iter_motes(&self) -> impl Iterator<Item = (MoteId, MoteState)> + '_ {
        self.state
            .motes
            .keys()
            .map(move |id| (*id, self.state.state_of_id(id)))
    }

    /// Iterate every Mote currently in `state` at snapshot time.
    pub fn iter_motes_in_state(&self, state: MoteState) -> impl Iterator<Item = MoteId> + '_ {
        self.state
            .motes
            .keys()
            .filter(move |id| self.state.state_of_id(id) == state)
            .copied()
    }

    /// Count of Motes in `MoteState::Committed` at snapshot time.
    #[must_use]
    pub fn committed_count(&self) -> usize {
        self.iter_motes_in_state(MoteState::Committed).count()
    }

    /// Count of Motes in `MoteState::Repudiated` at snapshot time.
    #[must_use]
    pub fn repudiated_count(&self) -> usize {
        self.iter_motes_in_state(MoteState::Repudiated).count()
    }

    /// Count of Motes in `MoteState::Pending` at snapshot time.
    #[must_use]
    pub fn pending_count(&self) -> usize {
        self.iter_motes_in_state(MoteState::Pending).count()
    }

    /// Count of Motes in `MoteState::Failed` at snapshot time.
    #[must_use]
    pub fn failed_count(&self) -> usize {
        self.iter_motes_in_state(MoteState::Failed).count()
    }

    /// Count of Motes in `MoteState::Scheduled` at snapshot time.
    #[must_use]
    pub fn scheduled_count(&self) -> usize {
        self.iter_motes_in_state(MoteState::Scheduled).count()
    }
}

// ---------------------------------------------------------------------------
// Shared read-side implementations (used by both Projection and Snapshot)
// ---------------------------------------------------------------------------
