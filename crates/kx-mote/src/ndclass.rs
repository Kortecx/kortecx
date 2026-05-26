//! [`crate::NdClass`] — the three-way non-determinism tag (PURE / READ-ONLY-NONDET /
//! WORLD-MUTATING). Drives the runtime's recovery semantics per D5.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// NdClass — the non-determinism tag (one knob, three jobs)
// ---------------------------------------------------------------------------

/// The non-determinism tag attached to every Mote.
///
/// One knob, three jobs (recovery, storage tiering, scheduling priority).
/// See the private design corpus (`mote.md` §6 + D2) for the per-tag rules.
/// Stable u8 representations are used in journal entry headers (PURE=0,
/// READ-ONLY-NONDET=1, WORLD-MUTATING=2) — these MUST NOT change without
/// a journal `schema_version` bump.
///
/// # Examples
///
/// ```
/// use kx_mote::NdClass;
///
/// // Stable u8 discriminants for journal-entry headers.
/// assert_eq!(NdClass::Pure.as_u8(), 0);
/// assert_eq!(NdClass::ReadOnlyNondet.as_u8(), 1);
/// assert_eq!(NdClass::WorldMutating.as_u8(), 2);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum NdClass {
    /// Output is a mathematically-deterministic AND bit-stable function of inputs.
    /// No side effects, no model sampling, no external calls. Safe to re-run.
    /// Storage: droppable + recomputable under memory pressure.
    Pure = 0,

    /// Samples a non-deterministic source (model inference, RNG) but causes
    /// **no external state change**. NEVER re-run once Committed; recovery
    /// reads the committed result. Storage: always persisted.
    ReadOnlyNondet = 1,

    /// Causes external side effects (API call, write, message send) the runtime
    /// cannot reverse or recompute. NEVER re-run once Committed; pre-commit
    /// re-runs are safe only via [`crate::EffectPattern::IdempotentByConstruction`] or
    /// [`crate::EffectPattern::ValidateThenCommit`]. Storage: always persisted.
    /// Speculation forbidden by the executor.
    WorldMutating = 2,
}

impl NdClass {
    /// Convert to the canonical u8 representation for journal entry headers.
    #[inline]
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        self as u8
    }
}
