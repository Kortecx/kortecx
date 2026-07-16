//! [`RetrievalIndex`] — the vector / graph-RAG similarity seam.
//!
//! **SN-8 boundary (load-bearing).** Similarity search lives here, and it MUST
//! stay *inside* the ReadOnlyNondet retrieval boundary: a retrieval Mote queries
//! an index as an input-gathering act, then commits its result as a
//! content-addressed fact that everything downstream consumes by **exact** hash.
//! Similarity is NEVER an operator on the identity / commit / memoization path
//! (the runtime matches by exact cryptographic equality only). An approximate
//! (ANN) backend is non-deterministic by nature — another reason it is confined
//! behind this trait and never folded into a `MoteId`.
//!
//! [`InMemoryRetrievalIndex`] is an exact brute-force scan (deterministic,
//! suitable for tests + small corpora); a real ANN/Lance backend implements the
//! same trait in a later gated step.

use kx_content::ContentRef;
use std::collections::HashMap;

/// One retrieval result: a content ref and its similarity score (higher = closer).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    /// The retrieved payload's content-addressed identity.
    pub id: ContentRef,
    /// Cosine similarity in `[-1, 1]` (exact backend) or an approximate score.
    pub score: f32,
}

/// A similarity index over embedding vectors, keyed by content ref. Used ONLY
/// inside ReadOnlyNondet retrieval Motes (see the module note / SN-8).
pub trait RetrievalIndex {
    /// Add (or overwrite) the vector for `id`.
    fn insert(&mut self, id: ContentRef, vector: Vec<f32>);

    /// Return the `k` nearest entries to `query`, highest score first.
    /// Dimension-mismatched entries are skipped.
    fn query(&self, query: &[f32], k: usize) -> Vec<Hit>;

    /// Number of indexed vectors.
    fn len(&self) -> usize;

    /// `true` if the index is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The stored vector for `id`, if present. Defaults to `None` so existing
    /// backends compile unchanged; backends that retain their vectors (the
    /// in-memory + HNSW indices) override it. Used by MMR diversity rerank
    /// ([`crate::fusion::mmr_rerank`]) to measure candidate-to-candidate
    /// redundancy without re-embedding.
    fn vector_of(&self, _id: &ContentRef) -> Option<Vec<f32>> {
        None
    }
}

/// An exact brute-force cosine-similarity index. Deterministic: ties break by
/// ascending content ref, so identical inputs yield an identical result order.
#[derive(Default)]
pub struct InMemoryRetrievalIndex {
    items: Vec<(ContentRef, Vec<f32>)>,
    /// Ref -> its slot in `items`, so `insert` dedups in O(1) instead of
    /// scanning `items`. Kept exactly in sync with `items` (every ref in
    /// `items` has one entry here pointing at its index); it is purely an
    /// acceleration structure and never alters iteration order or results.
    positions: HashMap<ContentRef, usize>,
}

impl InMemoryRetrievalIndex {
    /// Create an empty index.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

/// Cosine similarity; `0.0` if either vector has zero norm or the dimensions
/// differ.
fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    let denom = na.sqrt() * nb.sqrt();
    if denom == 0.0 {
        0.0
    } else {
        dot / denom
    }
}

impl RetrievalIndex for InMemoryRetrievalIndex {
    fn insert(&mut self, id: ContentRef, vector: Vec<f32>) {
        if let Some(&slot) = self.positions.get(&id) {
            self.items[slot].1 = vector;
        } else {
            self.positions.insert(id, self.items.len());
            self.items.push((id, vector));
        }
    }

    fn query(&self, query: &[f32], k: usize) -> Vec<Hit> {
        let mut scored: Vec<Hit> = self
            .items
            .iter()
            .map(|(id, v)| Hit {
                id: *id,
                score: cosine(query, v),
            })
            .collect();
        // Deterministic order: score descending (total_cmp handles NaN/ties
        // totally), then ascending content ref as the stable tiebreak.
        scored.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then_with(|| a.id.as_bytes().cmp(b.id.as_bytes()))
        });
        scored.truncate(k);
        scored
    }

    fn len(&self) -> usize {
        self.items.len()
    }

    fn vector_of(&self, id: &ContentRef) -> Option<Vec<f32>> {
        self.items
            .iter()
            .find(|(existing, _)| existing == id)
            .map(|(_, v)| v.clone())
    }
}
