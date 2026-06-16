//! Slice-B (D151) — the FuzzyDiscovery seam: ADVISORY fuzzy-in / exact-out
//! retrieval over a dataset's `RetrievalIndex`.
//!
//! A sibling of [`DatasetView`](crate::DatasetView), spoken in the same wire
//! vocabulary (`&[u8]` / `Vec<f32>` / `String`) so gateway-core gains NO dataset
//! crate dependency. A SEPARATE optional seam (not a new `DatasetView` method) so
//! it stays additive — an old host need not implement it — and can be wired
//! independently of the inline RAG path. The host (`kx-gateway`, behind `hnsw`)
//! implements it over the SAME `HostDatasetView` (one `Arc`, two seams).
//!
//! # Why a distinct contract (vs `QueryDataset`)
//!
//! `QueryDataset` echoes the document bytes + a float score for the inline RAG
//! path. FuzzyDiscovery is "fuzzy in, **exact out**": it returns ONLY the
//! content-addressed refs + a DISPLAY-ONLY integer basis-point score, and the
//! caller joins back to bytes with an EXACT `GetContent` on the ref.
//!
//! # SN-8 (load-bearing)
//!
//! [`FuzzyHitEntry::score_bp`] is DISPLAY-ONLY — it never enters a committed
//! fact, a `MoteId`, or any identity decision; only the ordered content-ref SET
//! is durable, matched downstream by EXACT hash. The approximate, build-order-
//! sensitive ANN ranking never reaches identity. A `None` seam ⇒ the RPC returns
//! `unimplemented` (old-gateway forward-compat degrade).

use kx_proto::proto;

use crate::datasets::DatasetError;

/// One advisory discovery hit: the exact-out join key + a display-only score.
#[derive(Clone, Copy, Debug)]
pub struct FuzzyHitEntry {
    /// The 32-byte content-addressed id of the candidate document (EXACT-OUT).
    pub content_ref: [u8; 32],
    /// The similarity, in basis points (0..=10000) — DISPLAY-ONLY (SN-8). NEVER
    /// an identity input; the host derives it from the approximate ANN score.
    pub score_bp: u32,
}

/// The advisory fuzzy-discovery seam. The host implements it over the same
/// `RetrievalIndex` + (optional) server embedder that backs [`crate::DatasetView`].
/// A `None` seam on the service ⇒ `FuzzyDiscovery` returns `unimplemented`.
pub trait FuzzyDiscoveryView: Send + Sync {
    /// Discover the top-`k` candidate documents in `dataset`. `query_embedding`
    /// (`Some`) is the client-vector path (FFI-free); `None` falls back to
    /// embedding `query_text` (needs a server embedder). Best-first; the result
    /// is the ordered content-ref SET (the score is advisory display only).
    ///
    /// # Errors
    /// [`DatasetError`] — the same honest codes as [`crate::DatasetView::query`]
    /// (`not_found` / `invalid_argument` / `failed_precondition` / `internal`).
    fn discover(
        &self,
        dataset: &str,
        query_embedding: Option<&[f32]>,
        query_text: &str,
        k: usize,
    ) -> Result<Vec<FuzzyHitEntry>, DatasetError>;
}

/// Convert an approximate cosine similarity into a DISPLAY-ONLY basis-point score
/// (0..=10000). A non-finite or out-of-range score (cosine can be negative for
/// opposed vectors) is clamped — this value is for the eye only (SN-8), never an
/// identity or ordering input on the wire.
#[must_use]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn score_to_bp(score: f32) -> u32 {
    if !score.is_finite() {
        return 0;
    }
    // clamp to [0,1] then scale to basis points; round-to-nearest.
    // SAFETY: the clamped, rounded value is a finite f32 in [0.0, 10000.0] — the
    // `as u32` is exact (non-negative, well under u32::MAX): no truncation/sign loss.
    let bp = (score.clamp(0.0, 1.0) * 10_000.0).round();
    bp as u32
}

/// Map a gateway-core fuzzy hit into the wire type.
pub(crate) fn fuzzy_hit_to_proto(h: FuzzyHitEntry) -> proto::FuzzyHit {
    proto::FuzzyHit {
        content_ref: h.content_ref.to_vec(),
        score_bp: h.score_bp,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn score_to_bp_clamps_and_scales() {
        assert_eq!(score_to_bp(1.0), 10_000);
        assert_eq!(score_to_bp(0.0), 0);
        assert_eq!(score_to_bp(0.5), 5_000);
        // out-of-[0,1] finite scores clamp to the display band; non-finite (NaN/±inf,
        // which a cosine score never is) defaults to 0 — defensive, never panics.
        assert_eq!(score_to_bp(1.5), 10_000);
        assert_eq!(score_to_bp(-0.3), 0);
        assert_eq!(score_to_bp(f32::NAN), 0);
        assert_eq!(score_to_bp(f32::INFINITY), 0);
    }

    #[test]
    fn fuzzy_hit_maps_ref_and_bp_to_wire() {
        let h = FuzzyHitEntry {
            content_ref: [7u8; 32],
            score_bp: 4_242,
        };
        let p = fuzzy_hit_to_proto(h);
        assert_eq!(p.content_ref, vec![7u8; 32]);
        assert_eq!(p.score_bp, 4_242);
    }
}
