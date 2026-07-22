// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! The in-memory BM25 inverted index.

use std::collections::HashMap;

use kx_content::ContentRef;
use kx_dataset::{Hit, LexicalIndex};

use crate::tokenize::tokenize;

/// Okapi BM25 scoring parameters. `k1` controls term-frequency saturation; `b`
/// controls document-length normalization. The defaults (`k1 = 1.2`, `b = 0.75`)
/// are the classic Robertson/Sparck-Jones values. `stopwords` toggles the fixed
/// English stoplist (default off). `k1`/`b` are display-tuning only (they reorder
/// scores, never the term set) and are deliberately EXCLUDED from the index
/// fingerprint; the tokenizer + `stopwords` ARE fingerprinted.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bm25Params {
    /// Term-frequency saturation (`k1`).
    pub k1: f32,
    /// Document-length normalization (`b`).
    pub b: f32,
    /// Drop a fixed English stoplist when tokenizing.
    pub stopwords: bool,
}

impl Default for Bm25Params {
    fn default() -> Self {
        Self {
            k1: 1.2,
            b: 0.75,
            stopwords: false,
        }
    }
}

/// A hand-rolled in-memory BM25 inverted index over content-addressed documents,
/// implementing the shared `LexicalIndex` seam. Retains the raw `(ref, text)`
/// records so it persists as a rebuildable cache (the inverted index itself is
/// never serialized — it is re-tokenized on open).
///
/// **Content-addressed inserts.** A known ref already carries this exact text, so
/// `insert` is idempotent by ref (a repeat is a no-op).
pub struct Bm25Index {
    params: Bm25Params,
    /// term -> postings `(doc_id, term-frequency)`, appended in ascending doc_id.
    postings: HashMap<String, Vec<(u32, u32)>>,
    /// `doc_id` (index) -> the content ref it stands for.
    ids: Vec<ContentRef>,
    /// content ref -> `doc_id`, for idempotent-by-ref inserts.
    by_ref: HashMap<ContentRef, u32>,
    /// `doc_id` -> the raw text, retained so the index persists as a rebuildable
    /// cache (the inverted index is rebuilt by re-tokenizing on open).
    texts: Vec<String>,
    /// `doc_id` -> token count (document length).
    doc_len: Vec<u32>,
    /// Sum of all document lengths (for `avgdl`).
    total_len: u64,
}

impl Bm25Index {
    /// Create an empty index with default parameters.
    pub fn new() -> Self {
        Self::with_params(Bm25Params::default())
    }

    /// Create an empty index with explicit parameters.
    pub fn with_params(params: Bm25Params) -> Self {
        Self {
            params,
            postings: HashMap::new(),
            ids: Vec::new(),
            by_ref: HashMap::new(),
            texts: Vec::new(),
            doc_len: Vec::new(),
            total_len: 0,
        }
    }

    /// Snapshot for persistence: the `(ref, text)` records in `doc_id` order. The
    /// inverted index is intentionally NOT exported — it is rebuilt from these
    /// records on `open`.
    pub(crate) fn snapshot(&self) -> (&[ContentRef], &[String]) {
        (&self.ids, &self.texts)
    }
}

impl Default for Bm25Index {
    fn default() -> Self {
        Self::new()
    }
}

impl LexicalIndex for Bm25Index {
    fn insert(&mut self, id: ContentRef, text: &str) {
        // Content-addressed: a known ref already carries this exact text → no-op.
        if self.by_ref.contains_key(&id) {
            return;
        }
        // doc_id is a u32 (compact postings); refuse beyond the addressable range
        // rather than wrap (unreachable on a single-node corpus).
        if self.ids.len() >= u32::MAX as usize {
            return;
        }
        let doc_id = self.ids.len() as u32;
        let tokens = tokenize(text, self.params.stopwords);
        // Per-document term frequencies.
        let mut tf: HashMap<&str, u32> = HashMap::new();
        for t in &tokens {
            *tf.entry(t.as_str()).or_insert(0) += 1;
        }
        for (term, freq) in tf {
            self.postings
                .entry(term.to_string())
                .or_default()
                .push((doc_id, freq));
        }
        let dl = u32::try_from(tokens.len()).unwrap_or(u32::MAX);
        self.by_ref.insert(id, doc_id);
        self.ids.push(id);
        self.texts.push(text.to_string());
        self.doc_len.push(dl);
        self.total_len += u64::from(dl);
    }

    fn query(&self, query: &str, k: usize) -> Vec<Hit> {
        if k == 0 || self.ids.is_empty() {
            return Vec::new();
        }
        // Unique query terms in SORTED order ⇒ deterministic per-doc accumulation.
        let mut terms = tokenize(query, self.params.stopwords);
        terms.sort();
        terms.dedup();
        if terms.is_empty() {
            return Vec::new();
        }

        let n = self.ids.len() as f64;
        // Empty corpus is handled above; an all-empty-document corpus has no
        // postings, so `avgdl`'s exact value is irrelevant — guard div-by-zero.
        let avgdl = if self.total_len == 0 {
            1.0
        } else {
            self.total_len as f64 / n
        };
        let k1 = f64::from(self.params.k1);
        let b = f64::from(self.params.b);

        let mut scores: HashMap<u32, f64> = HashMap::new();
        for term in &terms {
            let Some(postings) = self.postings.get(term) else {
                continue;
            };
            let df = postings.len() as f64;
            // BM25+ non-negative IDF: ln(1 + (N − df + 0.5) / (df + 0.5)) ≥ 0.
            let idf = (1.0 + (n - df + 0.5) / (df + 0.5)).ln();
            for &(doc_id, tf) in postings {
                let tf = f64::from(tf);
                let dl = f64::from(self.doc_len[doc_id as usize]);
                let denom = tf + k1 * (1.0 - b + b * dl / avgdl);
                let contrib = idf * (tf * (k1 + 1.0)) / denom;
                *scores.entry(doc_id).or_insert(0.0) += contrib;
            }
        }

        let mut hits: Vec<Hit> = scores
            .into_iter()
            .map(|(doc_id, score)| Hit {
                id: self.ids[doc_id as usize],
                score: score as f32,
            })
            .collect();
        // Deterministic order: BM25 desc, then ascending content ref (byte-identical
        // tiebreak to the dense backends).
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
}
