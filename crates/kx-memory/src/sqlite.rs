// SPDX-License-Identifier: Apache-2.0
//! [`SqliteMemoryStore`] — the durable, rebuildable memory store (`memory.db`).
//!
//! Mirrors the RAG `HostDatasetView` durability posture: the SQLite table is the
//! single source of truth; the per-namespace similarity index + content map are a
//! rebuildable projection rebuilt synchronously on [`SqliteMemoryStore::open`].
//! Writes are **durable-first** (the row is committed before the in-memory index is
//! touched), so the store never has a phantom, queryable-but-unpersisted memory.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::Path;
use std::sync::Mutex;

use kx_content::ContentRef;
use kx_dataset::{InMemoryRetrievalIndex, RetrievalIndex};
use rusqlite::{params, Connection, OptionalExtension};

use crate::error::MemoryError;
use crate::record::{
    all_finite, cosine_sim, decode_vector_le, encode_vector_le, memory_id, now_ms,
    validate_content, validate_namespace, BundleRequest, DecayCandidate, DecayPolicy, DecayReport,
    MemoryHit, MemoryKind, MemoryRecord, MemoryStats, StoreOutcome, StoreRequest,
};
use crate::store::MemoryStore;

// The `memory_decay` table is an ADDITIVE, off-digest sidecar (RC5b): salience
// (`access_count` / `last_accessed_ms`, bumped on recall) + a reversible soft-tombstone
// (`tombstoned_ms`; NULL = live). `CREATE TABLE IF NOT EXISTS` is a no-op on an RC5a
// db (no `ALTER TABLE`, so the `memories` schema stays byte-identical — D40 "rebuild,
// never migrate"). The `memories` row is NEVER deleted by decay ⇒ full reversibility.
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
CREATE INDEX IF NOT EXISTS memories_ns_seq ON memories(namespace, seq);
CREATE TABLE IF NOT EXISTS memory_decay (
    namespace        TEXT NOT NULL,
    memory_id        BLOB NOT NULL,
    access_count     INTEGER NOT NULL DEFAULT 0,
    last_accessed_ms INTEGER NOT NULL DEFAULT 0,
    tombstoned_ms    INTEGER,
    PRIMARY KEY (namespace, memory_id)
);";

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

    /// The recency [`MemoryStore::bundle`] path — newest-first live memories,
    /// optionally restricted to one kind + a `created_ms` window. Kind + window are
    /// integer literals (no injection surface); the row order is authoritative (`seq`).
    fn bundle_recency(
        &self,
        namespace: &str,
        kind: Option<MemoryKind>,
        window_ms: Option<(i64, i64)>,
        limit: usize,
    ) -> Result<Vec<MemoryRecord>, MemoryError> {
        let guard = self.lock()?;
        let map_err = |e: rusqlite::Error| MemoryError::Internal(format!("memory bundle: {e}"));
        let limit_i64 = i64::try_from(limit).unwrap_or(i64::MAX);
        let mut sql = String::from(
            "SELECT m.memory_id, m.content, m.kind, m.instance_id, m.created_ms, length(m.vector) \
             FROM memories m \
             LEFT JOIN memory_decay d ON d.namespace = m.namespace AND d.memory_id = m.memory_id \
             WHERE m.namespace = ?1 AND d.tombstoned_ms IS NULL",
        );
        if let Some(k) = kind {
            let _ = write!(sql, " AND m.kind = {}", k.as_i64());
        }
        if let Some((lo, hi)) = window_ms {
            let _ = write!(sql, " AND m.created_ms BETWEEN {lo} AND {hi}");
        }
        sql.push_str(" ORDER BY m.seq DESC LIMIT ?2");
        let mut stmt = guard.db.prepare(&sql).map_err(map_err)?;
        let mut rows = stmt.query(params![namespace, limit_i64]).map_err(map_err)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().map_err(map_err)? {
            let id_blob: Vec<u8> = row.get(0).map_err(map_err)?;
            let content: Vec<u8> = row.get(1).map_err(map_err)?;
            let kind_i: i64 = row.get(2).map_err(map_err)?;
            let inst_blob: Vec<u8> = row.get(3).map_err(map_err)?;
            let created_ms: i64 = row.get(4).map_err(map_err)?;
            let vec_len: i64 = row.get(5).map_err(map_err)?;
            let Ok(id_arr) = <[u8; 32]>::try_from(id_blob.as_slice()) else {
                continue;
            };
            let instance_id = <[u8; 16]>::try_from(inst_blob.as_slice()).unwrap_or([0u8; 16]);
            out.push(MemoryRecord {
                memory_id: ContentRef::from_bytes(id_arr),
                namespace: namespace.to_string(),
                content,
                kind: MemoryKind::from_i64(kind_i),
                instance_id,
                created_ms,
                dim: u32::try_from(vec_len / 4).unwrap_or(0),
                access_count: 0,
                last_accessed_ms: 0,
                tombstoned_ms: None,
            });
        }
        Ok(out)
    }

    /// The semantic [`MemoryStore::bundle`] path — fetch the live, kind/window-filtered
    /// candidate set (with vectors), re-rank by cosine to `query_vec`, truncate to
    /// `limit`. The similarity score never leaves this function (SN-8).
    fn bundle_semantic(
        &self,
        namespace: &str,
        kind: Option<MemoryKind>,
        query_vec: &[f32],
        window_ms: Option<(i64, i64)>,
        limit: usize,
    ) -> Result<Vec<MemoryRecord>, MemoryError> {
        let guard = self.lock()?;
        // Dim guard against the namespace's fixed dimension (unknown namespace ⇒ empty).
        match guard.ns.get(namespace) {
            None => return Ok(Vec::new()),
            Some(state) => {
                if let Some(dim) = state.dim {
                    if u32::try_from(query_vec.len()).is_ok_and(|n| n != dim) {
                        return Err(MemoryError::DimMismatch(format!(
                            "namespace dim is {dim}, query is {}",
                            query_vec.len()
                        )));
                    }
                }
            }
        }
        let map_err = |e: rusqlite::Error| MemoryError::Internal(format!("memory bundle: {e}"));
        let mut sql = String::from(
            "SELECT m.memory_id, m.content, m.vector, m.kind, m.instance_id, m.created_ms \
             FROM memories m \
             LEFT JOIN memory_decay d ON d.namespace = m.namespace AND d.memory_id = m.memory_id \
             WHERE m.namespace = ?1 AND d.tombstoned_ms IS NULL",
        );
        if let Some(k) = kind {
            let _ = write!(sql, " AND m.kind = {}", k.as_i64());
        }
        if let Some((lo, hi)) = window_ms {
            let _ = write!(sql, " AND m.created_ms BETWEEN {lo} AND {hi}");
        }
        let mut stmt = guard.db.prepare(&sql).map_err(map_err)?;
        let mut rows = stmt.query(params![namespace]).map_err(map_err)?;
        let mut scored: Vec<(f32, MemoryRecord)> = Vec::new();
        while let Some(row) = rows.next().map_err(map_err)? {
            let id_blob: Vec<u8> = row.get(0).map_err(map_err)?;
            let content: Vec<u8> = row.get(1).map_err(map_err)?;
            let vec_blob: Vec<u8> = row.get(2).map_err(map_err)?;
            let kind_i: i64 = row.get(3).map_err(map_err)?;
            let inst_blob: Vec<u8> = row.get(4).map_err(map_err)?;
            let created_ms: i64 = row.get(5).map_err(map_err)?;
            let Ok(id_arr) = <[u8; 32]>::try_from(id_blob.as_slice()) else {
                continue;
            };
            let vector = decode_vector_le(&vec_blob);
            let sim = cosine_sim(query_vec, &vector);
            let instance_id = <[u8; 16]>::try_from(inst_blob.as_slice()).unwrap_or([0u8; 16]);
            scored.push((
                sim,
                MemoryRecord {
                    memory_id: ContentRef::from_bytes(id_arr),
                    namespace: namespace.to_string(),
                    content,
                    kind: MemoryKind::from_i64(kind_i),
                    instance_id,
                    created_ms,
                    dim: u32::try_from(vector.len()).unwrap_or(0),
                    access_count: 0,
                    last_accessed_ms: 0,
                    tombstoned_ms: None,
                },
            ));
        }
        // Deterministic order: cosine desc, then newest-first, then id (total order).
        scored.sort_by(|a, b| {
            b.0.partial_cmp(&a.0)
                .unwrap_or(Ordering::Equal)
                .then(b.1.created_ms.cmp(&a.1.created_ms))
                .then(a.1.memory_id.as_bytes().cmp(b.1.memory_id.as_bytes()))
        });
        Ok(scored.into_iter().take(limit).map(|(_, r)| r).collect())
    }

    /// Deterministic [`MemoryStore::decay`] with an injected `now` (the wall clock is
    /// the only nondeterminism — tests pin it). Soft-tombstones every candidate (the
    /// `memories` row is never deleted); `dry_run` previews without writing.
    pub(crate) fn decay_at(
        &self,
        namespace: &str,
        policy: DecayPolicy,
        now: i64,
    ) -> Result<DecayReport, MemoryError> {
        validate_namespace(namespace)?;
        let mut guard = self.lock()?;
        let Inner { db, ns } = &mut *guard;
        let map_err = |e: rusqlite::Error| MemoryError::Internal(format!("memory decay: {e}"));
        let mut candidates: Vec<DecayCandidate> = Vec::new();
        let mut kept = 0usize;
        {
            let mut stmt = db
                .prepare(
                    "SELECT m.memory_id, m.content, m.kind, m.created_ms, \
                     COALESCE(d.access_count, 0), COALESCE(d.last_accessed_ms, 0) \
                     FROM memories m \
                     LEFT JOIN memory_decay d ON d.namespace = m.namespace AND d.memory_id = m.memory_id \
                     WHERE m.namespace = ?1 AND d.tombstoned_ms IS NULL ORDER BY m.seq",
                )
                .map_err(map_err)?;
            let mut rows = stmt.query(params![namespace]).map_err(map_err)?;
            while let Some(row) = rows.next().map_err(map_err)? {
                let id_blob: Vec<u8> = row.get(0).map_err(map_err)?;
                let content: Vec<u8> = row.get(1).map_err(map_err)?;
                let kind_i: i64 = row.get(2).map_err(map_err)?;
                let created_ms: i64 = row.get(3).map_err(map_err)?;
                let access_i: i64 = row.get(4).map_err(map_err)?;
                let last_accessed_ms: i64 = row.get(5).map_err(map_err)?;
                let Ok(id_arr) = <[u8; 32]>::try_from(id_blob.as_slice()) else {
                    continue;
                };
                let access_count = u32::try_from(access_i).unwrap_or(u32::MAX);
                // Candidate iff OLD (past TTL) AND under-recalled (low salience).
                if now.saturating_sub(created_ms) > policy.ttl_ms
                    && access_count < policy.min_access
                {
                    candidates.push(DecayCandidate {
                        memory_id: ContentRef::from_bytes(id_arr),
                        content,
                        kind: MemoryKind::from_i64(kind_i),
                        created_ms,
                        access_count,
                        last_accessed_ms,
                    });
                } else {
                    kept += 1;
                }
            }
        }
        let swept = if policy.dry_run {
            0
        } else {
            for c in &candidates {
                db.execute(
                    "INSERT INTO memory_decay(namespace, memory_id, access_count, last_accessed_ms, tombstoned_ms)\
                     VALUES(?1, ?2, ?3, ?4, ?5)\
                     ON CONFLICT(namespace, memory_id) DO UPDATE SET tombstoned_ms = ?5",
                    params![
                        namespace,
                        c.memory_id.as_bytes().as_slice(),
                        i64::from(c.access_count),
                        c.last_accessed_ms,
                        now
                    ],
                )
                .map_err(map_err)?;
                if let Some(state) = ns.get_mut(namespace) {
                    state.content.remove(&c.memory_id);
                }
            }
            candidates.len()
        };
        Ok(DecayReport {
            candidates,
            swept,
            kept,
            dry_run: policy.dry_run,
        })
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
    // Re-apply decay tombstones: drop the content of every soft-evicted memory so it
    // is skipped by recall/list after a cold reopen (the durable `memory_decay` sidecar
    // survives the rebuild; the index vector lingers harmlessly, filtered by content).
    let mut tomb = conn
        .prepare("SELECT namespace, memory_id FROM memory_decay WHERE tombstoned_ms IS NOT NULL")?;
    let trows = tomb.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
    })?;
    for row in trows {
        let (namespace, id_blob) = row?;
        let Ok(id_arr) = <[u8; 32]>::try_from(id_blob.as_slice()) else {
            continue;
        };
        if let Some(state) = ns.get_mut(&namespace) {
            state.content.remove(&ContentRef::from_bytes(id_arr));
        }
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
        // Collect the hits in a scope that borrows `state`, so the borrow is dropped
        // before the best-effort salience write below touches `guard.db`.
        let hits: Vec<MemoryHit> = {
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
            state
                .index
                .query(query_vec, k)
                .into_iter()
                .filter_map(|h| {
                    // Skip a forgotten OR decayed memory (its index vector lingers
                    // until the next rebuild, but its content is gone) — never surface
                    // a tombstone.
                    state.content.get(&h.id).map(|content| MemoryHit {
                        memory_id: h.id,
                        content: content.clone(),
                        score: h.score,
                    })
                })
                .collect()
        };
        // Salience (RC5b): bump `access_count` / `last_accessed_ms` for the recalled
        // memories in ONE WAL transaction. Best-effort + error-swallowed — a recall
        // must never fail or slow because the off-digest sidecar write hiccuped.
        if !hits.is_empty() {
            let now = now_ms();
            let _ = guard.db.execute_batch("BEGIN");
            for h in &hits {
                let _ = guard.db.execute(
                    "INSERT INTO memory_decay(namespace, memory_id, access_count, last_accessed_ms) \
                     VALUES(?1, ?2, 1, ?3) \
                     ON CONFLICT(namespace, memory_id) DO UPDATE \
                     SET access_count = access_count + 1, last_accessed_ms = ?3",
                    params![namespace, h.memory_id.as_bytes().as_slice(), now],
                );
            }
            let _ = guard.db.execute_batch("COMMIT");
        }
        Ok(hits)
    }

    fn list(
        &self,
        namespace: &str,
        instance_filter: Option<[u8; 16]>,
        limit: usize,
        include_tombstoned: bool,
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
            let access_count: i64 = row.get(6)?;
            let last_accessed_ms: i64 = row.get(7)?;
            let tombstoned_ms: Option<i64> = row.get(8)?;
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
                access_count: u32::try_from(access_count).unwrap_or(u32::MAX),
                last_accessed_ms,
                tombstoned_ms,
            });
            Ok(())
        };
        let map_err = |e: rusqlite::Error| MemoryError::Internal(format!("memory list: {e}"));
        // LEFT JOIN the off-digest salience/tombstone sidecar; when `include_tombstoned`
        // is false (the default view) hide decayed rows.
        let tomb_clause = if include_tombstoned {
            ""
        } else {
            " AND d.tombstoned_ms IS NULL"
        };
        let base = "SELECT m.memory_id, m.content, m.kind, m.instance_id, m.created_ms, \
             length(m.vector), COALESCE(d.access_count, 0), COALESCE(d.last_accessed_ms, 0), \
             d.tombstoned_ms FROM memories m \
             LEFT JOIN memory_decay d ON d.namespace = m.namespace AND d.memory_id = m.memory_id \
             WHERE m.namespace = ?1";
        if let Some(inst) = instance_filter {
            let sql =
                format!("{base} AND m.instance_id = ?2{tomb_clause} ORDER BY m.seq DESC LIMIT ?3");
            let mut stmt = guard.db.prepare(&sql).map_err(map_err)?;
            let mut rows = stmt
                .query(params![namespace, inst.as_slice(), limit_i64])
                .map_err(map_err)?;
            while let Some(row) = rows.next().map_err(map_err)? {
                push(row).map_err(map_err)?;
            }
        } else {
            let sql = format!("{base}{tomb_clause} ORDER BY m.seq DESC LIMIT ?2");
            let mut stmt = guard.db.prepare(&sql).map_err(map_err)?;
            let mut rows = stmt.query(params![namespace, limit_i64]).map_err(map_err)?;
            while let Some(row) = rows.next().map_err(map_err)? {
                push(row).map_err(map_err)?;
            }
        }
        Ok(out)
    }

    fn bundle(&self, req: BundleRequest<'_>) -> Result<Vec<MemoryRecord>, MemoryError> {
        validate_namespace(req.namespace)?;
        let limit = req.limit.max(1);
        match req.query_vec {
            // Recency path: newest-first, kind/window-filtered, tombstone-excluded.
            None => self.bundle_recency(req.namespace, req.kind, req.window_ms, limit),
            // Semantic path: fetch the live, filtered candidate set + vectors, re-rank
            // by cosine to the query, truncate. The score never leaves this function.
            Some(v) => {
                if v.is_empty() {
                    return Err(MemoryError::InvalidArgument(
                        "bundle query vector must be non-empty".to_string(),
                    ));
                }
                if !all_finite(v) {
                    return Err(MemoryError::InvalidArgument(
                        "bundle query vector has a non-finite (NaN/inf) component".to_string(),
                    ));
                }
                self.bundle_semantic(req.namespace, req.kind, v, req.window_ms, limit)
            }
        }
    }

    fn decay(&self, namespace: &str, policy: DecayPolicy) -> Result<DecayReport, MemoryError> {
        self.decay_at(namespace, policy, now_ms())
    }

    fn decay_all(&self, policy: DecayPolicy) -> Result<usize, MemoryError> {
        let now = now_ms();
        let namespaces = {
            let guard = self.lock()?;
            let mut stmt = guard
                .db
                .prepare("SELECT DISTINCT namespace FROM memories")
                .map_err(|e| MemoryError::Internal(format!("memory decay_all: {e}")))?;
            let rows = stmt
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(|e| MemoryError::Internal(format!("memory decay_all: {e}")))?;
            rows.collect::<Result<Vec<String>, _>>()
                .map_err(|e| MemoryError::Internal(format!("memory decay_all: {e}")))?
        };
        let mut swept = 0usize;
        for ns in namespaces {
            swept += self.decay_at(&ns, policy, now)?.swept;
        }
        Ok(swept)
    }

    fn stats(&self, namespace: &str) -> Result<MemoryStats, MemoryError> {
        let guard = self.lock()?;
        let (dim, fingerprint) = guard.ns.get(namespace).map_or((0, String::new()), |s| {
            (s.dim.unwrap_or(0), s.fingerprint.clone())
        });
        let map_err = |e: rusqlite::Error| MemoryError::Internal(format!("memory stats: {e}"));
        let mut stmt = guard
            .db
            .prepare(
                "SELECT m.kind, m.created_ms, d.tombstoned_ms FROM memories m \
                 LEFT JOIN memory_decay d ON d.namespace = m.namespace AND d.memory_id = m.memory_id \
                 WHERE m.namespace = ?1",
            )
            .map_err(map_err)?;
        let mut rows = stmt.query(params![namespace]).map_err(map_err)?;
        let mut stats = MemoryStats {
            dim,
            fingerprint,
            ..MemoryStats::default()
        };
        let (mut oldest, mut newest) = (i64::MAX, i64::MIN);
        while let Some(row) = rows.next().map_err(map_err)? {
            let kind: i64 = row.get(0).map_err(map_err)?;
            let created: i64 = row.get(1).map_err(map_err)?;
            let tomb: Option<i64> = row.get(2).map_err(map_err)?;
            if tomb.is_some() {
                stats.tombstoned += 1;
                continue;
            }
            match MemoryKind::from_i64(kind) {
                MemoryKind::Semantic => stats.semantic += 1,
                MemoryKind::Episodic => stats.episodic += 1,
            }
            oldest = oldest.min(created);
            newest = newest.max(created);
        }
        stats.total = stats.semantic + stats.episodic;
        if stats.total > 0 {
            stats.oldest_ms = oldest;
            stats.newest_ms = newest;
        }
        Ok(stats)
    }

    fn restore(&self, namespace: &str, mid: &ContentRef) -> Result<bool, MemoryError> {
        let mut guard = self.lock()?;
        let Inner { db, ns } = &mut *guard;
        let affected = db
            .execute(
                "UPDATE memory_decay SET tombstoned_ms = NULL \
                 WHERE namespace = ?1 AND memory_id = ?2 AND tombstoned_ms IS NOT NULL",
                params![namespace, mid.as_bytes().as_slice()],
            )
            .map_err(|e| MemoryError::Internal(format!("memory restore: {e}")))?;
        if affected > 0 {
            // Rehydrate the in-memory content so recall/list surface it again (the
            // durable `memories` row + index vector were never removed by decay).
            let content: Option<Vec<u8>> = db
                .query_row(
                    "SELECT content FROM memories WHERE namespace = ?1 AND memory_id = ?2",
                    params![namespace, mid.as_bytes().as_slice()],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|e| MemoryError::Internal(format!("memory restore read: {e}")))?;
            if let (Some(c), Some(state)) = (content, ns.get_mut(namespace)) {
                state.content.insert(*mid, c);
            }
        }
        Ok(affected > 0)
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
            // Clean the off-digest sidecar row so no orphan salience/tombstone lingers.
            let _ = db.execute(
                "DELETE FROM memory_decay WHERE namespace = ?1 AND memory_id = ?2",
                params![namespace, mid.as_bytes().as_slice()],
            );
            if let Some(state) = ns.get_mut(namespace) {
                // Drop the content so recall skips this memory (the index vector
                // lingers as a harmless tombstone until the next rebuild).
                state.content.remove(mid);
            }
        }
        Ok(affected > 0)
    }
}
