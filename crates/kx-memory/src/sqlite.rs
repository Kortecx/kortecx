// SPDX-License-Identifier: Apache-2.0
//! [`SqliteMemoryStore`] — the durable, rebuildable memory store (`memory.db`).
//!
//! Mirrors the RAG `HostDatasetView` durability posture: the SQLite table is the
//! single source of truth; the per-namespace similarity index + content map are a
//! rebuildable projection rebuilt synchronously on [`SqliteMemoryStore::open`].
//! Writes are **durable-first** (the row is committed before the in-memory index is
//! touched), so the store never has a phantom, queryable-but-unpersisted memory.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use kx_content::ContentRef;
use kx_dataset::{InMemoryRetrievalIndex, RetrievalIndex};
use rusqlite::{params, Connection};

use crate::error::MemoryError;
use crate::record::{
    all_finite, decode_vector_le, encode_vector_le, memory_id, validate_content,
    validate_namespace, MemoryHit, MemoryKind, MemoryRecord, StoreOutcome, StoreRequest,
};
use crate::store::MemoryStore;

const SCHEMA: &str = "\
CREATE TABLE IF NOT EXISTS memories (
    seq         INTEGER PRIMARY KEY AUTOINCREMENT,
    namespace   TEXT NOT NULL,
    memory_id   BLOB NOT NULL,
    content     BLOB NOT NULL,
    vector      BLOB NOT NULL,
    kind        INTEGER NOT NULL DEFAULT 0,
    instance_id BLOB NOT NULL,
    created_ms  INTEGER NOT NULL,
    fingerprint TEXT NOT NULL DEFAULT '',
    UNIQUE (namespace, memory_id)
);
CREATE INDEX IF NOT EXISTS memories_ns_seq ON memories(namespace, seq);";

/// One namespace's rebuildable in-memory projection: the exact similarity index
/// (deterministic; the `kx-dataset-hnsw` HNSW index is a drop-in for scale behind
/// the same [`RetrievalIndex`] trait), the id→content map for hit snippets, the
/// fixed embedding dimension, and the embed-model fingerprint the vectors share.
struct NamespaceState {
    dim: Option<u32>,
    fingerprint: String,
    index: InMemoryRetrievalIndex,
    content: HashMap<ContentRef, Vec<u8>>,
}

impl NamespaceState {
    fn empty() -> Self {
        Self {
            dim: None,
            fingerprint: String::new(),
            index: InMemoryRetrievalIndex::new(),
            content: HashMap::new(),
        }
    }
}

struct Inner {
    db: Connection,
    ns: HashMap<String, NamespaceState>,
}

/// The durable memory store. Interior-mutable (`Mutex<Inner>`) so it can be shared
/// as an `Arc<dyn MemoryStore>` by the read-only recall / write remember capabilities.
pub struct SqliteMemoryStore {
    inner: Mutex<Inner>,
}

impl SqliteMemoryStore {
    /// Open (or create) `memory.db` under `dir`, rebuilding the per-namespace
    /// indices from the durable rows before returning (no memory is recallable
    /// before its index is warm).
    ///
    /// # Errors
    /// [`MemoryError::Internal`] if the directory / db / schema cannot be opened.
    pub fn open(dir: &Path) -> Result<Self, MemoryError> {
        std::fs::create_dir_all(dir)
            .map_err(|e| MemoryError::Internal(format!("memory dir: {e}")))?;
        let db_path = dir.join("memory.db");
        let conn = Connection::open(&db_path)
            .map_err(|e| MemoryError::Internal(format!("memory db: {e}")))?;
        Self::from_conn(conn)
    }

    /// Open an in-memory `memory.db` (tests / ephemeral use).
    ///
    /// # Errors
    /// [`MemoryError::Internal`] if the db / schema cannot be opened.
    pub fn open_ephemeral() -> Result<Self, MemoryError> {
        let conn = Connection::open_in_memory()
            .map_err(|e| MemoryError::Internal(format!("memory db: {e}")))?;
        Self::from_conn(conn)
    }

    fn from_conn(conn: Connection) -> Result<Self, MemoryError> {
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = FULL;",
        )
        .map_err(|e| MemoryError::Internal(format!("memory pragma: {e}")))?;
        conn.execute_batch(SCHEMA)
            .map_err(|e| MemoryError::Internal(format!("memory schema: {e}")))?;
        let ns =
            rebuild(&conn).map_err(|e| MemoryError::Internal(format!("memory rebuild: {e}")))?;
        Ok(Self {
            inner: Mutex::new(Inner { db: conn, ns }),
        })
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Inner>, MemoryError> {
        self.inner
            .lock()
            .map_err(|_| MemoryError::Internal("memory store lock poisoned".to_string()))
    }
}

/// Rebuild every namespace's in-memory projection from the durable rows (oldest
/// first, so the fixed dim/fingerprint come from the first-stored memory). A
/// corrupt (non-32B ref, dim-mismatched) row is skipped, never a panic — the
/// D40 "rebuild, never migrate" guarantee.
fn rebuild(conn: &Connection) -> Result<HashMap<String, NamespaceState>, rusqlite::Error> {
    let mut ns: HashMap<String, NamespaceState> = HashMap::new();
    let mut stmt = conn.prepare(
        "SELECT namespace, memory_id, content, vector, fingerprint FROM memories ORDER BY seq",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Vec<u8>>(1)?,
            row.get::<_, Vec<u8>>(2)?,
            row.get::<_, Vec<u8>>(3)?,
            row.get::<_, String>(4)?,
        ))
    })?;
    for row in rows {
        let (namespace, id_blob, content, vec_blob, fp) = row?;
        let Ok(id_arr) = <[u8; 32]>::try_from(id_blob.as_slice()) else {
            continue; // a corrupt (non-32B) id row is skipped
        };
        let mid = ContentRef::from_bytes(id_arr);
        let vector = decode_vector_le(&vec_blob);
        let state = ns.entry(namespace).or_insert_with(NamespaceState::empty);
        if state.dim.is_none() && !vector.is_empty() {
            state.dim = u32::try_from(vector.len()).ok();
            state.fingerprint = fp;
        }
        // Skip a row whose stored dim disagrees with the namespace's (a corrupt db;
        // the store path enforces a uniform dim) so counts never over-report.
        let dim_ok = state
            .dim
            .is_none_or(|d| u32::try_from(vector.len()).is_ok_and(|n| n == d));
        if !dim_ok {
            continue;
        }
        state.index.insert(mid, vector);
        state.content.insert(mid, content);
    }
    Ok(ns)
}

impl MemoryStore for SqliteMemoryStore {
    fn store(&self, req: StoreRequest<'_>) -> Result<StoreOutcome, MemoryError> {
        validate_namespace(req.namespace)?;
        validate_content(req.content)?;
        if req.vector.is_empty() {
            return Err(MemoryError::InvalidArgument(
                "memory vector must be non-empty (embed the content first)".to_string(),
            ));
        }
        if !all_finite(req.vector) {
            return Err(MemoryError::InvalidArgument(
                "memory vector has a non-finite (NaN/inf) component".to_string(),
            ));
        }
        let mid = memory_id(req.content);
        let dim = u32::try_from(req.vector.len())
            .map_err(|_| MemoryError::InvalidArgument("vector too long".to_string()))?;

        let mut guard = self.lock()?;
        let Inner { db, ns } = &mut *guard;
        let state = ns
            .entry(req.namespace.to_string())
            .or_insert_with(NamespaceState::empty);

        // Vector-space guards: a namespace has ONE dim + ONE embed fingerprint.
        if let Some(existing) = state.dim {
            if existing != dim {
                return Err(MemoryError::DimMismatch(format!(
                    "namespace dim is {existing}, vector is {dim}"
                )));
            }
            if !req.embed_fingerprint.is_empty()
                && !state.fingerprint.is_empty()
                && state.fingerprint != req.embed_fingerprint
            {
                return Err(MemoryError::StaleIndex(format!(
                    "namespace indexed under a different embed model ({} != {}); \
                     forget + re-remember to rebuild",
                    state.fingerprint, req.embed_fingerprint
                )));
            }
        }

        // Durable-first: commit the row BEFORE the in-memory index is touched.
        let affected = db
            .execute(
                "INSERT OR IGNORE INTO memories\
                 (namespace, memory_id, content, vector, kind, instance_id, created_ms, fingerprint)\
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    req.namespace,
                    mid.as_bytes().as_slice(),
                    req.content,
                    encode_vector_le(req.vector).as_slice(),
                    req.kind.as_i64(),
                    req.instance_id.as_slice(),
                    req.created_ms,
                    req.embed_fingerprint,
                ],
            )
            .map_err(|e| MemoryError::Internal(format!("memory insert: {e}")))?;
        let inserted = affected > 0;
        if inserted {
            state.index.insert(mid, req.vector.to_vec());
            state.content.insert(mid, req.content.to_vec());
            if state.dim.is_none() {
                state.dim = Some(dim);
                state.fingerprint = req.embed_fingerprint.to_string();
            }
        }
        Ok(StoreOutcome {
            memory_id: mid,
            inserted,
            dim: state.dim.unwrap_or(dim),
        })
    }

    fn recall(
        &self,
        namespace: &str,
        query_vec: &[f32],
        k: usize,
        embed_fingerprint: &str,
    ) -> Result<Vec<MemoryHit>, MemoryError> {
        if query_vec.is_empty() {
            return Err(MemoryError::InvalidArgument(
                "query vector must be non-empty".to_string(),
            ));
        }
        if !all_finite(query_vec) {
            return Err(MemoryError::InvalidArgument(
                "query vector has a non-finite (NaN/inf) component".to_string(),
            ));
        }
        let guard = self.lock()?;
        let Some(state) = guard.ns.get(namespace) else {
            return Ok(Vec::new()); // unknown/empty namespace ⇒ honest empty
        };
        if let Some(dim) = state.dim {
            if u32::try_from(query_vec.len()).is_ok_and(|n| n != dim) {
                return Err(MemoryError::DimMismatch(format!(
                    "namespace dim is {dim}, query is {}",
                    query_vec.len()
                )));
            }
            if !embed_fingerprint.is_empty()
                && !state.fingerprint.is_empty()
                && state.fingerprint != embed_fingerprint
            {
                return Err(MemoryError::StaleIndex(format!(
                    "namespace indexed under a different embed model ({} != {})",
                    state.fingerprint, embed_fingerprint
                )));
            }
        }
        let hits = state
            .index
            .query(query_vec, k)
            .into_iter()
            .filter_map(|h| {
                // Skip a forgotten memory (its index vector lingers until the next
                // rebuild, but its content is gone) — never surface a tombstone.
                state.content.get(&h.id).map(|content| MemoryHit {
                    memory_id: h.id,
                    content: content.clone(),
                    score: h.score,
                })
            })
            .collect();
        Ok(hits)
    }

    fn list(
        &self,
        namespace: &str,
        instance_filter: Option<[u8; 16]>,
        limit: usize,
    ) -> Result<Vec<MemoryRecord>, MemoryError> {
        let guard = self.lock()?;
        let limit_i64 = i64::try_from(limit.max(1)).unwrap_or(i64::MAX);
        let mut out = Vec::new();
        let mut push = |row: &rusqlite::Row<'_>| -> rusqlite::Result<()> {
            let id_blob: Vec<u8> = row.get(0)?;
            let content: Vec<u8> = row.get(1)?;
            let kind: i64 = row.get(2)?;
            let inst_blob: Vec<u8> = row.get(3)?;
            let created_ms: i64 = row.get(4)?;
            let vec_len: i64 = row.get(5)?;
            let Ok(id_arr) = <[u8; 32]>::try_from(id_blob.as_slice()) else {
                return Ok(()); // skip a corrupt id row
            };
            let instance_id = <[u8; 16]>::try_from(inst_blob.as_slice()).unwrap_or([0u8; 16]);
            out.push(MemoryRecord {
                memory_id: ContentRef::from_bytes(id_arr),
                namespace: namespace.to_string(),
                content,
                kind: MemoryKind::from_i64(kind),
                instance_id,
                created_ms,
                dim: u32::try_from(vec_len / 4).unwrap_or(0),
            });
            Ok(())
        };
        let map_err = |e: rusqlite::Error| MemoryError::Internal(format!("memory list: {e}"));
        if let Some(inst) = instance_filter {
            let mut stmt = guard
                .db
                .prepare(
                    "SELECT memory_id, content, kind, instance_id, created_ms, length(vector)\
                     FROM memories WHERE namespace = ?1 AND instance_id = ?2\
                     ORDER BY seq DESC LIMIT ?3",
                )
                .map_err(map_err)?;
            let mut rows = stmt
                .query(params![namespace, inst.as_slice(), limit_i64])
                .map_err(map_err)?;
            while let Some(row) = rows.next().map_err(map_err)? {
                push(row).map_err(map_err)?;
            }
        } else {
            let mut stmt = guard
                .db
                .prepare(
                    "SELECT memory_id, content, kind, instance_id, created_ms, length(vector)\
                     FROM memories WHERE namespace = ?1 ORDER BY seq DESC LIMIT ?2",
                )
                .map_err(map_err)?;
            let mut rows = stmt.query(params![namespace, limit_i64]).map_err(map_err)?;
            while let Some(row) = rows.next().map_err(map_err)? {
                push(row).map_err(map_err)?;
            }
        }
        Ok(out)
    }

    fn forget(&self, namespace: &str, mid: &ContentRef) -> Result<bool, MemoryError> {
        let mut guard = self.lock()?;
        let Inner { db, ns } = &mut *guard;
        let affected = db
            .execute(
                "DELETE FROM memories WHERE namespace = ?1 AND memory_id = ?2",
                params![namespace, mid.as_bytes().as_slice()],
            )
            .map_err(|e| MemoryError::Internal(format!("memory forget: {e}")))?;
        if affected > 0 {
            if let Some(state) = ns.get_mut(namespace) {
                // Drop the content so recall skips this memory (the index vector
                // lingers as a harmless tombstone until the next rebuild).
                state.content.remove(mid);
            }
        }
        Ok(affected > 0)
    }
}
