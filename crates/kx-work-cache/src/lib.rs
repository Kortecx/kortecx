#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::doc_markdown
)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! # kx-work-cache — the cross-run, content-addressed work cache
//!
//! The in-run memoizer (`kx_memoizer::lookup`) serves a committed result by
//! **exact `MoteId` equality**, where `MoteId = blake3(mote_def_hash ‖ input_data_id
//! ‖ graph_position)`. Because `graph_position` folds a **run-scoped** salt (react
//! turns salt by `instance_id`, entrypoints by a per-run seed), identical work in a
//! *different* run derives a *different* `MoteId` and is recomputed.
//!
//! This crate closes that gap for **PURE** work with a second lookup keyed on a
//! **run-independent** [`WorkFingerprint`]:
//!
//! ```text
//! work_fingerprint = blake3( DOMAIN_SEP(nd_class) ‖ mote_def_hash[32] ‖ input_data_id[32] )
//! ```
//!
//! `graph_position` is deliberately **excluded**. A [`NdClass::Pure`] Mote is a
//! bit-stable function of `(logic = mote_def_hash, inputs = input_data_id)` ONLY, so
//! two nodes with equal `(def, iid)` compute identical bytes regardless of position;
//! dropping the positional salt is exactly what makes the key run-independent, and it
//! cannot change a pure output. `(mote_def_hash, input_data_id)` is a strict subset of
//! `MoteId`'s inputs, so a cross-run hit can only ever conflate motes the in-run
//! identity would also conflate if they shared a `graph_position`.
//!
//! ## Boundaries (load-bearing)
//!
//! - **Off the truth path.** The cache is a rebuildable, non-authoritative
//!   projection — a lost/corrupt cache costs only recomputation, never correctness.
//!   Matching is EXACT cryptographic equality, never similarity. A cross-run hit does
//!   NOT skip the journal write: this run still commits its own `MoteId → result_ref`
//!   fact (audit + `ProjectionDigest` preserved); only the *body compute* is skipped.
//! - **PURE only.** The class is baked into `DOMAIN_SEP`, so a `pure` lookup can never
//!   find a `rond`/`wm` entry even for identical `(def, iid)`. `WorldMutating` results
//!   must NEVER be served from a content cache (a real effect would be skipped) — the
//!   read hook lives only in the worker's PURE dispatch path (WorldMutating work goes
//!   through a different arm), and [`work_fingerprint`] additionally gives
//!   `WorldMutating` a distinct, never-looked-up domain separator as a backstop.
//! - **Infallible from the caller's view.** [`WorkCache::insert`] / [`WorkCache::evict`]
//!   never surface errors to the run: a cache-write failure is logged and swallowed, so
//!   the cache can never break a run it is only meant to accelerate.

use kx_content::ContentRef;
use kx_mote::{InputDataId, MoteDefHash, MoteId, NdClass};
use serde::{Deserialize, Serialize};

mod in_memory;
mod sqlite;

pub use in_memory::InMemoryWorkCache;
pub use sqlite::{SqliteWorkCache, WorkCacheError};

// ---------------------------------------------------------------------------
// WorkFingerprint
// ---------------------------------------------------------------------------

/// The 32-byte, **run-independent** key of a unit of work.
///
/// `blake3(DOMAIN_SEP(nd_class) ‖ mote_def_hash ‖ input_data_id)`. Unlike `MoteId`
/// it excludes `graph_position`, so byte-identical work in two different runs shares
/// one fingerprint. Construct via [`work_fingerprint`].
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct WorkFingerprint(pub [u8; 32]);

impl WorkFingerprint {
    /// Construct a `WorkFingerprint` from raw 32 bytes.
    #[inline]
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Borrow the underlying 32 bytes.
    #[inline]
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl std::fmt::Debug for WorkFingerprint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "WorkFingerprint({})",
            blake3::Hash::from_bytes(self.0).to_hex()
        )
    }
}

impl std::fmt::Display for WorkFingerprint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", blake3::Hash::from_bytes(self.0).to_hex())
    }
}

/// The per-class domain separator. The `NdClass` is folded into the key so a `pure`
/// lookup can never collide with a `rond` (or the forbidden `wm`) entry, even for
/// identical `(mote_def_hash, input_data_id)`. This is the third structural barrier
/// against ever serving a WorldMutating result from the content cache.
const fn domain_sep(nd: NdClass) -> &'static [u8] {
    match nd {
        NdClass::Pure => b"kx-work-cache/v1/pure",
        NdClass::ReadOnlyNondet => b"kx-work-cache/v1/rond",
        // Never inserted and never looked up (the executor read hook is PURE-only).
        // A distinct separator guarantees a WM entry could not be *found* by a
        // pure/rond lookup even if some future caller mis-wired an insert.
        NdClass::WorldMutating => b"kx-work-cache/v1/wm-FORBIDDEN",
    }
}

/// Compute the run-independent [`WorkFingerprint`] for a unit of work.
///
/// Both `mote_def_hash` and `input_data_id` are fixed 32-byte values, so the
/// concatenation boundary is unambiguous (the same discipline as
/// [`kx_mote::derive_mote_id`]). The class-specific `domain_sep` prefix keeps the
/// PURE and ROND keyspaces disjoint.
#[must_use]
pub fn work_fingerprint(
    nd: NdClass,
    mote_def_hash: &MoteDefHash,
    input_data_id: &InputDataId,
) -> WorkFingerprint {
    let mut hasher = blake3::Hasher::new();
    hasher.update(domain_sep(nd));
    hasher.update(mote_def_hash.as_bytes());
    hasher.update(input_data_id.as_bytes());
    WorkFingerprint::from_bytes(*hasher.finalize().as_bytes())
}

// ---------------------------------------------------------------------------
// WorkCache
// ---------------------------------------------------------------------------

/// A cross-run cache mapping a [`WorkFingerprint`] to the `ContentRef` a prior run
/// already computed for that exact work.
///
/// The executor consults this ONLY inside the PURE lifecycle path, so a
/// `WorldMutating` result can never be served from it. All methods take `&self`
/// (interior mutability in the backends) so a single `Arc<dyn WorkCache>` is shared
/// across a serve's workers.
pub trait WorkCache: Send + Sync {
    /// Return the cached `ContentRef` for `fp`, or `None` on a miss.
    ///
    /// EXACT-equality lookup, never similarity. A pure read: it never mutates.
    /// The caller MUST still verify the ref's bytes are present in the content store
    /// (GC guard) before serving it, and MUST still commit this run's own fact.
    fn lookup(&self, fp: &WorkFingerprint) -> Option<ContentRef>;

    /// Record that `result_ref` is the result of the work identified by `fp`.
    ///
    /// **First-writer-wins**: a second insert for an existing `fp` is a no-op (the
    /// stored ref is deterministic anyway, since the work is PURE). Infallible from
    /// the caller's view — any backend error is logged and swallowed, never
    /// propagated, so the cache cannot break a run.
    fn insert(&self, fp: WorkFingerprint, result_ref: ContentRef, nd: NdClass, source: MoteId);

    /// Remove the entry for `fp`, if any.
    ///
    /// Called when a PURE committed mote is repudiated (a "PURE" that turned out not
    /// to be bit-stable), so a stale ref is not served to a later run. Infallible from
    /// the caller's view (errors logged and swallowed).
    fn evict(&self, fp: &WorkFingerprint);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def_hash(b: u8) -> MoteDefHash {
        MoteDefHash::from_bytes([b; 32])
    }
    fn iid(b: u8) -> InputDataId {
        InputDataId::from_bytes([b; 32])
    }

    #[test]
    fn fingerprint_is_deterministic() {
        let a = work_fingerprint(NdClass::Pure, &def_hash(1), &iid(2));
        let b = work_fingerprint(NdClass::Pure, &def_hash(1), &iid(2));
        assert_eq!(a, b);
    }

    #[test]
    fn distinct_inputs_distinct_fingerprint() {
        let base = work_fingerprint(NdClass::Pure, &def_hash(1), &iid(2));
        assert_ne!(base, work_fingerprint(NdClass::Pure, &def_hash(9), &iid(2)));
        assert_ne!(base, work_fingerprint(NdClass::Pure, &def_hash(1), &iid(9)));
    }

    #[test]
    fn class_domain_separation() {
        // Identical (def, iid) across classes must NOT collide — WM/ROND can never be
        // found by a PURE lookup.
        let pure = work_fingerprint(NdClass::Pure, &def_hash(1), &iid(2));
        let rond = work_fingerprint(NdClass::ReadOnlyNondet, &def_hash(1), &iid(2));
        let wm = work_fingerprint(NdClass::WorldMutating, &def_hash(1), &iid(2));
        assert_ne!(pure, rond);
        assert_ne!(pure, wm);
        assert_ne!(rond, wm);
    }
}
