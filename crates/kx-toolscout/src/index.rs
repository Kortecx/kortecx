//! [`ToolManifestIndex`] — manifests + (optional) embedding vectors behind the
//! SN-8-confined [`RetrievalIndex`] seam.
//!
//! The index RANKS; it never grants. Its output is `(ContentRef, score-bp)`
//! pairs for a picker/preview surface — the score type never crosses into
//! [`crate::lower_to_workflow_def`] (whose signature admits no score), and the
//! committed world only ever sees exact identities.

use std::collections::BTreeMap;

use kx_bundle::TaskBundle;
use kx_content::ContentRef;
use kx_dataset::RetrievalIndex;

use crate::fingerprint::ToolFingerprint;
use crate::score::fingerprint_tolerance_score;

/// A caller-supplied text-embedding function (the `FuzzyDiscovery` stance:
/// embeddings are OPAQUE, caller-supplied vectors — this crate never computes
/// or ships a model). Absent ⇒ the string rungs alone rank (neutral fallback).
pub trait Embedder {
    /// Embed `text` into the SAME vector space the index's vectors use.
    fn embed(&self, text: &str) -> Vec<f32>;
}

/// The advisory tool-manifest index: fingerprints keyed by their content hash,
/// with optional embedding vectors living in any [`RetrievalIndex`] backend
/// (exact `InMemoryRetrievalIndex` for reproducible sets; the `hnsw` backend at
/// scale — both already behind the seam).
pub struct ToolManifestIndex<I: RetrievalIndex> {
    vectors: I,
    manifests: BTreeMap<ContentRef, ToolFingerprint>,
}

impl<I: RetrievalIndex> ToolManifestIndex<I> {
    /// Wrap a vector backend (usually freshly constructed).
    pub fn new(vectors: I) -> Self {
        Self {
            vectors,
            manifests: BTreeMap::new(),
        }
    }

    /// Register a manifest, optionally with a description-embedding vector.
    /// Vectorless manifests still rank via the string rungs.
    pub fn insert(&mut self, fp: ToolFingerprint, vector: Option<Vec<f32>>) {
        let key = fp.fingerprint_hash();
        if let Some(v) = vector {
            self.vectors.insert(key, v);
        }
        self.manifests.insert(key, fp);
    }

    /// Number of registered manifests.
    #[must_use]
    pub fn len(&self) -> usize {
        self.manifests.len()
    }

    /// `true` when no manifest is registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.manifests.is_empty()
    }

    /// Look a manifest up by its content hash (e.g. to render a ranked hit).
    #[must_use]
    pub fn manifest(&self, key: &ContentRef) -> Option<&ToolFingerprint> {
        self.manifests.get(key)
    }

    /// Rank every registered manifest against the bundle's intent, best
    /// first, deterministically: score descending, then ascending
    /// `ContentRef` (the same tiebreak as `InMemoryRetrievalIndex`). With an
    /// embedder, rung 3 consults the vector backend for each manifest's
    /// cosine; without one the string rungs alone decide.
    ///
    /// ADVISORY output: `(manifest key, score-bp)` for display/ordering only.
    #[must_use]
    pub fn rank(
        &self,
        bundle: &TaskBundle,
        embedder: Option<&dyn Embedder>,
        k: usize,
    ) -> Vec<(ContentRef, u16)> {
        // Resolve rung-3 cosines in ONE backend query (score desc by contract).
        let cosines: BTreeMap<ContentRef, f32> = match embedder {
            Some(e) => {
                let query = e.embed(&bundle.intent);
                self.vectors
                    .query(&query, self.vectors.len())
                    .into_iter()
                    .map(|hit| (hit.id, hit.score))
                    .collect()
            }
            None => BTreeMap::new(),
        };

        let mut scored: Vec<(ContentRef, u16)> = self
            .manifests
            .iter()
            .map(|(key, fp)| {
                let cos = cosines.get(key).copied();
                (*key, fingerprint_tolerance_score(bundle, fp, cos))
            })
            .collect();
        scored.sort_by(|a, b| {
            b.1.cmp(&a.1)
                .then_with(|| a.0.as_bytes().cmp(b.0.as_bytes()))
        });
        scored.truncate(k);
        scored
    }
}
