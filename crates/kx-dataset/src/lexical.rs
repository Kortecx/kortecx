// SPDX-License-Identifier: Apache-2.0
//! [`LexicalIndex`] — the sparse / keyword (BM25) retrieval seam.
//!
//! A sibling of [`RetrievalIndex`](crate::RetrievalIndex): where that trait ranks
//! by dense-vector similarity, this one ranks by **lexical term overlap** (BM25).
//! The two legs are fused (rank-based, see [`crate::fusion`]) into one hybrid
//! result, which catches exact-term matches a weak decoder-LLM embedding misses.
//!
//! **SN-8 boundary (load-bearing).** Like dense retrieval, a lexical index is used
//! ONLY inside a ReadOnlyNondet retrieval Mote: the BM25 score is a display/ranking
//! aid that is fused and then discarded — only the ordered content-ref SET is
//! committed, matched downstream by exact hash. A score never reaches a `MoteId`.
//!
//! The concrete BM25 implementation (tokenizer + inverted index + rebuild-on-open
//! persistence) lives in the opt-in `kx-dataset-bm25` crate, behind this seam, so
//! the default build stays FFI-free and lexical search is a pluggable backend.

use kx_content::ContentRef;

use crate::index::Hit;

/// A lexical (keyword / BM25) similarity index, keyed by content ref. Used ONLY
/// inside ReadOnlyNondet retrieval Motes (see the module note / SN-8).
///
/// `insert` is idempotent by ref (content-addressed: a known ref already carries
/// this exact text). `query` tokenizes `query` with the index's own tokenizer and
/// returns the top-`k` by BM25, highest score first, with a deterministic
/// ascending-ref tiebreak (byte-identical to [`RetrievalIndex`](crate::RetrievalIndex)).
pub trait LexicalIndex {
    /// Index `text` under `id` (idempotent by ref).
    fn insert(&mut self, id: ContentRef, text: &str);

    /// Return the `k` highest-BM25 entries for `query`, best first.
    fn query(&self, query: &str, k: usize) -> Vec<Hit>;

    /// Number of indexed documents.
    fn len(&self) -> usize;

    /// `true` if the index is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
