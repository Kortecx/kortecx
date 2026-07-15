//! The Datasets data-plane read/write seam (the T3.7 `ListDatasets` /
//! `IngestDocuments` / `QueryDataset` path).
//!
//! Spoken entirely in gateway-core's OWN wire vocabulary (`&[u8]` / `Vec<f32>` /
//! `String`) — no `kx-dataset` / `kx-dataset-hnsw` type crosses the seam, so
//! gateway-core gains NO dataset crate dependency and stays off the writer wall.
//! The host (`kx-gateway`, behind the opt-in `hnsw` feature) implements
//! [`DatasetView`] over `kx-dataset-hnsw` + the durable content store.
//!
//! # Boundaries (load-bearing)
//!
//! - **SN-8.** [`DatasetHitEntry::score`] is DISPLAY-ONLY — it never enters a
//!   committed fact or a `MoteId`; only the ordered content-ref SET is the
//!   durable retrieval result. A `None` seam ⇒ the three RPCs return
//!   `unimplemented` (old-gateway forward-compat degrade).
//! - **Server-derived identity.** The host derives each document's id from its
//!   content (content-addressed); an advisory client `doc_id` is never identity.
//! - **Embedding is pluggable.** A document/query may carry a client-computed
//!   vector (the FFI-free path) or rely on a server embedder (the `inference`
//!   path); the seam carries the optional vector and lets the host decide.

use kx_proto::proto;
use tonic::Status;

/// The retrieval strategy a query requests — gateway-core's own vocab (the host
/// maps it to dense-only vs BM25+dense RRF fusion). `Default` ⇒ the host's
/// operator-configured default; `Hybrid` silently falls back to dense when there
/// is no `query_text` (the FFI-free client-vector path). RC4a.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum RetrievalMode {
    /// Use the host's operator-configured default.
    #[default]
    Default,
    /// Dense (embedding) ANN only — reproducible.
    Dense,
    /// BM25 + dense, RRF-fused (+ operator MMR rerank).
    Hybrid,
}

/// Map the wire enum discriminant to gateway-core's [`RetrievalMode`]. Unknown /
/// `UNSPECIFIED` ⇒ `Default` (the host default), never an error.
pub(crate) fn retrieval_mode_from_proto(v: i32) -> RetrievalMode {
    match proto::RetrievalMode::try_from(v) {
        Ok(proto::RetrievalMode::Dense) => RetrievalMode::Dense,
        Ok(proto::RetrievalMode::Hybrid) => RetrievalMode::Hybrid,
        _ => RetrievalMode::Default,
    }
}

/// One dataset in a [`DatasetView::list_datasets`] enumeration.
#[derive(Clone, Debug)]
pub struct DatasetSummaryEntry {
    /// The dataset's stable host name (server-validated, not a hash).
    pub dataset_id: String,
    /// The advisory human handle (today == `dataset_id`).
    pub name: String,
    /// The distinct PARENT document count (RC4a: not chunks — see `chunk_count`).
    pub doc_count: u64,
    /// The embedding dimension (0 until the first insert fixes it).
    pub dim: u32,
    /// The unix-ms create time (display only; off every hash).
    pub created_ms: i64,
    /// RC4a: ingested under the chunking pipeline (hits are passages, not docs).
    pub chunked: bool,
    /// RC4a: hex of the index fingerprint (embed model/pooling/dim/chunk/tokenizer);
    /// empty for a legacy/unstamped dataset.
    pub embed_model_fingerprint: String,
    /// RC4a: the on-disk retrieval-index schema version (cold-index / compat guard).
    pub index_version: u32,
    /// RC4a: distinct retrievable chunks (== `doc_count` for legacy/un-chunked).
    pub chunk_count: u64,
}

/// One document to ingest: content bytes ALWAYS, plus an OPTIONAL client-computed
/// embedding. `embedding == None` requires a server embedder (the `inference`
/// path); `Some` is the FFI-free client-vector path. Borrows from the request so
/// the handler does not copy the payload before the host dedups it.
#[derive(Clone, Copy, Debug)]
pub struct IngestDoc<'a> {
    /// The retrievable payload (the host content-addresses this for the id).
    pub content: &'a [u8],
    /// The client-computed vector, or `None` to ask the host to embed `content`.
    pub embedding: Option<&'a [f32]>,
}

/// The outcome of a [`DatasetView::ingest`] call.
#[derive(Clone, Debug)]
pub struct IngestOutcome {
    /// The validated dataset name.
    pub dataset_id: String,
    /// The total distinct docs in the dataset AFTER this ingest.
    pub doc_count: u64,
    /// The NEW distinct docs added by this call (post content-addressed dedup).
    pub inserted: u64,
    /// The dataset's embedding dimension.
    pub dim: u32,
}

/// One retrieval hit. `score` is DISPLAY-ONLY (SN-8).
#[derive(Clone, Debug)]
pub struct DatasetHitEntry {
    /// The 32-byte content-addressed id of the retrieved CHUNK (RC4a).
    pub content_ref: [u8; 32],
    /// The chunk payload bytes (so a UI can show the snippet).
    pub content: Vec<u8>,
    /// The similarity score — DISPLAY-ONLY; NEVER an identity input.
    pub score: f32,
    /// RC4a: the 32-byte id of the PARENT document (== `content_ref` for
    /// legacy/un-chunked corpora).
    pub parent_ref: [u8; 32],
    /// RC4a: 0-based ordinal of this chunk within its parent (display/ordering).
    pub chunk_index: u32,
    /// RC4a: total chunks in the parent (display).
    pub chunk_count: u32,
}

/// A failure from the [`DatasetView`] seam, mapped to honest gRPC codes by the
/// service handler (`not_found` / `invalid_argument` / `failed_precondition` /
/// `internal`).
#[derive(Debug)]
pub enum DatasetError {
    /// The named dataset does not exist (a query/embed against it) ⇒ `not_found`.
    NotFound,
    /// A vector's length disagrees with the dataset's fixed dimension ⇒
    /// `invalid_argument`.
    DimMismatch(String),
    /// A server-embed was requested (a vector-less doc / `query_text`) but no
    /// embedder is wired ⇒ `failed_precondition`.
    EmbedderUnavailable,
    /// RC4a: the live embedder's fingerprint disagrees with the one the dataset was
    /// indexed under (a different embed model / pooling / chunk params) — querying
    /// would compare incompatible vector spaces, so the host refuses rather than
    /// silently mis-rank ⇒ `failed_precondition`. Re-ingest to rebuild.
    StaleIndex(String),
    /// A malformed request (empty content, bad dataset name, non-UTF-8 text for a
    /// server-embed) ⇒ `invalid_argument`.
    InvalidArgument(String),
    /// A backend failure (store / persist / poisoned lock) ⇒ `internal`.
    Internal(String),
}

/// The dataset read/write seam. The host implements it over `kx-dataset-hnsw` +
/// the content store (behind the `hnsw` feature). A `None` seam on the service ⇒
/// the three dataset RPCs return `unimplemented`.
pub trait DatasetView: Send + Sync {
    /// Every dataset, in deterministic (name) order.
    fn list_datasets(&self) -> Vec<DatasetSummaryEntry>;

    /// The LIVE embed scope — a stable tag over the current embedder + chunk config
    /// (the same inputs as a dataset's `embed_model_fingerprint`), or `None` when the
    /// host has no embedder to describe (the client-vector path).
    ///
    /// Names the space a server-embedded ingest would land in, so a caller can key a
    /// derived dataset name on it (`app_dataset_scoped_name`) and rotate that name when
    /// the operator swaps models — the escape from an unqueryable stale index, given
    /// ingest refuses to MIX embed spaces and no RPC can drop a dataset. Advisory +
    /// off-identity: never journaled, never a `MoteId` or digest input.
    ///
    /// Defaulted to `None` so a view that never server-embeds (a stub, a client-vector
    /// host) opts out without a change.
    fn embed_scope_tag(&self) -> Option<String> {
        None
    }

    /// Ingest `docs` into `dataset` (created on first ingest). A doc carrying a
    /// vector uses the client-vector path; a vector-less doc needs an embedder.
    ///
    /// # Errors
    /// [`DatasetError`] on a bad name/content, a dim mismatch, a missing embedder,
    /// or a backend failure.
    fn ingest(&self, dataset: &str, docs: &[IngestDoc<'_>]) -> Result<IngestOutcome, DatasetError>;

    /// Query `dataset` for the top-`k` nearest chunks. `query_embedding`
    /// (`Some`) is the client-vector path; `None` falls back to embedding
    /// `query_text` (needs an embedder). `mode` selects dense vs hybrid (RC4a;
    /// hybrid needs `query_text`). `rerank` (RC4c) overrides the operator's MMR
    /// diversity-rerank default per query (`None` ⇒ the server default). Ordered
    /// score-desc, ascending-ref.
    ///
    /// # Errors
    /// [`DatasetError::NotFound`] for an unknown dataset; otherwise as `ingest`.
    fn query(
        &self,
        dataset: &str,
        query_embedding: Option<&[f32]>,
        query_text: &str,
        k: usize,
        mode: RetrievalMode,
        rerank: Option<bool>,
    ) -> Result<Vec<DatasetHitEntry>, DatasetError>;
}

/// Map a [`DatasetError`] to its honest gRPC [`Status`].
pub(crate) fn dataset_status(err: DatasetError) -> Status {
    match err {
        DatasetError::NotFound => Status::not_found("dataset not found"),
        DatasetError::DimMismatch(detail) | DatasetError::InvalidArgument(detail) => {
            Status::invalid_argument(detail)
        }
        DatasetError::EmbedderUnavailable => Status::failed_precondition(
            "no embedding model wired: provide vectors client-side, or run \
             `kx serve --features inference` with a model",
        ),
        DatasetError::StaleIndex(detail) => Status::failed_precondition(detail),
        DatasetError::Internal(detail) => Status::internal(detail),
    }
}

/// Map a gateway-core dataset summary into the wire type.
pub(crate) fn dataset_summary_to_proto(d: DatasetSummaryEntry) -> proto::DatasetSummary {
    proto::DatasetSummary {
        dataset_id: d.dataset_id,
        name: d.name,
        doc_count: d.doc_count,
        dim: d.dim,
        created_ms: d.created_ms,
        chunked: d.chunked,
        embed_model_fingerprint: d.embed_model_fingerprint,
        index_version: d.index_version,
        chunk_count: d.chunk_count,
    }
}

/// Map a gateway-core retrieval hit into the wire type.
pub(crate) fn dataset_hit_to_proto(h: DatasetHitEntry) -> proto::DatasetHit {
    proto::DatasetHit {
        content_ref: h.content_ref.to_vec(),
        content: h.content,
        score: h.score,
        parent_ref: h.parent_ref.to_vec(),
        chunk_index: h.chunk_index,
        chunk_count: h.chunk_count,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tonic::Code;

    #[test]
    fn dataset_status_maps_each_error_to_its_honest_code() {
        assert_eq!(
            dataset_status(DatasetError::NotFound).code(),
            Code::NotFound
        );
        assert_eq!(
            dataset_status(DatasetError::DimMismatch("d".into())).code(),
            Code::InvalidArgument
        );
        assert_eq!(
            dataset_status(DatasetError::InvalidArgument("a".into())).code(),
            Code::InvalidArgument
        );
        assert_eq!(
            dataset_status(DatasetError::EmbedderUnavailable).code(),
            Code::FailedPrecondition
        );
        assert_eq!(
            dataset_status(DatasetError::StaleIndex("stale".into())).code(),
            Code::FailedPrecondition
        );
        assert_eq!(
            dataset_status(DatasetError::Internal("i".into())).code(),
            Code::Internal
        );
    }

    #[test]
    fn retrieval_mode_from_proto_maps_known_and_defaults_unknown() {
        assert_eq!(
            retrieval_mode_from_proto(proto::RetrievalMode::Dense as i32),
            RetrievalMode::Dense
        );
        assert_eq!(
            retrieval_mode_from_proto(proto::RetrievalMode::Hybrid as i32),
            RetrievalMode::Hybrid
        );
        // UNSPECIFIED and any unknown discriminant ⇒ the host default.
        assert_eq!(retrieval_mode_from_proto(0), RetrievalMode::Default);
        assert_eq!(retrieval_mode_from_proto(99), RetrievalMode::Default);
    }
}
