// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! Errors for the HNSW retrieval backend.

/// An error opening, persisting, or decoding the HNSW cache.
///
/// The cache is a rebuildable projection (D40), so callers recover from
/// `Corrupt` / `Io` by rebuilding the index from the journal — never by trusting
/// a partially-decoded file.
#[derive(Debug, thiserror::Error)]
pub enum HnswError {
    /// An I/O error reading or writing the cache file.
    #[error("hnsw cache i/o: {0}")]
    Io(#[from] std::io::Error),
    /// The cache file is malformed (bad header, truncation, or trailing bytes);
    /// rebuild the index from the journal.
    #[error("corrupt hnsw cache: {0}")]
    Corrupt(&'static str),
    /// The cache path contained a parent-dir (`..`) traversal component and was
    /// refused.
    #[error("hnsw cache path rejected: parent-dir (`..`) component")]
    PathTraversal,
}
