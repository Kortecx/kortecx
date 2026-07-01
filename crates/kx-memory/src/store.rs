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
use crate::record::{MemoryHit, MemoryRecord, StoreOutcome, StoreRequest};

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
    /// optionally scoped to the run `instance_filter`.
    ///
    /// # Errors
    /// [`MemoryError::Internal`] on a backend failure.
    fn list(
        &self,
        namespace: &str,
        instance_filter: Option<[u8; 16]>,
        limit: usize,
    ) -> Result<Vec<MemoryRecord>, MemoryError>;

    /// Erase a memory from `namespace`. Returns `true` if a row was removed.
    ///
    /// # Errors
    /// [`MemoryError::Internal`] on a backend failure.
    fn forget(&self, namespace: &str, memory_id: &ContentRef) -> Result<bool, MemoryError>;
}
