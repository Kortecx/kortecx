// SPDX-License-Identifier: Apache-2.0
//! The HNSW retrieval index — an approximate-nearest-neighbour `RetrievalIndex`.

use std::collections::HashMap;

use hnsw_rs::prelude::{DistCosine, Hnsw};
use kx_content::ContentRef;
use kx_dataset::{Hit, RetrievalIndex};

/// Construction + search parameters for the HNSW graph.
///
/// The defaults target a 10k–1M-vector single-node corpus. Raise
/// `max_nb_connection` / `ef_construction` for higher build recall (at the cost
/// of build time + memory), and `ef_search` for higher query recall.
#[derive(Clone, Copy, Debug)]
pub struct HnswParams {
    /// `M` — maximum neighbour links per node per layer (graph degree).
    pub max_nb_connection: usize,
    /// `efConstruction` — candidate-list width during insertion (build recall).
    pub ef_construction: usize,
    /// Maximum graph layer count (Malkov-Yashunin `mL`).
    pub max_layer: usize,
    /// Expected element count — a graph-sizing hint; inserting beyond it is allowed.
    pub capacity_hint: usize,
    /// `ef` — candidate-list width during search (query recall). Clamped up to `k`.
    pub ef_search: usize,
}

impl Default for HnswParams {
    fn default() -> Self {
        Self {
            max_nb_connection: 16,
            ef_construction: 200,
            max_layer: 16,
            capacity_hint: 10_000,
            ef_search: 64,
        }
    }
}

/// A file-backed, in-process approximate-nearest-neighbour index over embedding
/// vectors, implementing the shared `RetrievalIndex` seam via `hnsw_rs`.
///
/// **SN-8.** Like every `RetrievalIndex`, this is used ONLY inside the
/// ReadOnlyNondet retrieval Mote: similarity stays inside, and only the ordered
/// neighbour-ref SET is committed (matched downstream by exact hash). The
/// approximate, build-order-sensitive nature of HNSW is therefore safe — a score
/// never reaches a `MoteId`. For reproducible-by-reference corpora the exact
/// `InMemoryRetrievalIndex` remains the default; this is the opt-in scale path.
///
/// **Content-addressed inserts.** Because ids are content refs (D17), the same
/// ref always carries the same vector, so `insert` is idempotent by ref (a repeat
/// is a no-op). A single embedding dimension is assumed per index; a
/// dimension-mismatched insert is skipped.
pub struct HnswRetrievalIndex {
    hnsw: Hnsw<'static, f32, DistCosine>,
    params: HnswParams,
    /// `DataId` (the index into this Vec) -> the content ref it stands for.
    ids: Vec<ContentRef>,
    /// Content ref -> `DataId`, for idempotent-by-ref inserts.
    by_ref: HashMap<ContentRef, usize>,
    /// `DataId` -> the raw vector, retained so the index persists as a rebuildable
    /// cache (the HNSW graph itself is never serialized).
    vectors: Vec<Vec<f32>>,
    /// The fixed embedding dimension, set by the first insert.
    dim: Option<usize>,
}

impl HnswRetrievalIndex {
    /// Create an empty index with default parameters.
    pub fn new() -> Self {
        Self::with_params(HnswParams::default())
    }

    /// Create an empty index with explicit parameters.
    pub fn with_params(params: HnswParams) -> Self {
        let hnsw: Hnsw<'static, f32, DistCosine> = Hnsw::new(
            params.max_nb_connection,
            params.capacity_hint.max(1),
            params.max_layer,
            params.ef_construction,
            DistCosine,
        );
        Self {
            hnsw,
            params,
            ids: Vec::new(),
            by_ref: HashMap::new(),
            vectors: Vec::new(),
            dim: None,
        }
    }

    /// The embedding dimension once known (after the first insert).
    pub fn dim(&self) -> Option<usize> {
        self.dim
    }

    /// Snapshot for persistence: the dimension + the `(ref, vector)` records in
    /// `DataId` order. The HNSW graph is intentionally NOT exported — it is
    /// rebuilt from these records on `open`.
    pub(crate) fn snapshot(&self) -> (u32, &[ContentRef], &[Vec<f32>]) {
        let dim = u32::try_from(self.dim.unwrap_or(0)).unwrap_or(u32::MAX);
        (dim, &self.ids, &self.vectors)
    }
}

impl Default for HnswRetrievalIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl RetrievalIndex for HnswRetrievalIndex {
    fn insert(&mut self, id: ContentRef, vector: Vec<f32>) {
        // Content-addressed: a known ref already carries this exact vector → no-op.
        if self.by_ref.contains_key(&id) {
            return;
        }
        // One embedding dimension per index; the first insert fixes it.
        match self.dim {
            None => self.dim = Some(vector.len()),
            Some(d) if d == vector.len() => {}
            Some(_) => return, // skip a dimension-mismatched insert
        }
        let data_id = self.ids.len();
        self.hnsw.insert((vector.as_slice(), data_id));
        self.by_ref.insert(id, data_id);
        self.ids.push(id);
        self.vectors.push(vector);
    }

    fn query(&self, query: &[f32], k: usize) -> Vec<Hit> {
        if k == 0 || self.ids.is_empty() {
            return Vec::new();
        }
        // A wrong-dimension query has no meaningful neighbours in this index.
        if self.dim.is_none_or(|d| d != query.len()) {
            return Vec::new();
        }
        let ef = self.params.ef_search.max(k);
        // EXACT search when the corpus fits the search width (`n <= ef`): HNSW's
        // approximate graph traversal buys nothing once `ef >= n`, and on a TINY
        // graph the crate's randomized layer assignment can occasionally MISS the
        // true nearest neighbour even with `ef >= n` (the
        // `T-DATASETS-HNSW-DISCOVER-FLAKE` class — a non-deterministic top-hit on a
        // small corpus). A brute-force pass over the stored vectors is exhaustive,
        // deterministic, and cheaper here; above `ef` we keep the HNSW path
        // unchanged for large corpora. `score = cosine similarity` matches the HNSW
        // arm's `1.0 - DistCosine` (DistCosine == 1 - cosine_similarity).
        let mut hits: Vec<Hit> = if self.ids.len() <= ef {
            self.vectors
                .iter()
                .enumerate()
                .map(|(data_id, v)| Hit {
                    id: self.ids[data_id],
                    score: cosine_similarity(query, v),
                })
                .collect()
        } else {
            self.hnsw
                .search(query, k, ef)
                .iter()
                .filter_map(|n| {
                    self.ids.get(n.get_origin_id()).map(|&id| Hit {
                        id,
                        // DistCosine distance == 1 - cosine_similarity → similarity == 1 - distance.
                        score: 1.0 - n.get_distance(),
                    })
                })
                .collect()
        };
        // Deterministic order for the committed ordered-ref fact: score desc, then
        // ascending content ref — mirrors InMemoryRetrievalIndex's stable tiebreak.
        hits.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then_with(|| a.id.as_bytes().cmp(b.id.as_bytes()))
        });
        hits.truncate(k);
        hits
    }

    fn len(&self) -> usize {
        self.ids.len()
    }

    fn vector_of(&self, id: &ContentRef) -> Option<Vec<f32>> {
        self.by_ref
            .get(id)
            .and_then(|&i| self.vectors.get(i))
            .cloned()
    }
}

/// Cosine similarity in `[-1, 1]`, matching `1.0 - DistCosine` (the HNSW arm's
/// score). Used by the exact small-corpus path in [`HnswRetrievalIndex::query`].
/// Pure + total: a zero-norm vector (no direction) yields `0.0` rather than a
/// `NaN` division; inputs are already validated finite upstream.
fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
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
