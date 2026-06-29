// SPDX-License-Identifier: Apache-2.0
//! Hybrid retrieval fusion + diversity rerank + the retrieval-index fingerprint.
//!
//! All three are **pure, deterministic** functions over ranks / vectors / config:
//!
//! - [`rrf_fuse`] — Reciprocal Rank Fusion of a dense (vector) ranking and a sparse
//!   (BM25) ranking into one list. Rank-based, so the incomparable score scales
//!   (cosine ∈ [−1,1] vs BM25 ∈ [0,∞)) never need normalizing.
//! - [`mmr_rerank`] — Maximal Marginal Relevance: greedily reorders a candidate
//!   pool to trade relevance against redundancy (diversity), demoting near-duplicates.
//! - [`index_fingerprint`] — a 32-byte content hash of everything that makes a
//!   built index incompatible with a differently-configured query (embed model /
//!   pooling / dim / chunk params / tokenizer), so a stale index is detected rather
//!   than silently mis-queried.
//!
//! **SN-8.** Fusion + rerank run INSIDE the ReadOnlyNondet retrieval boundary,
//! BEFORE the result commits. They consume ranks/vectors and produce an order; the
//! committed fact is still the ordered content-ref SET (scores excluded). Every
//! ordered boundary has an explicit `ascending content-ref` tiebreak, so the result
//! order is byte-identical across machines for a fixed index state + query + params.
//!
//! The float-math lints are scoped to this module: scores narrow f64→f32 for the
//! display-only `Hit.score` (truncation is intentional), rank/stat casts are
//! precision-safe for any corpus, exact float comparison is a deliberate
//! bit-exact tie test, and the fingerprint hashes 8 independent config axes.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::float_cmp,
    clippy::too_many_arguments
)]

use std::collections::BTreeMap;

use kx_content::ContentRef;

use crate::index::Hit;

/// The canonical RRF constant (Cormack et al. 2009). Damps the contribution of
/// any single ranker's tail; excluded from [`index_fingerprint`] (it reorders
/// display ranks only, never what was indexed).
pub const RRF_C: u32 = 60;

/// The default MMR relevance/diversity trade-off, as integer basis points
/// (`7000` = 0.70, relevance-leaning). Config-pinned; excluded from the fingerprint.
pub const MMR_LAMBDA_BP: u32 = 7000;

/// The on-disk retrieval-index format version baked into [`index_fingerprint`].
/// Bump on any layout/algorithm change that invalidates a built index.
pub const INDEX_FORMAT_VERSION: u32 = 1;

/// Fuse a dense ranking and a sparse ranking via Reciprocal Rank Fusion.
///
/// Each input is assumed already in its backend's deterministic order (score desc,
/// ref asc). For each list a document's contribution is `1 / (c + rank)` (rank
/// 1-based); a document absent from a list contributes nothing from it. The fused
/// `Hit.score` is the summed RRF weight (display-only); the output is sorted RRF
/// desc, then ascending ref, and truncated to `k`.
///
/// Rank-based ⇒ no score normalization and the two scales fuse cleanly. The union
/// is built in sorted-ref order (a `BTreeMap`), never HashMap iteration order, so
/// the result is reproducible.
#[must_use]
pub fn rrf_fuse(dense: &[Hit], sparse: &[Hit], c: u32, k: usize) -> Vec<Hit> {
    if k == 0 {
        return Vec::new();
    }
    let mut acc: BTreeMap<ContentRef, f64> = BTreeMap::new();
    for list in [dense, sparse] {
        for (rank0, hit) in list.iter().enumerate() {
            let rank = (rank0 as u64) + 1;
            let denom = f64::from(c) + rank as f64;
            *acc.entry(hit.id).or_insert(0.0) += 1.0 / denom;
        }
    }
    let mut fused: Vec<Hit> = acc
        .into_iter()
        .map(|(id, score)| Hit {
            id,
            score: score as f32,
        })
        .collect();
    // RRF desc, then ascending content ref (the deterministic tiebreak).
    fused.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.id.as_bytes().cmp(b.id.as_bytes()))
    });
    fused.truncate(k);
    fused
}

/// Reorder `candidates` by Maximal Marginal Relevance to diversify the top-`out_k`.
///
/// `MMR(d) = λ·rel(d) − (1 − λ)·max_{s ∈ selected} cos(d, s)`, where `rel(d)` is the
/// candidate's INCOMING retriever score (cosine for a dense list, the RRF weight for
/// a fused list) **min-max normalized to `[0,1]` over the candidate set** — so the
/// fusion signal is preserved (MMR diversifies the fused ranking, it does NOT
/// recompute relevance from the dense vector and discard the BM25 contribution).
/// `vector_of` supplies a candidate's embedding for the redundancy (diversity) term;
/// a candidate with no vector has `0` redundancy (never panics). The first pick is
/// the most relevant; each subsequent pick trades relevance against the strongest
/// similarity to anything already selected, demoting near-duplicates. Ties break by
/// ascending ref. Deterministic given identical inputs.
#[must_use]
pub fn mmr_rerank(
    candidates: &[Hit],
    vector_of: impl Fn(&ContentRef) -> Option<Vec<f32>>,
    lambda: f32,
    out_k: usize,
) -> Vec<Hit> {
    let want = out_k.min(candidates.len());
    if want == 0 {
        return Vec::new();
    }

    // Min-max normalize the incoming relevance (retriever/fused score) to [0,1] so the
    // λ trade-off is scale-robust regardless of cosine vs RRF. All-equal ⇒ rel = 1.
    let (min_s, max_s) = candidates
        .iter()
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), h| {
            (lo.min(h.score), hi.max(h.score))
        });
    let span = max_s - min_s;
    let cand: Vec<(Hit, Option<Vec<f32>>, f32)> = candidates
        .iter()
        .map(|h| {
            let v = vector_of(&h.id);
            let rel = if span > 0.0 {
                (h.score - min_s) / span
            } else {
                1.0
            };
            (*h, v, rel)
        })
        .collect();

    let mut remaining: Vec<usize> = (0..cand.len()).collect();
    let mut selected: Vec<usize> = Vec::with_capacity(want);

    while selected.len() < want {
        let mut best: Option<(usize, f32)> = None; // (index-into-remaining, mmr)
        for (ri, &ci) in remaining.iter().enumerate() {
            let rel = cand[ci].2;
            // Max similarity to anything already selected (0 when none yet).
            let redundancy = selected
                .iter()
                .filter_map(|&si| match (&cand[ci].1, &cand[si].1) {
                    (Some(a), Some(b)) => Some(cosine(a, b)),
                    _ => None,
                })
                .fold(0.0f32, f32::max);
            let mmr = lambda * rel - (1.0 - lambda) * redundancy;
            let better = match best {
                None => true,
                Some((bri, bmmr)) => {
                    // mmr desc, then ascending ref (total, deterministic) tiebreak.
                    mmr > bmmr
                        || (mmr == bmmr
                            && cand[ci].0.id.as_bytes() < cand[remaining[bri]].0.id.as_bytes())
                }
            };
            if better {
                best = Some((ri, mmr));
            }
        }
        // `remaining` is non-empty while `selected.len() < want <= cand.len()`;
        // `let else` keeps it panic-free regardless.
        let Some((ri, _)) = best else { break };
        selected.push(remaining.swap_remove(ri));
    }

    selected.into_iter().map(|i| cand[i].0).collect()
}

/// A 32-byte content hash of everything that makes a *built* index incompatible
/// with a *differently-configured* query. Stored at first ingest and compared on
/// later ingest/query: a mismatch means the index was built with a different embed
/// model / pooling / dimension / chunk params / tokenizer, so its vectors live in
/// a different space and must be rebuilt (re-ingested) — never silently queried.
///
/// Excludes the display-tuning constants (`k1`/`b`/`λ`/`RRF_C`): they reorder
/// scores only, never what was indexed, so they may change without a rebuild.
#[must_use]
pub fn index_fingerprint(
    embed_model_id: &str,
    embed_pooling: u8,
    embed_dim: u32,
    chunker_version: u32,
    chunk_max_chars: u32,
    chunk_overlap: u32,
    tokenizer_version: u32,
    stopwords: bool,
) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(b"kx-retrieval/index-fingerprint/v1");
    h.update(&INDEX_FORMAT_VERSION.to_le_bytes());
    // Length-prefixed model id so two ids never collide by concatenation.
    h.update(&(embed_model_id.len() as u64).to_le_bytes());
    h.update(embed_model_id.as_bytes());
    h.update(&[embed_pooling]);
    h.update(&embed_dim.to_le_bytes());
    h.update(&chunker_version.to_le_bytes());
    h.update(&chunk_max_chars.to_le_bytes());
    h.update(&chunk_overlap.to_le_bytes());
    h.update(&tokenizer_version.to_le_bytes());
    h.update(&[u8::from(stopwords)]);
    *h.finalize().as_bytes()
}

/// Cosine similarity in `[-1, 1]`; `0.0` on a dimension mismatch or a zero-norm
/// vector (no direction) — pure + total, never `NaN`.
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
