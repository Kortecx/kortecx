//! T3.7 host-side Datasets data-plane (the `kx-gateway-core` [`DatasetView`] seam).
//!
//! gateway-core stays off `kx-dataset` / `kx-dataset-hnsw` (the dependency wall),
//! so the concrete, ANN-backed implementation lives HERE, behind the opt-in
//! `hnsw` feature. The whole module is `#![cfg(feature = "hnsw")]`.
//!
//! # Storage model (durable, off-journal)
//!
//! A dataset is a named retrieval corpus. Unlike a run (whose facts are the
//! journal's), the direct `IngestDocuments` RPC is OFF-JOURNAL — so the dataset
//! store is PRIMARY durable data. It is held in a crash-safe SQLite db
//! (`<catalog_dir>/datasets/datasets.db`, WAL + `synchronous = FULL`), the same
//! durable-side-store posture as the membership ledger (`members.db`). Each
//! document row is `(dataset, ref, content, vector)`: `ref = ContentRef::of(content)`
//! (server-derived, content-addressed — SN-8), `vector` the canonical little-endian
//! f32 form. The HNSW graph ([`HnswRetrievalIndex`]) is a DERIVED in-memory
//! accelerator, rebuilt from the rows on open (so a graph-format break is recovered
//! by rebuild, never a migration — D40). The dataset NAME is a SQLite key, never a
//! filename, so there is no path-traversal surface.
//!
//! # Embedding (pluggable — the HuggingFace seam)
//!
//! A document/query may carry a client-computed vector (the FFI-free path, works
//! under `hnsw` alone) or rely on a server embedder (the `inference` path, an
//! `EmbeddingBackend`). This decoupling is the seam an external embedder (e.g.
//! HuggingFace transformers in the Py/TS SDK) plugs into with no runtime change.
//!
//! # SN-8
//!
//! The similarity score is DISPLAY-ONLY: the retrieval result the seam returns is
//! the ordered content-ref SET; a downstream consumer matches by EXACT hash. The
//! approximate, build-order-sensitive HNSW order never reaches a `MoteId`.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use kx_chunk::{chunk, ChunkParams};
use kx_content::ContentRef;
use kx_dataset::{mmr_rerank, rrf_fuse, Hit, LexicalIndex, RetrievalIndex, MMR_LAMBDA_BP, RRF_C};
use kx_dataset_bm25::{Bm25Index, Bm25Params};
use kx_dataset_hnsw::HnswRetrievalIndex;
// The index-fingerprint axes are used only on the server-embed path (the
// client-vector path owns its own vector space, so it has no embed fingerprint).
#[cfg(feature = "serve-engine")]
use kx_chunk::CHUNKER_VERSION;
#[cfg(feature = "serve-engine")]
use kx_dataset::index_fingerprint;
#[cfg(feature = "serve-engine")]
use kx_dataset_bm25::TOKENIZER_VERSION;
use kx_gateway_core::{
    score_to_bp, DatasetError, DatasetHitEntry, DatasetSummaryEntry, DatasetView,
    FuzzyDiscoveryView, FuzzyHitEntry, IngestDoc, IngestOutcome, RetrievalMode,
};
use rusqlite::{params, Connection};

use crate::error::GatewayError;

/// Operator-config retrieval tuning (all from `KX_SERVE_RAG_*` env — never client-
/// chosen, SN-8). Threaded into [`HostDatasetView`] at open. RC4a.
#[derive(Clone, Copy, Debug)]
pub struct RagConfig {
    /// Max chunk size in Unicode chars (server-embed ingest).
    pub chunk_max_chars: usize,
    /// Chunk overlap in chars.
    pub chunk_overlap_chars: usize,
    /// Per-document chunk cap (0 ⇒ unbounded; still bounded by content-max-bytes).
    pub max_chunks_per_doc: usize,
    /// The retrieval mode applied when a client sends `UNSPECIFIED`.
    pub default_mode: RetrievalMode,
    /// The RRF fusion constant.
    pub rrf_k: u32,
    /// The MMR relevance/diversity trade-off, in basis points.
    pub mmr_lambda_bp: u32,
    /// Whether MMR diversity rerank runs.
    pub rerank: bool,
    /// Whether the BM25 tokenizer drops the fixed English stoplist.
    pub stopwords: bool,
}

impl Default for RagConfig {
    fn default() -> Self {
        Self {
            chunk_max_chars: kx_chunk::DEFAULT_MAX_CHARS,
            chunk_overlap_chars: kx_chunk::DEFAULT_OVERLAP_CHARS,
            max_chunks_per_doc: 0,
            default_mode: RetrievalMode::Hybrid,
            rrf_k: RRF_C,
            mmr_lambda_bp: MMR_LAMBDA_BP,
            rerank: true,
            stopwords: false,
        }
    }
}

impl RagConfig {
    /// The BM25 parameters derived from this config (stopwords only; k1/b default).
    fn bm25_params(self) -> Bm25Params {
        Bm25Params {
            stopwords: self.stopwords,
            ..Bm25Params::default()
        }
    }
}

/// One resolved chunk ready to index: its content-addressed ref + bytes + vector,
/// plus provenance back to the parent document.
struct ChunkRow {
    chunk_ref: ContentRef,
    content: Vec<u8>,
    vector: Vec<f32>,
    parent_ref: ContentRef,
    chunk_index: u32,
}

/// Per-chunk provenance (off every hash — display/grouping only).
#[derive(Clone, Copy)]
struct ChunkProv {
    parent_ref: ContentRef,
    chunk_index: u32,
}

/// One raw search result from the shared search core; the wire mappers turn it
/// into a `DatasetHitEntry` (with content) or a `FuzzyHitEntry` (refs + score only).
struct SearchHit {
    chunk_ref: ContentRef,
    content: Vec<u8>,
    score: f32,
    parent_ref: ContentRef,
    chunk_index: u32,
    chunk_count: u32,
}

/// The server-side cap on a query's top-`k` (kept ≤ the HNSW default `ef_search`
/// so the ANN candidate list always covers the requested neighbours). An untrusted
/// `k` cannot force an unbounded scan.
const MAX_K: usize = 64;

/// The maximum dataset-name length (a SQLite key, so this is hygiene, not a
/// path-traversal bound — there is no per-dataset file).
const MAX_NAME_LEN: usize = 128;

/// The current on-disk retrieval-index schema version (RC4a: chunk provenance +
/// the embed fingerprint). A legacy (pre-RC4a) dataset rebuilds at version 1.
const INDEX_VERSION: u32 = 1;

/// The durable schema (idempotent — fresh dbs). `documents.ref`/`vector` are BLOBs;
/// the HNSW + BM25 indices are rebuilt from these rows on open. RC4a adds chunk
/// provenance (`parent_ref`/`chunk_index`) + per-dataset compat metadata
/// (`chunked`/`embed_fingerprint`/`index_version`); [`migrate_schema`] adds the same
/// columns to a pre-RC4a db.
const SCHEMA: &str = "\
CREATE TABLE IF NOT EXISTS datasets (
    name              TEXT PRIMARY KEY,
    created_ms        INTEGER NOT NULL,
    dim               INTEGER NOT NULL DEFAULT 0,
    chunked           INTEGER NOT NULL DEFAULT 0,
    embed_fingerprint TEXT NOT NULL DEFAULT '',
    index_version     INTEGER NOT NULL DEFAULT 1
);
CREATE TABLE IF NOT EXISTS documents (
    dataset     TEXT NOT NULL,
    ref         BLOB NOT NULL,
    content     BLOB NOT NULL,
    vector      BLOB NOT NULL,
    parent_ref  BLOB,
    chunk_index INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (dataset, ref)
);";

/// Bring a pre-RC4a `datasets.db` up to the current schema by adding the new columns.
/// SQLite has no `ADD COLUMN IF NOT EXISTS`, so a duplicate-column error means the
/// column already exists and is ignored. PURELY ADDITIVE — never a reshape, so a new
/// binary reads an old db (degraded back-compat: a legacy whole-doc row is a
/// single chunk with a NULL parent_ref) and an old binary still reads the base
/// columns of a new db.
fn migrate_schema(conn: &Connection) {
    for stmt in [
        "ALTER TABLE datasets ADD COLUMN chunked INTEGER NOT NULL DEFAULT 0",
        "ALTER TABLE datasets ADD COLUMN embed_fingerprint TEXT NOT NULL DEFAULT ''",
        "ALTER TABLE datasets ADD COLUMN index_version INTEGER NOT NULL DEFAULT 1",
        "ALTER TABLE documents ADD COLUMN parent_ref BLOB",
        "ALTER TABLE documents ADD COLUMN chunk_index INTEGER NOT NULL DEFAULT 0",
    ] {
        let _ = conn.execute(stmt, []); // duplicate-column ⇒ already migrated (ignore)
    }
}

/// Canonical little-endian f32 encoding of a vector (byte-identical to
/// `kx_model_harness::rag::encode_vector_le` — the reproducible content-addressed
/// form). Kept local so the host need not depend on the eval harness.
fn encode_vector_le(vector: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(vector.len() * 4);
    for v in vector {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// Decode the canonical little-endian f32 form (the inverse of [`encode_vector_le`]).
/// A trailing partial chunk (a corrupt row) is dropped rather than panicking.
fn decode_vector_le(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

/// `true` iff every component is finite (no `NaN` / ±inf). An untrusted client
/// vector must pass this before it touches the cosine ANN index — a `NaN`/inf
/// component would poison the similarity ordering (and is meaningless as an
/// embedding), so it is rejected as `invalid_argument` rather than indexed.
fn all_finite(v: &[f32]) -> bool {
    v.iter().all(|x| x.is_finite())
}

/// Wall-clock unix-ms (display only, off every hash). A pre-epoch clock ⇒ 0.
fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

/// Validate a request-supplied dataset name. A SQLite key (not a filename), so this
/// is hygiene: non-empty, bounded, an ASCII `[A-Za-z0-9._-]` allowlist, and never a
/// bare dot run.
fn validate_dataset_name(name: &str) -> Result<(), DatasetError> {
    if name.is_empty() || name.len() > MAX_NAME_LEN {
        return Err(DatasetError::InvalidArgument(format!(
            "dataset name must be 1..={MAX_NAME_LEN} chars"
        )));
    }
    if name == "." || name == ".." {
        return Err(DatasetError::InvalidArgument(
            "invalid dataset name".to_string(),
        ));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        return Err(DatasetError::InvalidArgument(
            "dataset name allows [A-Za-z0-9._-] only".to_string(),
        ));
    }
    Ok(())
}

/// One dataset's in-memory state: the rebuilt-on-open dense (HNSW) + sparse (BM25)
/// indices, the chunk→content map (for hit snippets), and chunk provenance. `dim`
/// is fixed by the first insert. RC4a.
struct DatasetState {
    created_ms: i64,
    dim: Option<u32>,
    /// True once a server-embed (chunked) ingest has run (display/advisory).
    chunked: bool,
    /// Hex of the index fingerprint the corpus was embedded under (empty = unstamped).
    embed_fingerprint: String,
    /// The on-disk index schema version (cold-index / compat guard).
    index_version: u32,
    /// The dense (embedding) ANN index.
    index: HnswRetrievalIndex,
    /// The sparse (BM25 keyword) index — the hybrid leg (RC4a).
    lex: Bm25Index,
    /// chunk_ref → chunk content bytes (hit snippets).
    docs: HashMap<ContentRef, Vec<u8>>,
    /// chunk_ref → provenance (parent doc + ordinal). Off every hash.
    prov: HashMap<ContentRef, ChunkProv>,
    /// parent_ref → number of chunks (for the per-hit `chunk_count`).
    parent_chunks: HashMap<ContentRef, u32>,
}

impl DatasetState {
    fn empty(created_ms: i64, bm25: Bm25Params) -> Self {
        Self {
            created_ms,
            dim: None,
            chunked: false,
            embed_fingerprint: String::new(),
            index_version: INDEX_VERSION,
            index: HnswRetrievalIndex::new(),
            lex: Bm25Index::with_params(bm25),
            docs: HashMap::new(),
            prov: HashMap::new(),
            parent_chunks: HashMap::new(),
        }
    }
}

/// The guarded interior: the durable connection + the in-memory dataset map. One
/// `Mutex` (rather than a per-dataset `RwLock`) keeps `HostDatasetView` trivially
/// `Send + Sync` regardless of the ANN backend's bounds and serializes the
/// off-journal store's writes; the OSS console is not a high-QPS dataset path. A
/// per-dataset read/write split is a flagged scale follow-on.
struct Inner {
    db: Connection,
    datasets: HashMap<String, DatasetState>,
    config: RagConfig,
}

impl Inner {
    /// Apply a batch of pre-resolved `(ref, content, vector)` documents to `dataset`
    /// (created on first ingest): validate the batch dim against the dataset's fixed
    /// dim, content-addressed dedup, durable insert (one transaction), then the
    /// derived index. The dim is validated for the WHOLE batch before any mutation,
    /// so an ingest is all-or-nothing on dimension.
    ///
    /// **Durable-first ordering (load-bearing).** Every SQL row is staged + the
    /// transaction is COMMITTED *before* the in-memory HNSW index / docs map is
    /// touched. A failed `execute`/`commit` rolls the durable write back AND leaves
    /// the derived state untouched — so the SQLite store stays the single source of
    /// truth (never phantom, queryable-but-unpersisted documents that vanish on a
    /// restart-rebuild). The in-memory apply is infallible, so it cannot diverge.
    fn ingest_resolved(
        &mut self,
        dataset: &str,
        rows: Vec<ChunkRow>,
        chunked: bool,
        fingerprint: Option<String>,
    ) -> Result<IngestOutcome, DatasetError> {
        let first = rows
            .first()
            .ok_or_else(|| DatasetError::InvalidArgument("no documents".to_string()))?;
        let batch_dim = u32::try_from(first.vector.len())
            .map_err(|_| DatasetError::InvalidArgument("vector too large".to_string()))?;
        let target_dim = self
            .datasets
            .get(dataset)
            .and_then(|s| s.dim)
            .unwrap_or(batch_dim);
        for r in &rows {
            let vlen = u32::try_from(r.vector.len())
                .map_err(|_| DatasetError::InvalidArgument("vector too large".to_string()))?;
            if vlen != target_dim {
                return Err(DatasetError::DimMismatch(format!(
                    "dataset dim is {target_dim}, got a {vlen}-dim vector"
                )));
            }
        }

        // RC4a staleness guard: refuse to MIX an existing corpus's embed space with a
        // different embed model / chunk config (would compare incompatible vectors).
        if let (Some(existing), Some(new_fp)) = (self.datasets.get(dataset), fingerprint.as_ref()) {
            if !existing.embed_fingerprint.is_empty() && existing.embed_fingerprint != *new_fp {
                return Err(DatasetError::StaleIndex(format!(
                    "dataset '{dataset}' was indexed under a different embed model / chunk \
                     config; create a new dataset or re-ingest to rebuild"
                )));
            }
        }

        // Content-addressed dedup BEFORE any write — against the already-persisted chunks
        // AND within this batch — so the durable INSERTs and the in-memory apply act on
        // the SAME new-chunk set and `inserted` is exact.
        let mut batch_seen: HashSet<ContentRef> = HashSet::new();
        let to_apply: Vec<ChunkRow> = {
            let existing = self.datasets.get(dataset);
            rows.into_iter()
                .filter(|r| {
                    let already = existing.is_some_and(|s| s.docs.contains_key(&r.chunk_ref));
                    !already && batch_seen.insert(r.chunk_ref)
                })
                .collect()
        };

        let created_ms = self
            .datasets
            .get(dataset)
            .map_or_else(now_ms, |s| s.created_ms);

        // DURABLE FIRST: COMMIT every row before touching the derived state.
        let inserted = u64::try_from(to_apply.len()).unwrap_or(u64::MAX);
        self.persist_batch(
            dataset,
            created_ms,
            target_dim,
            chunked,
            fingerprint.as_deref(),
            &to_apply,
        )?;
        // COMMITTED — only now apply to the derived in-memory state (infallible).
        let doc_count = self.apply_to_state(
            dataset,
            created_ms,
            target_dim,
            chunked,
            fingerprint,
            to_apply,
        );

        Ok(IngestOutcome {
            dataset_id: dataset.to_string(),
            doc_count,
            inserted,
            dim: target_dim,
        })
    }

    /// Stage + COMMIT a batch's durable rows in one transaction (the durable-first half
    /// of `ingest_resolved`). The derived in-memory state is untouched until this
    /// returns `Ok`, so a failed write never leaves a phantom queryable document.
    fn persist_batch(
        &mut self,
        dataset: &str,
        created_ms: i64,
        target_dim: u32,
        chunked: bool,
        fingerprint: Option<&str>,
        to_apply: &[ChunkRow],
    ) -> Result<(), DatasetError> {
        let txn = self
            .db
            .transaction()
            .map_err(|e| DatasetError::Internal(e.to_string()))?;
        txn.execute(
            "INSERT OR IGNORE INTO datasets(name, created_ms, dim, chunked, index_version) \
             VALUES(?, ?, ?, ?, ?)",
            params![
                dataset,
                created_ms,
                target_dim,
                i64::from(chunked),
                INDEX_VERSION
            ],
        )
        .map_err(|e| DatasetError::Internal(e.to_string()))?;
        // Fix the dim on the first real insert (the create above may have written a 0).
        txn.execute(
            "UPDATE datasets SET dim = ? WHERE name = ? AND dim = 0",
            params![target_dim, dataset],
        )
        .map_err(|e| DatasetError::Internal(e.to_string()))?;
        if chunked {
            txn.execute(
                "UPDATE datasets SET chunked = 1 WHERE name = ?",
                params![dataset],
            )
            .map_err(|e| DatasetError::Internal(e.to_string()))?;
        }
        // Stamp the fingerprint on the FIRST server-embed ingest (only when unset).
        if let Some(fp) = fingerprint {
            txn.execute(
                "UPDATE datasets SET embed_fingerprint = ? WHERE name = ? AND embed_fingerprint = ''",
                params![fp, dataset],
            )
            .map_err(|e| DatasetError::Internal(e.to_string()))?;
        }
        for r in to_apply {
            txn.execute(
                "INSERT OR IGNORE INTO documents(dataset, ref, content, vector, parent_ref, chunk_index) \
                 VALUES(?, ?, ?, ?, ?, ?)",
                params![
                    dataset,
                    &r.chunk_ref.as_bytes()[..],
                    r.content.as_slice(),
                    encode_vector_le(&r.vector).as_slice(),
                    &r.parent_ref.as_bytes()[..],
                    r.chunk_index,
                ],
            )
            .map_err(|e| DatasetError::Internal(e.to_string()))?;
        }
        txn.commit()
            .map_err(|e| DatasetError::Internal(e.to_string()))
    }

    /// Apply a committed batch to the in-memory dense + sparse indices, the
    /// content + provenance maps (infallible). Returns the dataset's distinct
    /// PARENT-document count.
    fn apply_to_state(
        &mut self,
        dataset: &str,
        created_ms: i64,
        target_dim: u32,
        chunked: bool,
        fingerprint: Option<String>,
        to_apply: Vec<ChunkRow>,
    ) -> u64 {
        let bm25 = self.config.bm25_params();
        let state = self
            .datasets
            .entry(dataset.to_string())
            .or_insert_with(|| DatasetState::empty(created_ms, bm25));
        state.dim = Some(target_dim);
        if chunked {
            state.chunked = true;
        }
        if let Some(fp) = fingerprint {
            if state.embed_fingerprint.is_empty() {
                state.embed_fingerprint = fp;
            }
        }
        for r in to_apply {
            state.index.insert(r.chunk_ref, r.vector);
            if let Ok(text) = std::str::from_utf8(&r.content) {
                state.lex.insert(r.chunk_ref, text);
            }
            state.docs.insert(r.chunk_ref, r.content);
            state.prov.insert(
                r.chunk_ref,
                ChunkProv {
                    parent_ref: r.parent_ref,
                    chunk_index: r.chunk_index,
                },
            );
            *state.parent_chunks.entry(r.parent_ref).or_insert(0) += 1;
        }
        u64::try_from(state.parent_chunks.len()).unwrap_or(u64::MAX)
    }
}

/// The bundled server embedder (the `serve-engine` path — routes to the in-process
/// llama.cpp backend OR an Ollama daemon via the host `RoutingBackend`): a
/// `kx_inference::EmbeddingBackend` + the model route + warrant + pooling that travel
/// together for every embed call.
#[cfg(feature = "serve-engine")]
pub struct HostEmbedder {
    backend: std::sync::Arc<dyn kx_inference::EmbeddingBackend>,
    model_id: kx_mote::ModelId,
    warrant: kx_warrant::WarrantSpec,
    pooling: kx_inference::EmbeddingPooling,
}

#[cfg(feature = "serve-engine")]
impl HostEmbedder {
    /// Bind a backend + model route + warrant (mean pooling — the HF default).
    #[must_use]
    pub fn new(
        backend: std::sync::Arc<dyn kx_inference::EmbeddingBackend>,
        model_id: kx_mote::ModelId,
        warrant: kx_warrant::WarrantSpec,
    ) -> Self {
        Self {
            backend,
            model_id,
            warrant,
            pooling: kx_inference::EmbeddingPooling::Mean,
        }
    }

    // `pub(crate)` so `HostMemoryView` (RC5a) reuses the SAME embedder type to embed
    // memory content/queries — one embedder abstraction, two data-plane views.
    pub(crate) fn embed(&self, text: &str) -> Result<Vec<f32>, DatasetError> {
        let out = self
            .backend
            .dispatch_embedding(&self.model_id, text, self.pooling, &self.warrant)
            .map_err(|e| DatasetError::Internal(format!("embedding: {e}")))?;
        Ok(out.vector)
    }

    /// The embed model id string — a fingerprint axis (RC4a).
    pub(crate) fn model_id_string(&self) -> String {
        self.model_id.0.clone()
    }

    /// The pooling strategy as a stable u8 tag — a fingerprint axis (RC4a).
    pub(crate) fn pooling_tag(&self) -> u8 {
        match self.pooling {
            kx_inference::EmbeddingPooling::Mean => 0,
            kx_inference::EmbeddingPooling::Cls => 1,
            kx_inference::EmbeddingPooling::Last => 2,
        }
    }
}

/// A [`DatasetView`] over a durable SQLite store + a rebuilt-on-open HNSW ANN index.
/// VIEW + INGEST (no journal write). Optionally carries a server `HostEmbedder`
/// (the `serve-engine` path); without it, only the client-vector path is available.
pub struct HostDatasetView {
    inner: Mutex<Inner>,
    config: RagConfig,
    #[cfg(feature = "serve-engine")]
    embedder: Option<HostEmbedder>,
}

impl HostDatasetView {
    /// Open (or create) the dataset store under `dir`, rebuilding every dataset's
    /// dense (HNSW) + sparse (BM25) indices SYNCHRONOUSLY from its durable rows
    /// before returning — no dataset is queryable before both arms are warm
    /// (closes the cold-index race). Uses the default [`RagConfig`]; override with
    /// [`with_rag_config`](Self::with_rag_config).
    ///
    /// # Errors
    /// [`GatewayError::Catalog`] if the directory / db / schema cannot be opened.
    pub fn open(dir: &Path) -> Result<Self, GatewayError> {
        std::fs::create_dir_all(dir)
            .map_err(|e| GatewayError::Catalog(format!("datasets dir: {e}")))?;
        let db_path = dir.join("datasets.db");
        let conn = Connection::open(&db_path)
            .map_err(|e| GatewayError::Catalog(format!("datasets db: {e}")))?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = FULL;
             PRAGMA foreign_keys = ON;",
        )
        .map_err(|e| GatewayError::Catalog(format!("datasets pragma: {e}")))?;
        conn.execute_batch(SCHEMA)
            .map_err(|e| GatewayError::Catalog(format!("datasets schema: {e}")))?;
        // Purely-additive migration of a pre-RC4a db (idempotent; never a reshape).
        migrate_schema(&conn);
        let config = RagConfig::default();
        let datasets = rebuild_state(&conn, config)
            .map_err(|e| GatewayError::Catalog(format!("datasets rebuild: {e}")))?;
        Ok(Self {
            inner: Mutex::new(Inner {
                db: conn,
                datasets,
                config,
            }),
            config,
            #[cfg(feature = "serve-engine")]
            embedder: None,
        })
    }

    /// Override the operator retrieval config (chunking + hybrid knobs). Re-builds
    /// the in-memory indices under the new BM25 params so a config change takes
    /// effect on open (the durable rows are unchanged).
    ///
    /// # Errors
    /// [`GatewayError::Catalog`] if the rebuild fails (a poisoned lock).
    #[must_use]
    pub fn with_rag_config(mut self, config: RagConfig) -> Self {
        self.config = config;
        if let Ok(mut inner) = self.inner.lock() {
            inner.config = config;
            if let Ok(rebuilt) = rebuild_state(&inner.db, config) {
                inner.datasets = rebuilt;
            }
        }
        self
    }

    /// Attach a server embedder (the `serve-engine` path), enabling text-only ingest
    /// and `query_text`.
    #[cfg(feature = "serve-engine")]
    #[must_use]
    pub fn with_embedder(mut self, embedder: HostEmbedder) -> Self {
        self.embedder = Some(embedder);
        self
    }

    /// Pre-load the embed model by firing ONE throwaway embed (`KX_SERVE_WARM_EMBED`).
    /// Probe-only: it pulls the model resident so the first real ingest is already
    /// warm (`T-OLLAMA-EMBED-COLD-TIMEOUT`); an error (no embedder / unreachable
    /// daemon) is ignored and never force-pulls a missing model. Returns whether a
    /// warm-up embed was attempted. Blocking — call it off the async runtime.
    #[cfg(feature = "serve-engine")]
    pub fn warm_embed(&self) -> bool {
        match &self.embedder {
            Some(e) => {
                let _ = e.embed("warmup");
                true
            }
            None => false,
        }
    }

    /// The current operator retrieval config (chunk params / hybrid knobs).
    #[must_use]
    pub fn rag_config(&self) -> RagConfig {
        self.config
    }

    /// Compute the live index fingerprint for `dim` from the current embedder +
    /// config — `None` when there is no server embedder (the client-vector path).
    #[cfg(feature = "serve-engine")]
    fn compute_fingerprint(&self, dim: u32) -> Option<String> {
        let e = self.embedder.as_ref()?;
        let fp = index_fingerprint(
            &e.model_id_string(),
            e.pooling_tag(),
            dim,
            CHUNKER_VERSION,
            u32::try_from(self.config.chunk_max_chars).unwrap_or(u32::MAX),
            u32::try_from(self.config.chunk_overlap_chars).unwrap_or(u32::MAX),
            TOKENIZER_VERSION,
            self.config.stopwords,
        );
        Some(ContentRef::from_bytes(fp).to_hex())
    }

    #[cfg(not(feature = "serve-engine"))]
    #[allow(clippy::unused_self)]
    fn compute_fingerprint(&self, _dim: u32) -> Option<String> {
        None
    }

    /// Embed `text` server-side, or [`DatasetError::EmbedderUnavailable`] when no
    /// embedder is wired (an `hnsw`-only build, or `serve-engine` without a model).
    #[cfg_attr(not(feature = "serve-engine"), allow(clippy::unused_self))]
    fn embed_text(&self, text: &str) -> Result<Vec<f32>, DatasetError> {
        #[cfg(feature = "serve-engine")]
        {
            self.embedder
                .as_ref()
                .ok_or(DatasetError::EmbedderUnavailable)?
                .embed(text)
        }
        #[cfg(not(feature = "serve-engine"))]
        {
            let _ = text;
            Err(DatasetError::EmbedderUnavailable)
        }
    }

    /// The shared search core for [`DatasetView::query`] and
    /// [`FuzzyDiscoveryView::discover`]: resolve the query vector (client-vector path
    /// takes precedence; text-only falls back to the server embedder), guard
    /// staleness, then run dense ANN + (for hybrid) BM25, RRF-fuse, MMR-rerank —
    /// all under ONE lock. Returns `(chunk_ref, content, score, parent_ref,
    /// chunk_index, chunk_count)`. One source of the search logic; no re-lock race.
    fn search(
        &self,
        dataset: &str,
        query_embedding: Option<&[f32]>,
        query_text: &str,
        k: usize,
        mode: RetrievalMode,
        rerank: Option<bool>,
    ) -> Result<Vec<SearchHit>, DatasetError> {
        if k == 0 {
            return Ok(Vec::new());
        }
        let k = k.min(MAX_K);
        let effective_mode = match mode {
            RetrievalMode::Default => self.config.default_mode,
            m => m,
        };
        // Resolve the query vector outside the lock (client-vector takes precedence;
        // text-only falls back to the server embedder). `server_embedded` ⇒ the
        // staleness guard applies (the client-vector path owns its own space).
        let (qvec, server_embedded) = match query_embedding {
            Some(v) if !v.is_empty() => (v.to_vec(), false),
            _ => {
                if query_text.is_empty() {
                    return Err(DatasetError::InvalidArgument(
                        "query requires query_text or a query_embedding".to_string(),
                    ));
                }
                (self.embed_text(query_text)?, true)
            }
        };
        if !all_finite(&qvec) {
            return Err(DatasetError::InvalidArgument(
                "query vector must be finite (no NaN/inf)".to_string(),
            ));
        }
        // The live fingerprint (server-embed only) — computed outside the lock.
        let live_fp = server_embedded
            .then(|| {
                let dim = u32::try_from(qvec.len()).unwrap_or(0);
                self.compute_fingerprint(dim)
            })
            .flatten();

        let inner = self
            .inner
            .lock()
            .map_err(|_| DatasetError::Internal("dataset store lock poisoned".to_string()))?;
        let state = inner.datasets.get(dataset).ok_or(DatasetError::NotFound)?;
        if let Some(dim) = state.dim {
            let qdim = u32::try_from(qvec.len())
                .map_err(|_| DatasetError::InvalidArgument("query vector too large".to_string()))?;
            if qdim != dim {
                return Err(DatasetError::DimMismatch(format!(
                    "dataset dim is {dim}, got a {qdim}-dim query"
                )));
            }
        }
        // Staleness guard (server-embed only): never query a corpus embedded under a
        // different model / chunk config with the live embedder (incompatible space).
        if let Some(live) = &live_fp {
            if !state.embed_fingerprint.is_empty() && state.embed_fingerprint != *live {
                return Err(DatasetError::StaleIndex(format!(
                    "dataset '{dataset}' was indexed under a different embed model / chunk \
                     config; re-ingest to rebuild (the client-vector path is unaffected)"
                )));
            }
        }

        // Candidate width: a bounded multiple of k (pinned function of k ⇒ reproducible).
        let n = k.saturating_mul(4).clamp(k, 256);
        let dense: Vec<Hit> = state.index.query(&qvec, n);
        // Sparse arm only when hybrid is requested AND there is query text to match.
        let sparse_on = effective_mode == RetrievalMode::Hybrid
            && !query_text.is_empty()
            && !state.lex.is_empty();
        let fused: Vec<Hit> = if sparse_on {
            let sparse = state.lex.query(query_text, n);
            rrf_fuse(&dense, &sparse, self.config.rrf_k, n)
        } else {
            let mut d = dense;
            d.truncate(n);
            d
        };
        // MMR diversity rerank (deterministic, off the committed fact): preserves the
        // fused relevance order while demoting near-duplicate passages. Skipped when
        // disabled by the operator — or by a per-query `rerank` override (RC4c).
        let ranked: Vec<Hit> = if rerank.unwrap_or(self.config.rerank) {
            #[allow(clippy::cast_precision_loss)]
            let lambda = self.config.mmr_lambda_bp as f32 / 10_000.0;
            mmr_rerank(&fused, |id| state.index.vector_of(id), lambda, k)
        } else {
            let mut f = fused;
            f.truncate(k);
            f
        };

        Ok(ranked
            .into_iter()
            .map(|h| {
                let content = state.docs.get(&h.id).cloned().unwrap_or_default();
                let prov = state.prov.get(&h.id).copied().unwrap_or(ChunkProv {
                    parent_ref: h.id,
                    chunk_index: 0,
                });
                let chunk_count = state
                    .parent_chunks
                    .get(&prov.parent_ref)
                    .copied()
                    .unwrap_or(1);
                SearchHit {
                    chunk_ref: h.id,
                    content,
                    score: h.score,
                    parent_ref: prov.parent_ref,
                    chunk_index: prov.chunk_index,
                    chunk_count,
                }
            })
            .collect())
    }
}

impl DatasetView for HostDatasetView {
    fn list_datasets(&self) -> Vec<DatasetSummaryEntry> {
        let Ok(inner) = self.inner.lock() else {
            return Vec::new();
        };
        let mut out: Vec<DatasetSummaryEntry> = inner
            .datasets
            .iter()
            .map(|(name, s)| DatasetSummaryEntry {
                dataset_id: name.clone(),
                name: name.clone(),
                // doc_count = distinct PARENT documents; chunk_count = chunks.
                doc_count: u64::try_from(s.parent_chunks.len()).unwrap_or(u64::MAX),
                dim: s.dim.unwrap_or(0),
                created_ms: s.created_ms,
                chunked: s.chunked,
                embed_model_fingerprint: s.embed_fingerprint.clone(),
                index_version: s.index_version,
                chunk_count: u64::try_from(s.docs.len()).unwrap_or(u64::MAX),
            })
            .collect();
        out.sort_by(|a, b| a.dataset_id.cmp(&b.dataset_id));
        out
    }

    fn ingest(&self, dataset: &str, docs: &[IngestDoc<'_>]) -> Result<IngestOutcome, DatasetError> {
        validate_dataset_name(dataset)?;
        if docs.is_empty() {
            return Err(DatasetError::InvalidArgument("no documents".to_string()));
        }
        // Resolve every chunk + vector BEFORE taking the lock (embedding may be slow +
        // the embed backend is its own owner thread; never hold the dataset lock
        // across it). A client-vector doc is one whole-doc chunk (the client owns
        // granularity); a server-embed doc is chunked, then each chunk is embedded.
        let chunk_params = ChunkParams {
            max_chars: self.config.chunk_max_chars,
            overlap_chars: self.config.chunk_overlap_chars,
        };
        let mut rows: Vec<ChunkRow> = Vec::with_capacity(docs.len());
        let mut used_server_embed = false;
        for d in docs {
            if d.content.is_empty() {
                return Err(DatasetError::InvalidArgument(
                    "empty document content".to_string(),
                ));
            }
            match d.embedding {
                Some(v) if !v.is_empty() => {
                    // Client-vector path: one chunk = the whole doc (no chunking).
                    if !all_finite(v) {
                        return Err(DatasetError::InvalidArgument(
                            "embedding must be finite (no NaN/inf)".to_string(),
                        ));
                    }
                    let chunk_ref = ContentRef::of(d.content);
                    rows.push(ChunkRow {
                        chunk_ref,
                        content: d.content.to_vec(),
                        vector: v.to_vec(),
                        parent_ref: chunk_ref,
                        chunk_index: 0,
                    });
                }
                _ => {
                    // Server-embed path: chunk the document, embed each passage.
                    used_server_embed = true;
                    let text = std::str::from_utf8(d.content).map_err(|_| {
                        DatasetError::InvalidArgument(
                            "server-embed requires UTF-8 text".to_string(),
                        )
                    })?;
                    let parent_ref = ContentRef::of(d.content);
                    let chunks = chunk(text, chunk_params);
                    if self.config.max_chunks_per_doc > 0
                        && chunks.len() > self.config.max_chunks_per_doc
                    {
                        return Err(DatasetError::InvalidArgument(format!(
                            "document produces {} chunks, over the {}-chunk cap",
                            chunks.len(),
                            self.config.max_chunks_per_doc
                        )));
                    }
                    for ch in chunks {
                        let chunk_bytes = ch.text.into_bytes();
                        let chunk_ref = ContentRef::of(&chunk_bytes);
                        let vector =
                            self.embed_text(std::str::from_utf8(&chunk_bytes).unwrap_or_default())?;
                        if vector.is_empty() || !all_finite(&vector) {
                            return Err(DatasetError::InvalidArgument(
                                "embedding must be non-empty and finite (no NaN/inf)".to_string(),
                            ));
                        }
                        rows.push(ChunkRow {
                            chunk_ref,
                            content: chunk_bytes,
                            vector,
                            parent_ref,
                            chunk_index: ch.index,
                        });
                    }
                }
            }
        }
        if rows.is_empty() {
            return Err(DatasetError::InvalidArgument(
                "no indexable content".to_string(),
            ));
        }
        // Stamp the fingerprint on a server-embed ingest (the dim is the resolved one).
        let fingerprint = if used_server_embed {
            let dim = u32::try_from(rows[0].vector.len()).unwrap_or(0);
            self.compute_fingerprint(dim)
        } else {
            None
        };
        self.inner
            .lock()
            .map_err(|_| DatasetError::Internal("dataset store lock poisoned".to_string()))?
            .ingest_resolved(dataset, rows, used_server_embed, fingerprint)
    }

    fn query(
        &self,
        dataset: &str,
        query_embedding: Option<&[f32]>,
        query_text: &str,
        k: usize,
        mode: RetrievalMode,
        rerank: Option<bool>,
    ) -> Result<Vec<DatasetHitEntry>, DatasetError> {
        Ok(self
            .search(dataset, query_embedding, query_text, k, mode, rerank)?
            .into_iter()
            .map(|h| DatasetHitEntry {
                content_ref: *h.chunk_ref.as_bytes(),
                content: h.content,
                score: h.score,
                parent_ref: *h.parent_ref.as_bytes(),
                chunk_index: h.chunk_index,
                chunk_count: h.chunk_count,
            })
            .collect())
    }
}

impl FuzzyDiscoveryView for HostDatasetView {
    /// Advisory fuzzy-in / exact-out discovery: the SAME search as
    /// [`DatasetView::query`], but the result is the ordered content-ref SET +
    /// a DISPLAY-ONLY basis-point score (SN-8) — no content bytes on the wire.
    /// The caller joins back to bytes with an EXACT `GetContent` on the ref.
    fn discover(
        &self,
        dataset: &str,
        query_embedding: Option<&[f32]>,
        query_text: &str,
        k: usize,
        mode: RetrievalMode,
    ) -> Result<Vec<FuzzyHitEntry>, DatasetError> {
        Ok(self
            // Fuzzy discovery uses the operator's MMR default (no per-call override).
            .search(dataset, query_embedding, query_text, k, mode, None)?
            .into_iter()
            .map(|h| FuzzyHitEntry {
                content_ref: *h.chunk_ref.as_bytes(),
                score_bp: score_to_bp(h.score),
                parent_ref: *h.parent_ref.as_bytes(),
                chunk_index: h.chunk_index,
            })
            .collect())
    }
}

/// Rebuild every dataset's in-memory state (the dense HNSW + sparse BM25 indices +
/// the chunk→content map + chunk provenance) from the durable rows on open. Both
/// indices are built SYNCHRONOUSLY here, so a dataset is fully warm before it is
/// queryable (closes the cold-index race). A legacy (pre-RC4a) row has a NULL
/// `parent_ref` ⇒ it is a single whole-doc chunk that is its own parent.
fn rebuild_state(
    conn: &Connection,
    config: RagConfig,
) -> Result<HashMap<String, DatasetState>, rusqlite::Error> {
    let bm25 = config.bm25_params();
    let mut datasets: HashMap<String, DatasetState> = HashMap::new();
    {
        let mut stmt = conn.prepare(
            "SELECT name, created_ms, dim, chunked, embed_fingerprint, index_version FROM datasets",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)?,
            ))
        })?;
        for row in rows {
            let (name, created_ms, dim, chunked, fp, iv) = row?;
            let mut state = DatasetState::empty(created_ms, bm25);
            if dim > 0 {
                state.dim = u32::try_from(dim).ok();
            }
            state.chunked = chunked != 0;
            state.embed_fingerprint = fp;
            state.index_version = u32::try_from(iv).unwrap_or(INDEX_VERSION);
            datasets.insert(name, state);
        }
    }
    {
        let mut stmt = conn.prepare(
            "SELECT dataset, ref, content, vector, parent_ref, chunk_index FROM documents \
             ORDER BY rowid",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, Vec<u8>>(2)?,
                row.get::<_, Vec<u8>>(3)?,
                row.get::<_, Option<Vec<u8>>>(4)?,
                row.get::<_, i64>(5)?,
            ))
        })?;
        for row in rows {
            let (dataset, ref_blob, content, vec_blob, parent_blob, chunk_index) = row?;
            let Ok(ref_arr) = <[u8; 32]>::try_from(ref_blob.as_slice()) else {
                continue; // a corrupt (non-32B) ref row is skipped, never panics
            };
            let chunk_ref = ContentRef::from_bytes(ref_arr);
            let vector = decode_vector_le(&vec_blob);
            // A NULL/corrupt parent_ref ⇒ a legacy whole-doc chunk = its own parent.
            let parent_ref = parent_blob
                .and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok())
                .map_or(chunk_ref, ContentRef::from_bytes);
            let ci = u32::try_from(chunk_index).unwrap_or(0);
            let state = datasets
                .entry(dataset.clone())
                .or_insert_with(|| DatasetState::empty(now_ms(), bm25));
            if state.dim.is_none() && !vector.is_empty() {
                state.dim = u32::try_from(vector.len()).ok();
            }
            // A row whose stored vector dim disagrees with the dataset's (externally
            // corrupted db only — the ingest path enforces a uniform batch dim) is
            // skipped from EVERY index/map (+ warn) so counts never over-report.
            let dim_ok = state
                .dim
                .is_none_or(|d| u32::try_from(vector.len()).is_ok_and(|n| n == d));
            if !dim_ok {
                tracing::warn!(
                    dataset = %dataset,
                    "skipping a dim-mismatched dataset row on rebuild (corrupt db)"
                );
                continue;
            }
            state.index.insert(chunk_ref, vector);
            if let Ok(text) = std::str::from_utf8(&content) {
                state.lex.insert(chunk_ref, text);
            }
            state.docs.insert(chunk_ref, content);
            state.prov.insert(
                chunk_ref,
                ChunkProv {
                    parent_ref,
                    chunk_index: ci,
                },
            );
            *state.parent_chunks.entry(parent_ref).or_insert(0) += 1;
        }
    }
    Ok(datasets)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 4-dim unit-ish vector pointing mostly along axis `i` — clearly separated so
    /// the (approximate) HNSW order is unambiguous for the tiny test corpora.
    fn axis_vec(i: usize) -> Vec<f32> {
        let mut v = vec![0.1f32; 4];
        v[i] = 1.0;
        v
    }

    fn open_view(dir: &std::path::Path) -> HostDatasetView {
        HostDatasetView::open(dir).unwrap()
    }

    fn doc<'a>(content: &'a [u8], embedding: &'a [f32]) -> IngestDoc<'a> {
        IngestDoc {
            content,
            embedding: Some(embedding),
        }
    }

    #[test]
    fn ingest_then_query_returns_nearest_by_client_vector() {
        let dir = tempfile::tempdir().unwrap();
        let view = open_view(dir.path());
        let (a, b, c) = (axis_vec(0), axis_vec(1), axis_vec(2));
        let out = view
            .ingest(
                "corpus",
                &[doc(b"alpha", &a), doc(b"bravo", &b), doc(b"charlie", &c)],
            )
            .unwrap();
        assert_eq!(out.inserted, 3);
        assert_eq!(out.doc_count, 3);
        assert_eq!(out.dim, 4);

        // A query closest to axis 1 ⇒ "bravo" is the top hit, with its bytes attached.
        let hits = view
            .query(
                "corpus",
                Some(&axis_vec(1)),
                "",
                1,
                RetrievalMode::Default,
                None,
            )
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].content, b"bravo");
        assert_eq!(hits[0].content_ref, *ContentRef::of(b"bravo").as_bytes());
    }

    #[test]
    fn reopen_recovers_datasets_and_serves_queries() {
        let dir = tempfile::tempdir().unwrap();
        {
            let view = open_view(dir.path());
            view.ingest(
                "c",
                &[doc(b"alpha", &axis_vec(0)), doc(b"bravo", &axis_vec(1))],
            )
            .unwrap();
        } // drop the view (and its connection)
        let reopened = open_view(dir.path());
        let list = reopened.list_datasets();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].dataset_id, "c");
        assert_eq!(list[0].doc_count, 2);
        assert_eq!(list[0].dim, 4);
        // The rebuilt index still serves the right neighbour.
        let hits = reopened
            .query("c", Some(&axis_vec(0)), "", 1, RetrievalMode::Default, None)
            .unwrap();
        assert_eq!(hits[0].content, b"alpha");
    }

    #[test]
    fn dedup_is_content_addressed() {
        let dir = tempfile::tempdir().unwrap();
        let view = open_view(dir.path());
        view.ingest("c", &[doc(b"same", &axis_vec(0))]).unwrap();
        let again = view.ingest("c", &[doc(b"same", &axis_vec(0))]).unwrap();
        assert_eq!(again.inserted, 0, "a repeat content is a no-op");
        assert_eq!(again.doc_count, 1);
    }

    #[test]
    fn dim_mismatch_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let view = open_view(dir.path());
        view.ingest("c", &[doc(b"a", &axis_vec(0))]).unwrap();
        let three = vec![0.0f32; 3];
        let err = view.ingest("c", &[doc(b"b", &three)]).unwrap_err();
        assert!(matches!(err, DatasetError::DimMismatch(_)));
    }

    /// PR-B back-compat (E3): a dataset's vector dimension is fixed by its first
    /// insert, so a QUERY vector of a different dimension — e.g. after switching
    /// `KX_SERVE_EMBED_MODEL` to a model of another dim on an existing corpus — is
    /// refused loudly, never silently returning garbage neighbours.
    #[test]
    fn query_dim_mismatch_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let view = open_view(dir.path());
        view.ingest("c", &[doc(b"a", &axis_vec(0))]).unwrap(); // dim 4
        let three = vec![0.0f32; 3];
        let err = view
            .query("c", Some(&three), "", 1, RetrievalMode::Default, None)
            .unwrap_err();
        assert!(matches!(err, DatasetError::DimMismatch(_)));
    }

    #[test]
    fn unknown_dataset_is_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let view = open_view(dir.path());
        let err = view
            .query(
                "nope",
                Some(&axis_vec(0)),
                "",
                1,
                RetrievalMode::Default,
                None,
            )
            .unwrap_err();
        assert!(matches!(err, DatasetError::NotFound));
    }

    #[test]
    fn server_embed_without_an_embedder_is_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        let view = open_view(dir.path()); // no embedder wired
        let textonly = IngestDoc {
            content: b"text",
            embedding: None,
        };
        let err = view.ingest("c", &[textonly]).unwrap_err();
        assert!(matches!(err, DatasetError::EmbedderUnavailable));
        let qerr = view
            .query("c", None, "hello", 1, RetrievalMode::Default, None)
            .unwrap_err();
        // unknown dataset OR embedder-unavailable — both are honest; the embed is
        // attempted first (before the lock), so EmbedderUnavailable wins here.
        assert!(matches!(qerr, DatasetError::EmbedderUnavailable));
    }

    /// PR-B: a deterministic FFI-free embedder — a one-hot 4-dim vector keyed on the
    /// first matching keyword (mirrors the harness `KeywordEmbed` stub). Proves the
    /// re-gated server-embed path (`HostEmbedder` → `HostDatasetView`) runs with NO
    /// model and NO FFI, i.e. an Ollama-only (`serve-engine`) serve embeds datasets at
    /// parity. The `EmbeddingBackend` capability rides the host `RoutingBackend` in
    /// production (unit-tested in `routing_backend`); here we drive the embedder seam
    /// directly so the datasets path is exercised without a live daemon.
    #[cfg(feature = "serve-engine")]
    struct KeywordEmbed;
    #[cfg(feature = "serve-engine")]
    impl kx_inference::InferenceBackend for KeywordEmbed {
        fn dispatch(
            &self,
            _model_id: &kx_mote::ModelId,
            _input: &kx_inference::InferenceInput,
            _params: &kx_inference::InferenceParams,
            _warrant: &kx_warrant::WarrantSpec,
        ) -> Result<kx_inference::InferenceOutput, kx_inference::InferenceError> {
            Err(kx_inference::InferenceError::Unsupported {
                reason: "fake: chat unsupported",
            })
        }
        fn supports(&self, _model_id: &kx_mote::ModelId) -> bool {
            true
        }
        fn name(&self) -> &'static str {
            "fake-embed"
        }
    }
    #[cfg(feature = "serve-engine")]
    impl kx_inference::EmbeddingBackend for KeywordEmbed {
        fn dispatch_embedding(
            &self,
            model_id: &kx_mote::ModelId,
            text: &str,
            _pooling: kx_inference::EmbeddingPooling,
            _warrant: &kx_warrant::WarrantSpec,
        ) -> Result<kx_inference::EmbeddingOutput, kx_inference::InferenceError> {
            let t = text.to_ascii_lowercase();
            let i = if t.contains("alpha") {
                0
            } else if t.contains("bravo") {
                1
            } else if t.contains("charlie") {
                2
            } else {
                3
            };
            let mut v = vec![0.1f32; 4];
            v[i] = 1.0;
            Ok(kx_inference::EmbeddingOutput {
                vector: v,
                dim: 4,
                backend_name: "fake-embed",
                model_id: model_id.clone(),
                elapsed: std::time::Duration::ZERO,
            })
        }
    }

    #[cfg(feature = "serve-engine")]
    #[test]
    fn server_embed_text_only_via_host_embedder_runs_ffi_free() {
        let dir = tempfile::tempdir().unwrap();
        let embedder = HostEmbedder::new(
            std::sync::Arc::new(KeywordEmbed),
            kx_mote::ModelId("fake-embed".into()),
            kx_warrant::WarrantSpec::default(),
        );
        let view = HostDatasetView::open(dir.path())
            .unwrap()
            .with_embedder(embedder);
        // Text-only ingest (embedding: None) ⇒ the server embedder runs per doc.
        let textonly = |c: &'static [u8]| IngestDoc {
            content: c,
            embedding: None,
        };
        let out = view
            .ingest(
                "corpus",
                &[
                    textonly(b"alpha one"),
                    textonly(b"bravo two"),
                    textonly(b"charlie three"),
                ],
            )
            .unwrap();
        assert_eq!(out.inserted, 3);
        assert_eq!(out.dim, 4);
        // A text-only query is embedded server-side ⇒ the "bravo" doc is nearest.
        let hits = view
            .query(
                "corpus",
                None,
                "where is bravo",
                1,
                RetrievalMode::Default,
                None,
            )
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].content, b"bravo two");
    }

    #[test]
    fn bad_dataset_names_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let view = open_view(dir.path());
        for bad in ["", "..", ".", "a/b", "x y", &"z".repeat(200)] {
            let err = view.ingest(bad, &[doc(b"a", &axis_vec(0))]).unwrap_err();
            assert!(
                matches!(err, DatasetError::InvalidArgument(_)),
                "name {bad:?} must be rejected"
            );
        }
    }

    #[test]
    fn empty_content_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let view = open_view(dir.path());
        let err = view.ingest("c", &[doc(b"", &axis_vec(0))]).unwrap_err();
        assert!(matches!(err, DatasetError::InvalidArgument(_)));
    }

    #[test]
    fn non_finite_vectors_are_rejected_on_ingest_and_query() {
        let dir = tempfile::tempdir().unwrap();
        let view = open_view(dir.path());
        // A NaN/inf component would poison the cosine ANN ordering → rejected.
        let nan = vec![f32::NAN, 0.0, 0.0, 0.0];
        let err = view.ingest("c", &[doc(b"x", &nan)]).unwrap_err();
        assert!(matches!(err, DatasetError::InvalidArgument(_)));
        // The bad ingest did not create the dataset (it failed before any write).
        assert!(view.list_datasets().is_empty());

        view.ingest("c", &[doc(b"a", &axis_vec(0))]).unwrap();
        let inf = vec![f32::INFINITY, 0.0, 0.0, 0.0];
        let qerr = view
            .query("c", Some(&inf), "", 1, RetrievalMode::Default, None)
            .unwrap_err();
        assert!(matches!(qerr, DatasetError::InvalidArgument(_)));
    }

    #[test]
    fn discover_returns_exact_out_refs_and_bp_scores() {
        let dir = tempfile::tempdir().unwrap();
        let view = open_view(dir.path());
        let (a, b, c) = (axis_vec(0), axis_vec(1), axis_vec(2));
        view.ingest(
            "corpus",
            &[doc(b"alpha", &a), doc(b"bravo", &b), doc(b"charlie", &c)],
        )
        .unwrap();
        // Closest to axis 1 ⇒ "bravo" is the top hit; the result carries the EXACT
        // content-ref + a display-only bp score (no content bytes — SN-8 exact-out).
        let hits = view
            .discover("corpus", Some(&axis_vec(1)), "", 3, RetrievalMode::Default)
            .unwrap();
        assert_eq!(hits.len(), 3);
        assert_eq!(hits[0].content_ref, *ContentRef::of(b"bravo").as_bytes());
        assert!(
            hits[0].score_bp <= 10_000,
            "bp score is a 0..=10000 display band"
        );
        assert!(hits[0].score_bp >= hits[1].score_bp, "best-first ordering");
    }

    #[test]
    fn discover_clamps_zero_k_and_reports_unknown_dataset() {
        let dir = tempfile::tempdir().unwrap();
        let view = open_view(dir.path());
        view.ingest("c", &[doc(b"a", &axis_vec(0))]).unwrap();
        assert!(view
            .discover("c", Some(&axis_vec(0)), "", 0, RetrievalMode::Default)
            .unwrap()
            .is_empty());
        let err = view
            .discover("nope", Some(&axis_vec(0)), "", 1, RetrievalMode::Default)
            .unwrap_err();
        assert!(matches!(err, DatasetError::NotFound));
    }

    // ---- RC4a: chunking, hybrid, provenance, fingerprint staleness, back-compat ----

    #[test]
    fn client_vector_ingest_is_not_chunked() {
        let dir = tempfile::tempdir().unwrap();
        let view = open_view(dir.path());
        view.ingest(
            "c",
            &[doc(b"a fairly long client document here", &axis_vec(0))],
        )
        .unwrap();
        let list = view.list_datasets();
        assert!(!list[0].chunked, "the client-vector path is never chunked");
        assert_eq!(list[0].chunk_count, 1, "one whole-doc chunk");
        assert_eq!(list[0].doc_count, 1);
        assert!(
            list[0].embed_model_fingerprint.is_empty(),
            "no server embed ⇒ no fingerprint"
        );
        // A hit's parent is itself (a whole-doc chunk).
        let hits = view
            .query("c", Some(&axis_vec(0)), "", 1, RetrievalMode::Default, None)
            .unwrap();
        assert_eq!(hits[0].parent_ref, hits[0].content_ref);
        assert_eq!(hits[0].chunk_index, 0);
        assert_eq!(hits[0].chunk_count, 1);
    }

    #[cfg(feature = "serve-engine")]
    #[test]
    fn server_embed_chunks_a_long_document_into_passages() {
        let dir = tempfile::tempdir().unwrap();
        let embedder = HostEmbedder::new(
            std::sync::Arc::new(KeywordEmbed),
            kx_mote::ModelId("fake-embed".into()),
            kx_warrant::WarrantSpec::default(),
        );
        let view = HostDatasetView::open(dir.path())
            .unwrap()
            .with_rag_config(RagConfig {
                chunk_max_chars: 12,
                chunk_overlap_chars: 3,
                ..RagConfig::default()
            })
            .with_embedder(embedder);
        let long = "alpha bravo charlie delta echo foxtrot golf hotel india";
        let out = view
            .ingest(
                "c",
                &[IngestDoc {
                    content: long.as_bytes(),
                    embedding: None,
                }],
            )
            .unwrap();
        assert_eq!(out.doc_count, 1, "one parent document");
        let list = view.list_datasets();
        assert_eq!(list[0].doc_count, 1, "one parent");
        assert!(
            list[0].chunk_count > 1,
            "the long doc split into multiple chunks (got {})",
            list[0].chunk_count
        );
        assert!(list[0].chunked, "the chunked flag is set");
        assert!(
            !list[0].embed_model_fingerprint.is_empty(),
            "a server-embed ingest stamps the fingerprint"
        );
        assert_eq!(list[0].index_version, INDEX_VERSION);
    }

    #[cfg(feature = "serve-engine")]
    #[test]
    fn hybrid_surfaces_a_keyword_match_a_dense_tie_misses() {
        let dir = tempfile::tempdir().unwrap();
        let embedder = HostEmbedder::new(
            std::sync::Arc::new(KeywordEmbed),
            kx_mote::ModelId("fake-embed".into()),
            kx_warrant::WarrantSpec::default(),
        );
        // No chunking (small docs); rerank OFF so the test reads the pure RRF fusion.
        let view = HostDatasetView::open(dir.path())
            .unwrap()
            .with_rag_config(RagConfig {
                rerank: false,
                ..RagConfig::default()
            })
            .with_embedder(embedder);
        // Both docs embed to axis 0 ("alpha") — a DENSE tie; only one has "xylophone".
        view.ingest(
            "c",
            &[
                IngestDoc {
                    content: b"alpha foo bar",
                    embedding: None,
                },
                IngestDoc {
                    content: b"alpha xylophone baz",
                    embedding: None,
                },
            ],
        )
        .unwrap();
        // A single rare query term: dense cannot separate the docs, but the BM25 arm
        // gives the ONLY doc containing "xylophone" a fusion boost no other doc has.
        let hits = view
            .query("c", None, "xylophone", 5, RetrievalMode::Hybrid, None)
            .unwrap();
        assert_eq!(
            hits[0].content, b"alpha xylophone baz",
            "the BM25 keyword match leads the hybrid fusion"
        );
    }

    /// RC4c: the per-query `rerank` override correctly selects the MMR path —
    /// `Some(false)` on a rerank-on view matches a rerank-off view, and vice-versa.
    /// A near-duplicate corpus makes MMR change the result, so the equivalence is
    /// non-trivial (proves the override is plumbed, not a no-op).
    #[test]
    fn per_query_rerank_override_selects_the_mmr_path() {
        // Two dense-identical docs (A, B) + one distinct (C); MMR demotes the dup.
        let (v0a, v0b, v2) = (axis_vec(0), axis_vec(0), axis_vec(2));
        let corpus = [
            doc(b"alpha one", &v0a),
            doc(b"alpha two", &v0b),
            doc(b"charlie far", &v2),
        ];
        let q = axis_vec(0);

        let mk = |rerank: bool, dir: &std::path::Path| {
            let view = open_view(dir).with_rag_config(RagConfig {
                rerank,
                // λ=0 ⇒ pure diversity, so MMR (when on) provably demotes the exact
                // duplicate B in favour of the distinct C — making on≠off observable.
                mmr_lambda_bp: 0,
                ..RagConfig::default()
            });
            view.ingest("c", &corpus).unwrap();
            view
        };
        let refs =
            |hits: &[DatasetHitEntry]| hits.iter().map(|h| h.content_ref).collect::<Vec<_>>();

        let d_on = tempfile::tempdir().unwrap();
        let on = mk(true, d_on.path());
        let d_off = tempfile::tempdir().unwrap();
        let off = mk(false, d_off.path());

        // Config defaults (no override).
        let on_default = on
            .query("c", Some(&q), "", 2, RetrievalMode::Default, None)
            .unwrap();
        let off_default = off
            .query("c", Some(&q), "", 2, RetrievalMode::Default, None)
            .unwrap();
        // MMR must actually change the top-2 on this near-dup corpus (else the test is vacuous).
        assert_ne!(
            refs(&on_default),
            refs(&off_default),
            "MMR reorders the near-dup corpus"
        );

        // Override flips each view to the OTHER config's behavior.
        let on_forced_off = on
            .query("c", Some(&q), "", 2, RetrievalMode::Default, Some(false))
            .unwrap();
        let off_forced_on = off
            .query("c", Some(&q), "", 2, RetrievalMode::Default, Some(true))
            .unwrap();
        assert_eq!(
            refs(&on_forced_off),
            refs(&off_default),
            "Some(false) disables MMR"
        );
        assert_eq!(
            refs(&off_forced_on),
            refs(&on_default),
            "Some(true) enables MMR"
        );
    }

    #[cfg(feature = "serve-engine")]
    #[test]
    fn fingerprint_staleness_refuses_a_model_swap_but_exempts_client_vectors() {
        let dir = tempfile::tempdir().unwrap();
        // Ingest server-embed under "model-a".
        {
            let view = HostDatasetView::open(dir.path())
                .unwrap()
                .with_embedder(HostEmbedder::new(
                    std::sync::Arc::new(KeywordEmbed),
                    kx_mote::ModelId("model-a".into()),
                    kx_warrant::WarrantSpec::default(),
                ));
            view.ingest(
                "c",
                &[IngestDoc {
                    content: b"alpha one",
                    embedding: None,
                }],
            )
            .unwrap();
        }
        // Reopen with a DIFFERENT embed model id ⇒ a server-embed query is refused.
        let view = HostDatasetView::open(dir.path())
            .unwrap()
            .with_embedder(HostEmbedder::new(
                std::sync::Arc::new(KeywordEmbed),
                kx_mote::ModelId("model-b".into()),
                kx_warrant::WarrantSpec::default(),
            ));
        let err = view
            .query("c", None, "where is alpha", 1, RetrievalMode::Default, None)
            .unwrap_err();
        assert!(
            matches!(err, DatasetError::StaleIndex(_)),
            "a same-dim model swap is refused, never silently mis-ranked"
        );
        // The client-vector path owns its own space ⇒ exempt from the guard.
        assert!(
            view.query("c", Some(&axis_vec(0)), "", 1, RetrievalMode::Default, None)
                .is_ok(),
            "the client-vector path is exempt from the staleness guard"
        );
    }

    #[test]
    fn legacy_db_without_chunk_columns_migrates_and_queries() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("datasets.db");
        // Simulate a pre-RC4a db: the OLD schema (no chunk columns) + a whole-doc row.
        {
            let conn = Connection::open(&db_path).unwrap();
            conn.execute_batch(
                "CREATE TABLE datasets(name TEXT PRIMARY KEY, created_ms INTEGER NOT NULL, \
                 dim INTEGER NOT NULL DEFAULT 0);
                 CREATE TABLE documents(dataset TEXT NOT NULL, ref BLOB NOT NULL, \
                 content BLOB NOT NULL, vector BLOB NOT NULL, PRIMARY KEY(dataset, ref));",
            )
            .unwrap();
            let v = axis_vec(1);
            let content = b"legacy bravo".to_vec();
            let cref = ContentRef::of(&content);
            conn.execute(
                "INSERT INTO datasets(name, created_ms, dim) VALUES('c', 0, 4)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO documents(dataset, ref, content, vector) VALUES('c', ?, ?, ?)",
                params![
                    &cref.as_bytes()[..],
                    content.as_slice(),
                    encode_vector_le(&v).as_slice()
                ],
            )
            .unwrap();
        }
        // New code opens it ⇒ migrate_schema adds the columns ⇒ queryable as one chunk.
        let view = open_view(dir.path());
        let list = view.list_datasets();
        assert_eq!(list[0].doc_count, 1);
        assert_eq!(
            list[0].chunk_count, 1,
            "a legacy whole-doc row is one chunk"
        );
        assert!(!list[0].chunked, "a legacy dataset is not marked chunked");
        let hits = view
            .query("c", Some(&axis_vec(1)), "", 1, RetrievalMode::Default, None)
            .unwrap();
        assert_eq!(hits[0].content, b"legacy bravo");
        assert_eq!(
            hits[0].parent_ref, hits[0].content_ref,
            "a legacy chunk is its own parent"
        );
    }
}
