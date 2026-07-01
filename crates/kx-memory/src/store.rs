// SPDX-License-Identifier: Apache-2.0
//! [`MemoryStore`] — the durable, per-namespace memory seam.
//!
//! Methods take `&self` (interior mutability) so a store can be shared
//! (`Arc<dyn MemoryStore>`) by a `recall@1` / `remember@1` capability without
//! granting it write authority over the journal. The store is
//! journal-authoritative: a reconstructible projection/cache of committed content,
//! never a second source of truth.

use kx_content::ContentRef;

use crate::error::MemoryError;
use crate::record::{
    BundleRequest, DecayPolicy, DecayReport, MemoryHit, MemoryRecord, MemoryStats, StoreOutcome,
    StoreRequest,
};

/// A durable, per-namespace store of what an agent learned. The default
/// implementation is [`crate::SqliteMemoryStore`] (a `memory.db` + a rebuildable
/// per-namespace similarity index).
pub trait MemoryStore: Send + Sync {
    /// Remember `req.content` (embedded as `req.vector`) in `req.namespace`.
    /// Content-addressed + idempotent: the same payload dedups to one row, so a
    /// pre-commit re-dispatch is a durable no-op.
    ///
    /// # Errors
    /// [`MemoryError::InvalidArgument`] for empty/oversize content, a bad namespace,
    /// or a non-finite vector; [`MemoryError::DimMismatch`] / [`MemoryError::StaleIndex`]
    /// if the vector is incompatible with the namespace's existing index;
    /// [`MemoryError::Internal`] on a backend failure.
    fn store(&self, req: StoreRequest<'_>) -> Result<StoreOutcome, MemoryError>;

    /// Recall the `k` memories in `namespace` most similar to `query_vec`, highest
    /// score first. An unknown/empty namespace yields an empty result (never an
    /// error). `embed_fingerprint` (non-empty) guards against querying with a vector
    /// from a different embed model than the namespace was indexed under.
    ///
    /// # Errors
    /// [`MemoryError::InvalidArgument`] for a non-finite query vector;
    /// [`MemoryError::DimMismatch`] / [`MemoryError::StaleIndex`] on an incompatible
    /// query; [`MemoryError::Internal`] on a backend failure.
    fn recall(
        &self,
        namespace: &str,
        query_vec: &[f32],
        k: usize,
        embed_fingerprint: &str,
    ) -> Result<Vec<MemoryHit>, MemoryError>;

    /// The episodic log of `namespace`, newest-first, at most `limit` rows,
    /// optionally scoped to the run `instance_filter`. `include_tombstoned` = `false`
    /// hides decayed memories (the default view); `true` surfaces them (the decayed
    /// view) with `tombstoned_ms` set so a UI can render + restore them.
    ///
    /// # Errors
    /// [`MemoryError::Internal`] on a backend failure.
    fn list(
        &self,
        namespace: &str,
        instance_filter: Option<[u8; 16]>,
        limit: usize,
        include_tombstoned: bool,
    ) -> Result<Vec<MemoryRecord>, MemoryError>;

    /// Gather a set of (live, non-tombstoned) memories for the model to consolidate —
    /// either the newest-first (recency) or the most-similar-to-a-query (semantic)
    /// slice of `req.namespace`, optionally restricted by kind + `created_ms` window.
    /// The similarity score, if any, stays INSIDE this call (the returned
    /// [`MemoryRecord`] carries no score — SN-8 by return type).
    ///
    /// # Errors
    /// [`MemoryError::InvalidArgument`] for a bad namespace / non-finite query vector;
    /// [`MemoryError::DimMismatch`] / [`MemoryError::StaleIndex`] on an incompatible
    /// query; [`MemoryError::Internal`] on a backend failure.
    fn bundle(&self, req: BundleRequest<'_>) -> Result<Vec<MemoryRecord>, MemoryError>;

    /// Preview or apply a TTL + salience decay sweep over `namespace`. A candidate is
    /// soft-tombstoned (the `memories` row is never deleted — reversible via
    /// [`MemoryStore::restore`]); `policy.dry_run` previews without evicting.
    ///
    /// # Errors
    /// [`MemoryError::InvalidArgument`] for a bad namespace;
    /// [`MemoryError::Internal`] on a backend failure.
    fn decay(&self, namespace: &str, policy: DecayPolicy) -> Result<DecayReport, MemoryError>;

    /// Apply a decay sweep across EVERY namespace (the operator auto-sweep on open).
    /// Returns the total number of memories tombstoned.
    ///
    /// # Errors
    /// [`MemoryError::Internal`] on a backend failure.
    fn decay_all(&self, policy: DecayPolicy) -> Result<usize, MemoryError>;

    /// Namespace statistics — live counts by kind, tombstoned count, dim, fingerprint,
    /// and the live age range. Read-only.
    ///
    /// # Errors
    /// [`MemoryError::Internal`] on a backend failure.
    fn stats(&self, namespace: &str) -> Result<MemoryStats, MemoryError>;

    /// Un-decay (restore) a soft-tombstoned memory in `namespace`, re-surfacing it to
    /// recall/list. Returns `true` if a tombstone was cleared.
    ///
    /// # Errors
    /// [`MemoryError::Internal`] on a backend failure.
    fn restore(&self, namespace: &str, memory_id: &ContentRef) -> Result<bool, MemoryError>;

    /// Erase a memory from `namespace` (a HARD delete — not reversible, unlike decay).
    /// Returns `true` if a row was removed.
    ///
    /// # Errors
    /// [`MemoryError::Internal`] on a backend failure.
    fn forget(&self, namespace: &str, memory_id: &ContentRef) -> Result<bool, MemoryError>;
}
