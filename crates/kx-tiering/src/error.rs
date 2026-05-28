//! The tiering-pass error type.

use kx_content::StoreError;

/// A failure during a tiering pass.
///
/// Note that a [`NotFound`](kx_content::NotFound) while *sizing* a candidate is
/// **not** an error — an already-absent PURE payload (evicted by a prior pass or
/// the orphan-GC walker) is simply skipped. Only a genuine store/IO failure on
/// [`delete`](kx_content::ContentStore::delete) surfaces here.
#[derive(Debug, thiserror::Error)]
pub enum TieringError {
    /// A `ContentStore::delete` call failed.
    #[error("content store delete failed: {0}")]
    Delete(#[from] StoreError),
}
