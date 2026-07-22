// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! [`MemoryError`] — the memory-store error vocabulary. Deliberately shaped 1:1
//! with `kx_gateway_core::DatasetError` so the `recall@1` / `remember@1`
//! capabilities map failures to honest gRPC codes / soft-fail observations exactly
//! as `retrieve@1` does over datasets.

use thiserror::Error;

/// A failure from a [`crate::MemoryStore`] operation.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MemoryError {
    /// The namespace does not exist (has no memories). A recall against it yields an
    /// empty result rather than this error; this is reserved for operations that
    /// require an existing namespace.
    #[error("memory namespace not found")]
    NotFound,

    /// A vector's length disagrees with the namespace's fixed embedding dimension.
    #[error("vector dimension mismatch: {0}")]
    DimMismatch(String),

    /// The vector was produced under a different embed-model fingerprint than the
    /// namespace was indexed under — querying/storing would mix incompatible vector
    /// spaces, so the store refuses rather than silently mis-rank. Re-index to rebuild.
    #[error("stale index (embed-model fingerprint mismatch): {0}")]
    StaleIndex(String),

    /// A malformed request (empty/oversize content, a bad namespace, a non-finite
    /// vector).
    #[error("invalid argument: {0}")]
    InvalidArgument(String),

    /// A backend failure (the durable store / a poisoned lock).
    #[error("memory store backend failure: {0}")]
    Internal(String),
}
