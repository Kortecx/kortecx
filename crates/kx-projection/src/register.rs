//! [`RegisterMote`] — workflow-author declaration of a Mote before any
//! journal entry exists for it.

use kx_content::ContentRef;
use kx_mote::{EffectPattern, MoteId, NdClass, ParentRef};
use smallvec::SmallVec;

/// Workflow-author declaration of a Mote before any journal entry exists for it.
///
/// Submitted at workflow-compile time by the executor (P1.9) or directly by tests.
/// Adds the Mote to the projection's "expected" set so it appears as
/// [`crate::MoteState::Pending`] in `state_of` and is eligible for `ready_set` once its
/// parents commit.
#[derive(Debug, Clone)]
pub struct RegisterMote {
    /// The Mote's identity.
    pub mote_id: MoteId,
    /// The Mote's non-determinism tag (drives recovery semantics, scheduling priority).
    pub nd_class: NdClass,
    /// Which effect pattern this Mote uses (D20). Determines whether `ready_set`
    /// gates downstream consumers on a critic verdict (3c only).
    pub effect_pattern: EffectPattern,
    /// If this Mote is a critic of another Mote, the producer's identity.
    pub critic_for: Option<MoteId>,
    /// If this Mote is a topology shaper, set to `true` (P1.11 will use this).
    pub is_topology_shaper: bool,
    /// The Mote's declared parents with edge metadata. Up to 4 inline; spill heap.
    pub parents: SmallVec<[ParentRef; 4]>,
    /// **PR 11.5 / KG-1-close.** The warrant the Mote will execute under,
    /// content-addressed via [`kx_warrant::warrant_ref_of`]. Workflow-author
    /// submissions populate this with `warrant_ref_of(&submitted_warrant)`.
    /// Shaper-materialized children (D48 + D49) populate this with
    /// `warrant_ref_of(&intersect(shaper.warrant, role.spec))` — the
    /// closing of `topology.md` §13 KG-1's verbatim-inheritance gap.
    pub warrant_ref: ContentRef,
}
