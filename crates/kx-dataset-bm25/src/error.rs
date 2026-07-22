// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! Errors for the BM25 sparse retrieval backend.

/// An error opening, persisting, or decoding the BM25 cache.
///
/// The cache is a rebuildable projection (D40), so callers recover from
/// `Corrupt` / `Io` by rebuilding the index from the journal/rows — never by
/// trusting a partially-decoded file.
#[derive(Debug, thiserror::Error)]
pub enum Bm25Error {
    /// An I/O error reading or writing the cache file.
    #[error("bm25 cache i/o: {0}")]
    Io(#[from] std::io::Error),
    /// The cache file is malformed (bad header, truncation, bad UTF-8, or trailing
    /// bytes); rebuild the index from the journal/rows.
    #[error("corrupt bm25 cache: {0}")]
    Corrupt(&'static str),
    /// The cache path contained a parent-dir (`..`) traversal component and was
    /// refused.
    #[error("bm25 cache path rejected: parent-dir (`..`) component")]
    PathTraversal,
}
