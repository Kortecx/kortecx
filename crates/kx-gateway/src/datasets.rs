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

use kx_content::ContentRef;
use kx_dataset::RetrievalIndex;
use kx_dataset_hnsw::HnswRetrievalIndex;
use kx_gateway_core::{
    DatasetError, DatasetHitEntry, DatasetSummaryEntry, DatasetView, IngestDoc, IngestOutcome,
};
use rusqlite::{params, Connection};

use crate::error::GatewayError;

/// The server-side cap on a query's top-`k` (kept ≤ the HNSW default `ef_search`
/// so the ANN candidate list always covers the requested neighbours). An untrusted
/// `k` cannot force an unbounded scan.
const MAX_K: usize = 64;

/// The maximum dataset-name length (a SQLite key, so this is hygiene, not a
/// path-traversal bound — there is no per-dataset file).
const MAX_NAME_LEN: usize = 128;

/// The durable schema (idempotent). `documents.ref`/`vector` are BLOBs; the HNSW
/// graph is rebuilt from these rows on open.
const SCHEMA: &str = "\
CREATE TABLE IF NOT EXISTS datasets (
    name       TEXT PRIMARY KEY,
    created_ms INTEGER NOT NULL,
    dim        INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS documents (
    dataset TEXT NOT NULL,
    ref     BLOB NOT NULL,
    content BLOB NOT NULL,
    vector  BLOB NOT NULL,
    PRIMARY KEY (dataset, ref)
);";

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

/// One dataset's in-memory state: the rebuilt-on-open ANN index + the ref→content
/// map (for hit snippets). `dim` is fixed by the first insert.
struct DatasetState {
    created_ms: i64,
    dim: Option<u32>,
    index: HnswRetrievalIndex,
    docs: HashMap<ContentRef, Vec<u8>>,
}

impl DatasetState {
    fn empty(created_ms: i64) -> Self {
        Self {
            created_ms,
            dim: None,
            index: HnswRetrievalIndex::new(),
            docs: HashMap::new(),
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
        resolved: Vec<(ContentRef, Vec<u8>, Vec<f32>)>,
    ) -> Result<IngestOutcome, DatasetError> {
        let first = resolved
            .first()
            .ok_or_else(|| DatasetError::InvalidArgument("no documents".to_string()))?;
        let batch_dim = u32::try_from(first.2.len())
            .map_err(|_| DatasetError::InvalidArgument("vector too large".to_string()))?;
        let target_dim = self
            .datasets
            .get(dataset)
            .and_then(|s| s.dim)
            .unwrap_or(batch_dim);
        for (_, _, v) in &resolved {
            let vlen = u32::try_from(v.len())
                .map_err(|_| DatasetError::InvalidArgument("vector too large".to_string()))?;
            if vlen != target_dim {
                return Err(DatasetError::DimMismatch(format!(
                    "dataset dim is {target_dim}, got a {vlen}-dim vector"
                )));
            }
        }

        // Content-addressed dedup BEFORE any write — against the already-persisted docs
        // AND within this batch — so the durable INSERTs and the in-memory apply act on
        // the SAME new-doc set and `inserted` is exact.
        let mut batch_seen: HashSet<ContentRef> = HashSet::new();
        let to_apply: Vec<(ContentRef, Vec<u8>, Vec<f32>)> = {
            let existing = self.datasets.get(dataset);
            resolved
                .into_iter()
                .filter(|(r, _, _)| {
                    let already = existing.is_some_and(|s| s.docs.contains_key(r));
                    !already && batch_seen.insert(*r)
                })
                .collect()
        };

        let created_ms = self
            .datasets
            .get(dataset)
            .map_or_else(now_ms, |s| s.created_ms);

        // DURABLE FIRST: stage every row, then COMMIT, before touching the derived state.
        {
            let txn = self
                .db
                .transaction()
                .map_err(|e| DatasetError::Internal(e.to_string()))?;
            txn.execute(
                "INSERT OR IGNORE INTO datasets(name, created_ms, dim) VALUES(?, ?, ?)",
                params![dataset, created_ms, target_dim],
            )
            .map_err(|e| DatasetError::Internal(e.to_string()))?;
            // Fix the dim on the first real insert (the create above may have written a 0).
            txn.execute(
                "UPDATE datasets SET dim = ? WHERE name = ? AND dim = 0",
                params![target_dim, dataset],
            )
            .map_err(|e| DatasetError::Internal(e.to_string()))?;
            for (doc_ref, content, vector) in &to_apply {
                txn.execute(
                    "INSERT OR IGNORE INTO documents(dataset, ref, content, vector) \
                     VALUES(?, ?, ?, ?)",
                    params![
                        dataset,
                        &doc_ref.as_bytes()[..],
                        content.as_slice(),
                        encode_vector_le(vector).as_slice()
                    ],
                )
                .map_err(|e| DatasetError::Internal(e.to_string()))?;
            }
            txn.commit()
                .map_err(|e| DatasetError::Internal(e.to_string()))?;
        }

        // COMMITTED — only now apply to the derived in-memory state (infallible, so it
        // cannot leave the index/docs ahead of the durable rows).
        let inserted = u64::try_from(to_apply.len()).unwrap_or(u64::MAX);
        let state = self
            .datasets
            .entry(dataset.to_string())
            .or_insert_with(|| DatasetState::empty(created_ms));
        state.dim = Some(target_dim);
        for (doc_ref, content, vector) in to_apply {
            state.index.insert(doc_ref, vector);
            state.docs.insert(doc_ref, content);
        }

        Ok(IngestOutcome {
            dataset_id: dataset.to_string(),
            doc_count: u64::try_from(state.docs.len()).unwrap_or(u64::MAX),
            inserted,
            dim: target_dim,
        })
    }
}

/// The bundled server embedder (the `inference` path): a `kx_inference::EmbeddingBackend`
/// + the model route + warrant + pooling that travel together for every embed call.
#[cfg(feature = "inference")]
pub struct HostEmbedder {
    backend: std::sync::Arc<dyn kx_inference::EmbeddingBackend>,
    model_id: kx_mote::ModelId,
    warrant: kx_warrant::WarrantSpec,
    pooling: kx_inference::EmbeddingPooling,
}

#[cfg(feature = "inference")]
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

    fn embed(&self, text: &str) -> Result<Vec<f32>, DatasetError> {
        let out = self
            .backend
            .dispatch_embedding(&self.model_id, text, self.pooling, &self.warrant)
            .map_err(|e| DatasetError::Internal(format!("embedding: {e}")))?;
        Ok(out.vector)
    }
}

/// A [`DatasetView`] over a durable SQLite store + a rebuilt-on-open HNSW ANN index.
/// VIEW + INGEST (no journal write). Optionally carries a server `HostEmbedder`
/// (the `inference` path); without it, only the client-vector path is available.
pub struct HostDatasetView {
    inner: Mutex<Inner>,
    #[cfg(feature = "inference")]
    embedder: Option<HostEmbedder>,
}

impl HostDatasetView {
    /// Open (or create) the dataset store under `dir`, rebuilding every dataset's
    /// HNSW index from its durable rows.
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
        let datasets = rebuild_state(&conn)
            .map_err(|e| GatewayError::Catalog(format!("datasets rebuild: {e}")))?;
        Ok(Self {
            inner: Mutex::new(Inner { db: conn, datasets }),
            #[cfg(feature = "inference")]
            embedder: None,
        })
    }

    /// Attach a server embedder (the `inference` path), enabling text-only ingest
    /// and `query_text`.
    #[cfg(feature = "inference")]
    #[must_use]
    pub fn with_embedder(mut self, embedder: HostEmbedder) -> Self {
        self.embedder = Some(embedder);
        self
    }

    /// Embed `content` server-side (UTF-8), or [`DatasetError::EmbedderUnavailable`]
    /// when no embedder is wired (the `hnsw`-only build, or `inference` without a model).
    #[cfg_attr(not(feature = "inference"), allow(clippy::unused_self))]
    fn embed_bytes(&self, content: &[u8]) -> Result<Vec<f32>, DatasetError> {
        #[cfg(feature = "inference")]
        {
            let text = std::str::from_utf8(content).map_err(|_| {
                DatasetError::InvalidArgument("server-embed requires UTF-8 text".to_string())
            })?;
            self.embedder
                .as_ref()
                .ok_or(DatasetError::EmbedderUnavailable)?
                .embed(text)
        }
        #[cfg(not(feature = "inference"))]
        {
            let _ = content;
            Err(DatasetError::EmbedderUnavailable)
        }
    }

    /// Embed a query string server-side, or [`DatasetError::EmbedderUnavailable`].
    #[cfg_attr(not(feature = "inference"), allow(clippy::unused_self))]
    fn embed_query(&self, text: &str) -> Result<Vec<f32>, DatasetError> {
        #[cfg(feature = "inference")]
        {
            self.embedder
                .as_ref()
                .ok_or(DatasetError::EmbedderUnavailable)?
                .embed(text)
        }
        #[cfg(not(feature = "inference"))]
        {
            let _ = text;
            Err(DatasetError::EmbedderUnavailable)
        }
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
                doc_count: u64::try_from(s.docs.len()).unwrap_or(u64::MAX),
                dim: s.dim.unwrap_or(0),
                created_ms: s.created_ms,
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
        // Resolve every vector BEFORE taking the lock (embedding may be slow + the
        // embed backend is its own owner thread; never hold the dataset lock across it).
        let mut resolved: Vec<(ContentRef, Vec<u8>, Vec<f32>)> = Vec::with_capacity(docs.len());
        for d in docs {
            if d.content.is_empty() {
                return Err(DatasetError::InvalidArgument(
                    "empty document content".to_string(),
                ));
            }
            let vector = match d.embedding {
                Some(v) if !v.is_empty() => v.to_vec(),
                _ => self.embed_bytes(d.content)?,
            };
            if vector.is_empty() {
                return Err(DatasetError::InvalidArgument(
                    "empty embedding vector".to_string(),
                ));
            }
            if !all_finite(&vector) {
                return Err(DatasetError::InvalidArgument(
                    "embedding must be finite (no NaN/inf)".to_string(),
                ));
            }
            resolved.push((ContentRef::of(d.content), d.content.to_vec(), vector));
        }
        self.inner
            .lock()
            .map_err(|_| DatasetError::Internal("dataset store lock poisoned".to_string()))?
            .ingest_resolved(dataset, resolved)
    }

    fn query(
        &self,
        dataset: &str,
        query_embedding: Option<&[f32]>,
        query_text: &str,
        k: usize,
    ) -> Result<Vec<DatasetHitEntry>, DatasetError> {
        if k == 0 {
            return Ok(Vec::new());
        }
        let k = k.min(MAX_K);
        // Resolve the query vector outside the lock (the client-vector path takes
        // precedence; text-only falls back to the server embedder).
        let qvec = match query_embedding {
            Some(v) if !v.is_empty() => v.to_vec(),
            _ => {
                if query_text.is_empty() {
                    return Err(DatasetError::InvalidArgument(
                        "query requires query_text or a query_embedding".to_string(),
                    ));
                }
                self.embed_query(query_text)?
            }
        };
        if !all_finite(&qvec) {
            return Err(DatasetError::InvalidArgument(
                "query vector must be finite (no NaN/inf)".to_string(),
            ));
        }
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
        let hits = state
            .index
            .query(&qvec, k)
            .into_iter()
            .map(|h| DatasetHitEntry {
                content_ref: *h.id.as_bytes(),
                content: state.docs.get(&h.id).cloned().unwrap_or_default(),
                score: h.score,
            })
            .collect();
        Ok(hits)
    }
}

/// Rebuild every dataset's in-memory state (the HNSW index + the ref→content map)
/// from the durable rows on open.
fn rebuild_state(conn: &Connection) -> Result<HashMap<String, DatasetState>, rusqlite::Error> {
    let mut datasets: HashMap<String, DatasetState> = HashMap::new();
    {
        let mut stmt = conn.prepare("SELECT name, created_ms, dim FROM datasets")?;
        let rows = stmt.query_map([], |row| {
            let name: String = row.get(0)?;
            let created_ms: i64 = row.get(1)?;
            let dim: i64 = row.get(2)?;
            Ok((name, created_ms, dim))
        })?;
        for row in rows {
            let (name, created_ms, dim) = row?;
            let mut state = DatasetState::empty(created_ms);
            if dim > 0 {
                state.dim = u32::try_from(dim).ok();
            }
            datasets.insert(name, state);
        }
    }
    {
        let mut stmt =
            conn.prepare("SELECT dataset, ref, content, vector FROM documents ORDER BY rowid")?;
        let rows = stmt.query_map([], |row| {
            let dataset: String = row.get(0)?;
            let ref_blob: Vec<u8> = row.get(1)?;
            let content: Vec<u8> = row.get(2)?;
            let vec_blob: Vec<u8> = row.get(3)?;
            Ok((dataset, ref_blob, content, vec_blob))
        })?;
        for row in rows {
            let (dataset, ref_blob, content, vec_blob) = row?;
            let Ok(ref_arr) = <[u8; 32]>::try_from(ref_blob.as_slice()) else {
                continue; // a corrupt (non-32B) ref row is skipped, never panics
            };
            let doc_ref = ContentRef::from_bytes(ref_arr);
            let vector = decode_vector_le(&vec_blob);
            let state = datasets
                .entry(dataset.clone())
                .or_insert_with(|| DatasetState::empty(now_ms()));
            if state.dim.is_none() && !vector.is_empty() {
                state.dim = u32::try_from(vector.len()).ok();
            }
            // A row whose stored vector dim disagrees with the dataset's (externally
            // corrupted db only — the ingest path enforces a uniform batch dim) would
            // be SILENTLY skipped by the HNSW insert yet still counted in docs.len().
            // Skip it from BOTH the index and the docs map (+ warn) so doc_count never
            // over-reports retrievable documents.
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
            state.index.insert(doc_ref, vector);
            state.docs.insert(doc_ref, content);
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
        let hits = view.query("corpus", Some(&axis_vec(1)), "", 1).unwrap();
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
        let hits = reopened.query("c", Some(&axis_vec(0)), "", 1).unwrap();
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

    #[test]
    fn unknown_dataset_is_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let view = open_view(dir.path());
        let err = view.query("nope", Some(&axis_vec(0)), "", 1).unwrap_err();
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
        let qerr = view.query("c", None, "hello", 1).unwrap_err();
        // unknown dataset OR embedder-unavailable — both are honest; the embed is
        // attempted first (before the lock), so EmbedderUnavailable wins here.
        assert!(matches!(qerr, DatasetError::EmbedderUnavailable));
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
        let qerr = view.query("c", Some(&inf), "", 1).unwrap_err();
        assert!(matches!(qerr, DatasetError::InvalidArgument(_)));
    }
}
