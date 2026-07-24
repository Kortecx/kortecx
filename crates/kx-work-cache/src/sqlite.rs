//! `SqliteWorkCache` — the durable [`WorkCache`] backend, one per serve.
//!
//! A single `work-cache.db` sidecar (SQLite via `rusqlite`, bundled) shared across
//! every run in a serve. It is a **rebuildable, non-authoritative projection**: a
//! lost/corrupt/absent file costs only recomputation, never correctness, so it opens
//! with `synchronous = NORMAL` (not the journal's `FULL`) — durability is not required
//! of a cache. All state is a flat key→ref table; there is no schema-version gate
//! because a shape mismatch is handled by discard-and-rebuild, not migration.

use std::path::Path;
use std::sync::Mutex;

use kx_content::ContentRef;
use kx_mote::{MoteId, NdClass};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};

use crate::{WorkCache, WorkFingerprint};

/// Errors from the durable work-cache backend. Surfaced only from [`SqliteWorkCache::open`]
/// and the fallible internal helpers; the [`WorkCache`] trait methods swallow write
/// errors by design (a cache must never break a run).
#[derive(Debug, thiserror::Error)]
pub enum WorkCacheError {
    /// The underlying SQLite call failed.
    #[error("work-cache sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

/// A durable, first-writer-wins [`WorkCache`] backed by a SQLite sidecar.
#[derive(Debug)]
pub struct SqliteWorkCache {
    conn: Mutex<Connection>,
}

impl SqliteWorkCache {
    /// Open or create the cache at `path`. Creates the schema on first use.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, WorkCacheError> {
        let conn = Connection::open_with_flags(
            path.as_ref(),
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        Self::configure(&conn)?;
        Self::initialize(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Open an in-memory cache (tests).
    pub fn open_in_memory() -> Result<Self, WorkCacheError> {
        let conn = Connection::open_in_memory()?;
        Self::configure(&conn)?;
        Self::initialize(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// PRAGMA configuration. `synchronous = NORMAL` (not `FULL`) — this is a
    /// rebuildable cache, so we trade a vanishingly small crash-loss window (a few
    /// unsynced inserts that would simply be recomputed) for throughput.
    fn configure(conn: &Connection) -> Result<(), WorkCacheError> {
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA temp_store  = MEMORY;",
        )?;
        Ok(())
    }

    /// Create the schema (idempotent).
    fn initialize(conn: &Connection) -> Result<(), WorkCacheError> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS work_cache (
                 work_fingerprint BLOB PRIMARY KEY,  -- 32-byte run-independent key
                 result_ref       BLOB NOT NULL,     -- 32-byte ContentRef
                 nd_class         INTEGER NOT NULL,  -- 0=Pure, 1=ReadOnlyNondet (never 2=WM)
                 source_mote_id   BLOB NOT NULL       -- 32-byte provenance
             );",
        )?;
        Ok(())
    }

    /// Fallible lookup (the trait method wraps this and maps errors to a miss).
    fn try_lookup(&self, fp: &WorkFingerprint) -> Result<Option<ContentRef>, WorkCacheError> {
        let conn = self
            .conn
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let row: Option<Vec<u8>> = conn
            .query_row(
                "SELECT result_ref FROM work_cache WHERE work_fingerprint = ?1",
                params![&fp.as_bytes()[..]],
                |r| r.get(0),
            )
            .optional()?;
        Ok(row.and_then(|bytes| bytes.try_into().ok().map(ContentRef::from_bytes)))
    }

    /// Fallible insert (first-writer-wins via `INSERT OR IGNORE`).
    fn try_insert(
        &self,
        fp: WorkFingerprint,
        result_ref: ContentRef,
        nd: NdClass,
        source: MoteId,
    ) -> Result<(), WorkCacheError> {
        let conn = self
            .conn
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        conn.execute(
            "INSERT OR IGNORE INTO work_cache
                 (work_fingerprint, result_ref, nd_class, source_mote_id)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                &fp.as_bytes()[..],
                &result_ref.as_bytes()[..],
                i64::from(nd_class_to_u8(nd)),
                &source.as_bytes()[..],
            ],
        )?;
        Ok(())
    }

    /// Fallible evict.
    fn try_evict(&self, fp: &WorkFingerprint) -> Result<(), WorkCacheError> {
        let conn = self
            .conn
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        conn.execute(
            "DELETE FROM work_cache WHERE work_fingerprint = ?1",
            params![&fp.as_bytes()[..]],
        )?;
        Ok(())
    }
}

/// Stable `NdClass → u8` mapping (mirrors the journal header discriminants) without
/// relying on the enum's `repr`.
const fn nd_class_to_u8(nd: NdClass) -> u8 {
    match nd {
        NdClass::Pure => 0,
        NdClass::ReadOnlyNondet => 1,
        NdClass::WorldMutating => 2,
    }
}

impl WorkCache for SqliteWorkCache {
    fn lookup(&self, fp: &WorkFingerprint) -> Option<ContentRef> {
        match self.try_lookup(fp) {
            Ok(hit) => hit,
            Err(e) => {
                // A read error degrades to a miss → recompute. Never fatal.
                tracing::warn!(error = %e, "work-cache lookup failed; treating as miss");
                None
            }
        }
    }

    fn insert(&self, fp: WorkFingerprint, result_ref: ContentRef, nd: NdClass, source: MoteId) {
        if let Err(e) = self.try_insert(fp, result_ref, nd, source) {
            tracing::warn!(error = %e, "work-cache insert failed; skipping (run unaffected)");
        }
    }

    fn evict(&self, fp: &WorkFingerprint) {
        if let Err(e) = self.try_evict(fp) {
            tracing::warn!(error = %e, "work-cache evict failed; stale entry may persist");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kx_mote::{InputDataId, MoteDefHash};

    fn fp(b: u8) -> WorkFingerprint {
        crate::work_fingerprint(
            NdClass::Pure,
            &MoteDefHash::from_bytes([b; 32]),
            &InputDataId::from_bytes([b; 32]),
        )
    }

    #[test]
    fn round_trip() {
        let cache = SqliteWorkCache::open_in_memory().unwrap();
        let r = ContentRef::of(b"result");
        assert!(cache.lookup(&fp(1)).is_none());
        cache.insert(fp(1), r, NdClass::Pure, MoteId::from_bytes([7; 32]));
        assert_eq!(cache.lookup(&fp(1)), Some(r));
    }

    #[test]
    fn first_writer_wins() {
        let cache = SqliteWorkCache::open_in_memory().unwrap();
        let first = ContentRef::of(b"first");
        let second = ContentRef::of(b"second");
        cache.insert(fp(1), first, NdClass::Pure, MoteId::from_bytes([0; 32]));
        cache.insert(fp(1), second, NdClass::Pure, MoteId::from_bytes([1; 32]));
        assert_eq!(cache.lookup(&fp(1)), Some(first));
    }

    #[test]
    fn evict_removes() {
        let cache = SqliteWorkCache::open_in_memory().unwrap();
        let r = ContentRef::of(b"result");
        cache.insert(fp(1), r, NdClass::Pure, MoteId::from_bytes([0; 32]));
        cache.evict(&fp(1));
        assert!(cache.lookup(&fp(1)).is_none());
    }

    #[test]
    fn reopen_persists() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("work-cache.db");
        let r = ContentRef::of(b"durable");
        {
            let cache = SqliteWorkCache::open(&path).unwrap();
            cache.insert(fp(3), r, NdClass::Pure, MoteId::from_bytes([0; 32]));
        }
        let reopened = SqliteWorkCache::open(&path).unwrap();
        assert_eq!(reopened.lookup(&fp(3)), Some(r));
    }
}
