#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::doc_markdown,
    clippy::return_self_not_must_use
)]
// Inline test modules are exempted from the workspace deny on `unwrap_used` /
// `expect_used`. Integration tests under tests/*.rs carry per-file allows.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! # kx-memoizer — pure cache-hit lookup over projection state
//!
//! [`lookup`] answers one question as a **pure, total, deterministic** function of a
//! candidate [`Mote`] and a read-only [`Snapshot`]: *"is this exact `MoteId` already
//! committed and safe to serve as a cache hit?"* Matching is **EXACT cryptographic
//! equality only**, never similarity (SN-8) — two Motes match iff their derived
//! `MoteId`s are bit-identical. [`kx_mote::derive_mote_id`] is the single identity
//! truth; canonicalize fuzzy-similar inputs with the normalizer (P1.7.10) *before*
//! fingerprinting.
//!
//! It returns a [`CacheHit`] (variant mirroring the candidate's [`NdClass`]) iff the
//! projection reports the Mote `Committed`, no **Data-edge** parent is `Repudiated`
//! (a repudiated parent poisons the lineage; Control edges sequence-only, never
//! taint), and a `result_ref` is present. Read-side only: it never dispatches,
//! re-dispatches, runs inference, or mutates — the executor (P1.9) acts on the hit,
//! and the journal-write side goes through the executor's commit protocol.
//!
//! **Effect-once ≠ inference-once.** The memoizer is *inference*-once (it caches the
//! model's decision); the broker (P1.8.5) is *effect*-once (it dedups side effects).
//! So [`CacheHit::WorldMutating`] carries `redispatch_effect: true`: the cache
//! supplies the decision, but the effect still goes through the broker per attempt.
//! Rationale: design corpus `validate-then-commit.md` §10.5.

use kx_content::ContentRef;
use kx_mote::{EdgeKind, Mote, NdClass};
use kx_projection::{MoteState, Snapshot};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// CacheHit
// ---------------------------------------------------------------------------

/// A cache hit returned by [`lookup`].
///
/// The variant corresponds to the candidate [`Mote`]'s [`NdClass`]:
///
/// - [`CacheHit::Pure`] — deterministic result; safe to serve directly.
/// - [`CacheHit::ReadOnlyNondet`] — durable observation already committed;
///   safe to serve directly. The non-determinism was resolved on the prior
///   attempt and the observation is now a fact.
/// - [`CacheHit::WorldMutating`] — the model's decision is reusable, but
///   the executor MUST re-dispatch the broker effect against `result_ref`.
///   The `redispatch_effect: true` field exists to make this constraint
///   loud at every match site.
///
/// All variants carry the cached `result_ref`. Use [`CacheHit::result_ref`]
/// to read it variant-agnostically.
///
/// # Examples
///
/// ```
/// use kx_content::ContentRef;
/// use kx_memoizer::CacheHit;
///
/// let r = ContentRef::of(b"cached payload");
/// let hit = CacheHit::Pure { result_ref: r };
/// assert_eq!(hit.result_ref(), &r);
///
/// // Variants are distinct under PartialEq.
/// assert_ne!(
///     CacheHit::Pure { result_ref: r },
///     CacheHit::ReadOnlyNondet { result_ref: r },
/// );
///
/// // WorldMutating signals re-dispatch is required.
/// let wm = CacheHit::WorldMutating { result_ref: r, redispatch_effect: true };
/// assert!(matches!(
///     wm,
///     CacheHit::WorldMutating { redispatch_effect: true, .. },
/// ));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CacheHit {
    /// PURE Mote cache hit. Result is deterministic; downstream consumers
    /// read `result_ref` directly. No effect to re-dispatch.
    Pure {
        /// The cached payload's content ref.
        result_ref: ContentRef,
    },

    /// READ-ONLY-NONDET cache hit. The cached observation was committed on
    /// a prior attempt; the downstream reads `result_ref` to obtain the
    /// durable fact. No effect to re-dispatch (the observation IS the fact).
    ReadOnlyNondet {
        /// The cached observation's content ref.
        result_ref: ContentRef,
    },

    /// WORLD-MUTATING cache hit. The cached decision is reusable, but the
    /// broker effect MUST be re-dispatched per attempt against `result_ref`.
    /// `redispatch_effect` is always `true` for this variant; it exists to
    /// make the constraint loud in match arms.
    WorldMutating {
        /// The cached decision's content ref. The broker re-dispatches the
        /// effect against this ref per attempt.
        result_ref: ContentRef,
        /// Always `true` for this variant. Loud-by-design — the constraint
        /// is named at every match site rather than buried in prose.
        redispatch_effect: bool,
    },
}

impl CacheHit {
    /// Borrow the cached `result_ref` variant-agnostically.
    ///
    /// # Examples
    ///
    /// ```
    /// use kx_content::ContentRef;
    /// use kx_memoizer::CacheHit;
    ///
    /// let r = ContentRef::of(b"x");
    /// let hits = [
    ///     CacheHit::Pure { result_ref: r },
    ///     CacheHit::ReadOnlyNondet { result_ref: r },
    ///     CacheHit::WorldMutating { result_ref: r, redispatch_effect: true },
    /// ];
    /// for h in &hits {
    ///     assert_eq!(h.result_ref(), &r);
    /// }
    /// ```
    #[must_use]
    pub const fn result_ref(&self) -> &ContentRef {
        match self {
            Self::Pure { result_ref }
            | Self::ReadOnlyNondet { result_ref }
            | Self::WorldMutating { result_ref, .. } => result_ref,
        }
    }

    /// `true` iff the caller must re-dispatch a broker effect against
    /// `result_ref`. Always `true` for [`CacheHit::WorldMutating`], always
    /// `false` for the other variants.
    ///
    /// # Examples
    ///
    /// ```
    /// use kx_content::ContentRef;
    /// use kx_memoizer::CacheHit;
    ///
    /// let r = ContentRef::of(b"x");
    /// assert!(!CacheHit::Pure { result_ref: r }.requires_redispatch());
    /// assert!(!CacheHit::ReadOnlyNondet { result_ref: r }.requires_redispatch());
    /// assert!(
    ///     CacheHit::WorldMutating { result_ref: r, redispatch_effect: true }
    ///         .requires_redispatch()
    /// );
    /// ```
    #[must_use]
    pub const fn requires_redispatch(&self) -> bool {
        matches!(
            self,
            Self::WorldMutating {
                redispatch_effect: true,
                ..
            }
        )
    }
}

// ---------------------------------------------------------------------------
// lookup
// ---------------------------------------------------------------------------

/// Look up a cache hit for `mote` against `snapshot`.
///
/// Returns `Some(CacheHit)` iff ALL of:
///
/// 1. The projection reports `snapshot.state_of(&mote.id) == MoteState::Committed`.
/// 2. No `Data`-edge parent of `mote` is `MoteState::Repudiated`. Control-edge
///    parents are sync-only and do NOT taint cache eligibility.
/// 3. `snapshot.result_ref_of(&mote.id)` returns `Some(ref)`.
///
/// Returns `None` otherwise — the caller must dispatch the Mote.
///
/// The returned [`CacheHit`] variant mirrors the candidate Mote's declared
/// [`NdClass`]. By BLAKE3 collision resistance over [`kx_mote::derive_mote_id`]'s
/// inputs (which include `mote_def_hash`, and `mote_def_hash` includes
/// `nd_class`), a `MoteId` match implies the prior commit's `nd_class`
/// matches the candidate's — the memoizer can trust the candidate's
/// declared class without re-querying the committed entry.
///
/// **No similarity operator.** This is EXACT cryptographic equality only.
///
/// # Examples
///
/// Cache miss when no Committed entry exists for the Mote:
///
/// ```
/// use kx_journal::InMemoryJournal;
/// use kx_memoizer::lookup;
/// use kx_projection::Projection;
/// // Build an empty projection (no commits).
/// let journal = InMemoryJournal::new();
/// let snapshot = Projection::from_journal(&journal).unwrap().snapshot();
/// // A Mote that isn't in the projection cannot have a cache hit.
/// // (See kx-memoizer/tests/proptest_memoizer.rs for the full fixture.)
/// // ...
/// ```
#[tracing::instrument(level = "debug", skip_all, fields(mote_id = ?mote.id))]
#[must_use]
pub fn lookup(mote: &Mote, snapshot: &Snapshot) -> Option<CacheHit> {
    // 1. Snapshot must report the Mote as Committed.
    if !matches!(snapshot.state_of(&mote.id), MoteState::Committed) {
        return None;
    }

    // 2. Data-edge parents that are Repudiated poison the cache hit.
    //    Control edges do NOT taint (they're synchronization-only).
    for parent in &mote.parents {
        if parent.edge.kind != EdgeKind::Data {
            continue;
        }
        if matches!(snapshot.state_of(&parent.parent_id), MoteState::Repudiated) {
            return None;
        }
    }

    // 3. The cached result_ref must be present (Committed without a
    //    result_ref would be a projection-side anomaly; absence here is a
    //    cache miss, not an error).
    let result_ref = snapshot.result_ref_of(&mote.id)?;

    // 4. Construct the CacheHit variant per the candidate's NdClass.
    //    BLAKE3 collision resistance over derive_mote_id's inputs means
    //    the candidate's nd_class matches the committed entry's
    //    nondeterminism when their MoteIds match.
    Some(match mote.def.nd_class {
        NdClass::Pure => CacheHit::Pure { result_ref },
        NdClass::ReadOnlyNondet => CacheHit::ReadOnlyNondet { result_ref },
        NdClass::WorldMutating => CacheHit::WorldMutating {
            result_ref,
            redispatch_effect: true,
        },
    })
}

// ---------------------------------------------------------------------------
// Inline tests — fixture-heavy unit tests live in tests/proptest_memoizer.rs
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_hit_result_ref_accessor_is_variant_agnostic() {
        let r = ContentRef::of(b"x");
        assert_eq!(CacheHit::Pure { result_ref: r }.result_ref(), &r);
        assert_eq!(CacheHit::ReadOnlyNondet { result_ref: r }.result_ref(), &r);
        assert_eq!(
            CacheHit::WorldMutating {
                result_ref: r,
                redispatch_effect: true
            }
            .result_ref(),
            &r
        );
    }

    #[test]
    fn variants_are_distinct_under_partial_eq() {
        let r = ContentRef::of(b"x");
        assert_ne!(
            CacheHit::Pure { result_ref: r },
            CacheHit::ReadOnlyNondet { result_ref: r },
        );
        assert_ne!(
            CacheHit::Pure { result_ref: r },
            CacheHit::WorldMutating {
                result_ref: r,
                redispatch_effect: true
            },
        );
        assert_ne!(
            CacheHit::ReadOnlyNondet { result_ref: r },
            CacheHit::WorldMutating {
                result_ref: r,
                redispatch_effect: true
            },
        );
    }

    #[test]
    fn requires_redispatch_only_true_for_world_mutating() {
        let r = ContentRef::of(b"x");
        assert!(!CacheHit::Pure { result_ref: r }.requires_redispatch());
        assert!(!CacheHit::ReadOnlyNondet { result_ref: r }.requires_redispatch());
        assert!(CacheHit::WorldMutating {
            result_ref: r,
            redispatch_effect: true
        }
        .requires_redispatch());
    }
}
