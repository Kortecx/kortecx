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
use kx_context_assembler::{render_rerank_prompt, rerank_output_cap};
use kx_dataset::{
    mmr_rerank, rrf_fuse, ContentSchema, DataError, DataStore, Dataset, Hit, LexicalIndex,
    RetrievalIndex, MMR_LAMBDA_BP, RRF_C,
};
use kx_grammar::{GrammarSpec, PermutationSpec};
use kx_inference::{
    EmbeddingBackend, EmbeddingOutput, EmbeddingPooling, Grammar, InferenceBackend, InferenceError,
    InferenceInput, InferenceParams,
};
use kx_mote::{ModelId, MoteId};
use kx_toolcall::parse_permutation;
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

/// Like [`ingest_corpus`] but ALSO populates a [`LexicalIndex`] (BM25) so the corpus
/// supports HYBRID (keyword + dense) retrieval (RC4c). For each document the dense
/// vector goes to `index` and the raw text goes to `lexical` under the SAME content
/// ref, so the two legs fuse by ref ([`query_corpus_hybrid`]). Mirrors the live serve
/// `HostDatasetView` ingest, which builds both sidecars.
///
/// # Errors
/// [`RagError::Embedding`] if a document fails to embed; [`RagError::Store`] on a
/// store failure.
pub fn ingest_corpus_hybrid(
    store: &dyn DataStore,
    index: &mut dyn RetrievalIndex,
    lexical: &mut dyn LexicalIndex,
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
        lexical.insert(text_ref.content_ref, doc);
        rows.push(text_ref);
        rows.push(vec_ref);
    }
    Ok(Dataset::new(rows, lineage.to_vec()))
}

/// Width multiplier for the per-leg candidate pool before fusion (mirrors the gateway
/// `HostDatasetView` `N = (k*4).clamp(k, 256)`).
const HYBRID_POOL_MULT: usize = 4;
/// The hard candidate-pool ceiling (matches the gateway).
const HYBRID_POOL_MAX: usize = 256;

/// Embed a query and retrieve the top-`k` via HYBRID search (RC4c): dense (vector) +
/// sparse (BM25) candidate pools → Reciprocal-Rank-Fusion (`RRF_C`) → optional MMR
/// diversity rerank (`MMR_LAMBDA_BP`) → top-`k`. Mirrors the live serve
/// `HostDatasetView::query` hybrid path so an authored RAG workflow gets the same
/// retrieval quality.
///
/// Returns the SN-8-safe committed-fact ref ([`retrieval_result_ref`] — the ordered
/// content refs, **scores excluded**) AND the fused [`Hit`]s (scores for DISPLAY /
/// the optional LLM rerank only — never on a commit path).
///
/// # Errors
/// [`RagError::Embedding`] if the query fails to embed.
pub fn query_corpus_hybrid(
    index: &dyn RetrievalIndex,
    lexical: &dyn LexicalIndex,
    embedder: &Embedder<'_>,
    query: &str,
    k: usize,
    rerank: bool,
) -> Result<(ContentRef, Vec<Hit>), RagError> {
    let out = embedder.embed(query)?;
    let pool = k.saturating_mul(HYBRID_POOL_MULT).clamp(k, HYBRID_POOL_MAX);
    let dense = index.query(&out.vector, pool);
    let sparse = lexical.query(query, pool);
    let fused = rrf_fuse(&dense, &sparse, RRF_C, pool);
    let ranked = if rerank {
        #[allow(clippy::cast_precision_loss)] // bp ∈ [0, 10_000] ⇒ exact in f32
        let lambda = (MMR_LAMBDA_BP as f32) / 10_000.0;
        mmr_rerank(&fused, |id| index.vector_of(id), lambda, k)
    } else {
        fused.into_iter().take(k).collect()
    };
    let fact_ref = retrieval_result_ref(&ranked);
    Ok((fact_ref, ranked))
}

/// LLM listwise rerank (RC4c): ask `backend` to reorder `hits` (whose resolved
/// `texts[i]` corresponds to `hits[i]`) by relevance to `query`, constrained to a
/// permutation of `[0, n)` via the off-digest grammar carrier
/// ([`GrammarSpec::Permutation`]).
///
/// **FAIL-CLOSED.** Any non-permutation output, a carrier/dispatch error, or
/// mismatched inputs keeps the INPUT (RRF/MMR) order — a rerank can never reorder
/// into garbage (SN-8: the model proposes an order, the fail-closed
/// [`parse_permutation`] enforces exact validity).
///
/// `rerank_hits` is a one-shot, NON-memoized, NON-Mote dispatch (the embedding-class
/// precedent — it produces no `MoteId`), so it constructs [`InferenceParams`]
/// directly rather than via `inference_params_from_mote`; the D50 memoizer
/// source-of-truth invariant is N/A here (nothing memoizes this call).
#[must_use]
pub fn rerank_hits(
    backend: &dyn InferenceBackend,
    model_id: &ModelId,
    warrant: &WarrantSpec,
    query: &str,
    hits: &[Hit],
    texts: &[String],
) -> Vec<Hit> {
    let n = hits.len();
    // Nothing to reorder (0/1 candidate) or a caller bug (mismatched lengths) ⇒ identity.
    if n < 2 || texts.len() != n {
        return hits.to_vec();
    }
    let Ok(carrier) =
        GrammarSpec::Permutation(PermutationSpec::new(u32::try_from(n).unwrap_or(u32::MAX)))
            .to_raw()
    else {
        return hits.to_vec();
    };
    // The carrier requests a permutation. The Ollama backend renders it as a strict
    // whole-response `format`; the llama.cpp backend degrades it to the fail-closed
    // parser (its char-level grammar sampler crashes on a digit-array constraint with
    // some tokenizers — T-RERANK-GBNF-CRASH). Either way the model proposes an order
    // and `parse_permutation` enforces validity.
    let params = InferenceParams {
        grammar: Some(Grammar::new(carrier)),
        temperature_bps: 0, // greedy — the permutation is a decision, not creative output
        max_output_tokens: rerank_output_cap(n),
        ..InferenceParams::default()
    };
    let input = InferenceInput::text(render_rerank_prompt(query, texts));
    // A dispatch failure ⇒ keep the upstream (RRF/MMR) order (fail-closed).
    let Ok(out) = backend.dispatch(model_id, &input, &params, warrant) else {
        return hits.to_vec();
    };
    let text = String::from_utf8_lossy(&out.bytes);
    match parse_permutation(&text, n) {
        Some(order) => order.into_iter().map(|i| hits[i]).collect(),
        None => hits.to_vec(), // not a valid permutation ⇒ fail-closed to upstream order
    }
}
