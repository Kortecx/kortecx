//! RAG ingest + query glue (DP2) — the executable side of the `rag_pipeline`
//! recipe ([`kx_workflow::rag_pipeline`]).
//!
//! The recipe *structure* is pure data in `kx-workflow` (no FFI). The *execution*
//! — actually embedding a corpus, populating a [`RetrievalIndex`], and running a
//! query — lives HERE because it needs the FFI + dataset deps `kx-workflow` must
//! not carry (`kx-workflow` deps `kx-dataset` but NOT `kx-inference`/`kx-llamacpp`).
//! This mirrors how `synthesis_pipeline` (structure) is separate from
//! `ModelExecutor`/`ModelBroker` (execution).
//!
//! # SN-8 boundary
//!
//! [`query_corpus`] returns the committed-fact ref ([`kx_workflow::retrieval_result_ref`]
//! — the ORDERED content refs, **scores excluded**) AND the raw [`Hit`]s (scores
//! included). The scores are for the CALLER's display / ranking only — they never
//! enter the committed fact, a `MoteId`, or memoization. Downstream steps consume
//! the retrieved content by EXACT hash.

use kx_content::ContentRef;
use kx_dataset::{ContentSchema, DataError, DataStore, Dataset, Hit, RetrievalIndex};
use kx_inference::{EmbeddingBackend, EmbeddingOutput, EmbeddingPooling, InferenceError};
use kx_mote::{ModelId, MoteId};
use kx_warrant::WarrantSpec;
use kx_workflow::retrieval_result_ref;
use thiserror::Error;

/// Failure modes of the RAG ingest/query glue.
#[derive(Debug, Error)]
pub enum RagError {
    /// The embedding backend failed (or does not support embeddings).
    #[error("embedding: {0}")]
    Embedding(#[from] InferenceError),
    /// The data store failed (poisoned lock).
    #[error("data store: {0}")]
    Store(#[from] DataError),
}

/// Canonical **little-endian f32** encoding of an embedding vector — the
/// reproducible content-addressed form.
///
/// `kx-dataset` content-addresses (blake3) the RAW bytes passed to `put_typed`;
/// `ContentSchema::Vector{dim}` is only a type tag and performs NO
/// canonicalization. So the encoding MUST be fixed here, or the same embedding
/// would hash differently across machines / endiannesses and break `DatasetId`
/// reproducibility.
#[must_use]
pub fn encode_vector_le(vector: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(vector.len() * 4);
    for v in vector {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// The embedding context: a backend + the model route + warrant + pooling that
/// travel together for every embed call. Bundling them keeps [`ingest_corpus`] /
/// [`query_corpus`] to a small, clear signature and pins the
/// reproducibility-bearing axes (`model_id` + `pooling`) in one place.
pub struct Embedder<'a> {
    backend: &'a dyn EmbeddingBackend,
    model_id: &'a ModelId,
    warrant: &'a WarrantSpec,
    pooling: EmbeddingPooling,
}

impl<'a> Embedder<'a> {
    /// Bind a backend + model route + warrant + pooling for the corpus.
    #[must_use]
    pub fn new(
        backend: &'a dyn EmbeddingBackend,
        model_id: &'a ModelId,
        warrant: &'a WarrantSpec,
        pooling: EmbeddingPooling,
    ) -> Self {
        Self {
            backend,
            model_id,
            warrant,
            pooling,
        }
    }

    /// Embed `text` under this context (route-gated by the backend).
    ///
    /// # Errors
    /// Whatever [`EmbeddingBackend::dispatch_embedding`] returns.
    pub fn embed(&self, text: &str) -> Result<EmbeddingOutput, InferenceError> {
        self.backend
            .dispatch_embedding(self.model_id, text, self.pooling, self.warrant)
    }
}

/// Embed + store + index a corpus, returning the reproducible [`Dataset`].
///
/// For each document, in order:
/// 1. embed it ([`EmbeddingBackend::dispatch_embedding`], under `pooling`);
/// 2. store the SOURCE TEXT as a `ContentSchema::Text` row — the retrievable
///    payload AND the index key (so retrieval returns refs a caller can fetch);
/// 3. store the embedding as a canonical-LE `ContentSchema::Vector{dim}` row;
/// 4. `index.insert(text_ref, vector)`.
///
/// Content-addressed dedup is free: a duplicate document → the same `text_ref` →
/// an idempotent `put_typed` + an overwriting `insert`, so the index holds one
/// entry per DISTINCT document.
///
/// The returned [`Dataset`] rows interleave (text, vector) per document, so its
/// [`DatasetId`](kx_dataset::DatasetId) is a pure function of the source text AND
/// the embeddings: pin the [`Embedder`]'s `model_id` + `pooling` and the same
/// corpus re-ingests to a byte-identical `DatasetId` on any machine. `lineage`
/// records the Motes (if any) whose committed output produced the corpus — pass
/// `&[]` for a direct ingest.
///
/// # Errors
/// [`RagError::Embedding`] if a document fails to embed; [`RagError::Store`] on a
/// store failure.
pub fn ingest_corpus(
    store: &dyn DataStore,
    index: &mut dyn RetrievalIndex,
    embedder: &Embedder<'_>,
    docs: &[&str],
    lineage: &[MoteId],
) -> Result<Dataset, RagError> {
    let mut rows = Vec::with_capacity(docs.len().saturating_mul(2));
    for doc in docs {
        let out = embedder.embed(doc)?;
        let vec_bytes = encode_vector_le(&out.vector);
        let text_ref = store.put_typed(doc.as_bytes(), ContentSchema::Text)?;
        let vec_ref = store.put_typed(&vec_bytes, ContentSchema::Vector { dim: out.dim })?;
        index.insert(text_ref.content_ref, out.vector);
        rows.push(text_ref);
        rows.push(vec_ref);
    }
    Ok(Dataset::new(rows, lineage.to_vec()))
}

/// Embed a query and retrieve the top-`k` nearest documents from the index.
///
/// Returns the SN-8-safe committed-fact ref ([`retrieval_result_ref`] — the
/// ordered content refs, **scores excluded**) AND the raw [`Hit`]s (scores
/// included, for DISPLAY / ranking only — never on a commit path). For a fixed
/// index state + query + pooling the `fact_ref` is stable (the exact `InMemory`
/// backend is deterministic), so a downstream Mote that consumes it has a
/// reproducible identity.
///
/// # Errors
/// [`RagError::Embedding`] if the query fails to embed.
pub fn query_corpus(
    index: &dyn RetrievalIndex,
    embedder: &Embedder<'_>,
    query: &str,
    k: usize,
) -> Result<(ContentRef, Vec<Hit>), RagError> {
    let out = embedder.embed(query)?;
    let hits = index.query(&out.vector, k);
    let fact_ref = retrieval_result_ref(&hits);
    Ok((fact_ref, hits))
}
