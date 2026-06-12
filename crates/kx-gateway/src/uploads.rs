//! The Batch A uploads sidecar: `uploads.db` under `--catalog-dir`, backing the
//! [`UploadsLedger`] seam — `PutContent`'s advisory audit rows + the EMPTY-
//! `instance_id` ("uploads scope") authorized set on `GetContent`/`GetContentBatch`.
//!
//! ## Rebuildable-to-EMPTY (the honest difference vs `capture.db`)
//! Capture is journal-DERIVED — a corrupt sidecar rebuilds from the journal.
//! Uploads are NOT derivable from anything: the rows record client uploads that
//! never touch the journal. Truth (the blobs) lives in the content store, so on
//! corruption or a schema-version drift this ledger recreates EMPTY: the only
//! loss is the uploads-scope authorization index + advisory audit rows, and a
//! re-upload of the same bytes restores authorization at the SAME ref
//! (content-addressed). Never journaled, never a `MoteId` input, never a digest
//! input — dropping the file cannot move the canonical projection digest.
//!
//! ## Advisory metadata only
//! `media_type`/`filename` are display/audit fields (SN-8: identity is the
//! server-derived blake3 ref alone). `INSERT OR REPLACE` keeps a re-upload
//! idempotent — the latest advisory metadata wins.

use std::path::Path;
use std::sync::Mutex;

use kx_gateway_core::GatewayError as CoreError;
use kx_gateway_core::{UploadRecord, UploadsLedger};
use rusqlite::{params, Connection};

use crate::error::GatewayError;

/// Bump on any table-shape change. Unknown/missing version ⇒ recreate EMPTY
/// (see the module doc — uploads are not derivable, so there is no rebuild).
const SCHEMA_VERSION: i64 = 1;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS uploads (
    content_ref BLOB PRIMARY KEY,   -- 32B server-derived blake3
    media_type  TEXT NOT NULL,      -- advisory (display/audit)
    filename    TEXT NOT NULL,      -- advisory (display/audit)
    principal   TEXT NOT NULL,      -- server-resolved caller party
    uploaded_ms INTEGER NOT NULL    -- wall clock, audit only
);
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value INTEGER NOT NULL);
";

/// The durable uploads ledger over `uploads.db`. A single mutex'd connection:
/// uploads are interactive-rate (a chat attach, a CLI put), never contended.
pub(crate) struct UploadsDb {
    conn: Mutex<Connection>,
}

impl UploadsDb {
    /// Open (or create) `uploads.db` under `dir`. A corrupt/foreign file or a
    /// `schema_version` drift recreates the ledger EMPTY (module doc).
    ///
    /// # Errors
    /// [`GatewayError::Catalog`] on an unrecoverable open/pragma failure.
    pub(crate) fn open(dir: &Path) -> Result<Self, GatewayError> {
        std::fs::create_dir_all(dir)
            .map_err(|e| GatewayError::Catalog(format!("uploads dir: {e}")))?;
        let db_path = dir.join("uploads.db");
        // A non-SQLite file fails even the pragma — delete + recreate (the rows
        // are advisory audit state; the blobs in the content store are truth).
        let conn = if let Ok(c) = Self::open_with_pragma(&db_path) {
            c
        } else {
            let _ = std::fs::remove_file(&db_path);
            let _ = std::fs::remove_file(dir.join("uploads.db-wal"));
            let _ = std::fs::remove_file(dir.join("uploads.db-shm"));
            Self::open_with_pragma(&db_path)
                .map_err(|e| GatewayError::Catalog(format!("uploads reopen: {e}")))?
        };
        let fresh_or_stale = match Self::read_schema_version(&conn) {
            Ok(Some(v)) => v != SCHEMA_VERSION,
            Ok(None) | Err(_) => true,
        };
        if fresh_or_stale {
            conn.execute_batch(
                "DROP TABLE IF EXISTS uploads;
                 DROP TABLE IF EXISTS meta;",
            )
            .map_err(|e| GatewayError::Catalog(format!("uploads rebuild: {e}")))?;
        }
        conn.execute_batch(SCHEMA)
            .map_err(|e| GatewayError::Catalog(format!("uploads schema: {e}")))?;
        conn.execute(
            "INSERT OR IGNORE INTO meta(key, value) VALUES ('schema_version', ?1)",
            params![SCHEMA_VERSION],
        )
        .map_err(|e| GatewayError::Catalog(format!("uploads meta init: {e}")))?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn open_with_pragma(db_path: &Path) -> rusqlite::Result<Connection> {
        let conn = Connection::open(db_path)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;",
        )?;
        Ok(conn)
    }

    fn read_schema_version(conn: &Connection) -> rusqlite::Result<Option<i64>> {
        conn.query_row(
            "SELECT value FROM meta WHERE key = 'schema_version'",
            [],
            |r| r.get(0),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })
    }
}

impl UploadsLedger for UploadsDb {
    fn record(&self, rec: UploadRecord) -> Result<(), CoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| CoreError::Internal("uploads lock poisoned".into()))?;
        conn.execute(
            "INSERT OR REPLACE INTO uploads(content_ref, media_type, filename, principal, uploaded_ms) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                rec.content_ref.to_vec(),
                rec.media_type,
                rec.filename,
                rec.principal,
                i64::try_from(rec.uploaded_ms).unwrap_or(i64::MAX),
            ],
        )
        .map_err(|e| CoreError::Internal(format!("uploads record: {e}")))?;
        Ok(())
    }

    fn contains(&self, content_ref: &[u8; 32]) -> Result<bool, CoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| CoreError::Internal("uploads lock poisoned".into()))?;
        let found: Option<i64> = conn
            .query_row(
                "SELECT 1 FROM uploads WHERE content_ref = ?1",
                params![content_ref.to_vec()],
                |r| r.get(0),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })
            .map_err(|e| CoreError::Internal(format!("uploads contains: {e}")))?;
        Ok(found.is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(tag: u8) -> UploadRecord {
        UploadRecord {
            content_ref: [tag; 32],
            media_type: "image/png".into(),
            filename: format!("file-{tag}.png"),
            principal: "tester".into(),
            uploaded_ms: 1_000 + u64::from(tag),
        }
    }

    #[test]
    fn record_then_contains_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let db = UploadsDb::open(dir.path()).unwrap();
        assert!(!db.contains(&[0x42; 32]).unwrap());
        db.record(rec(0x42)).unwrap();
        assert!(db.contains(&[0x42; 32]).unwrap());
        // Idempotent re-record (latest advisory metadata wins, no error).
        db.record(rec(0x42)).unwrap();
        assert!(db.contains(&[0x42; 32]).unwrap());
    }

    #[test]
    fn reopen_preserves_rows() {
        let dir = tempfile::tempdir().unwrap();
        {
            let db = UploadsDb::open(dir.path()).unwrap();
            db.record(rec(0x11)).unwrap();
        }
        let db = UploadsDb::open(dir.path()).unwrap();
        assert!(db.contains(&[0x11; 32]).unwrap(), "rows survive a restart");
    }

    #[test]
    fn corrupt_file_recreates_empty() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("uploads.db"), b"not a sqlite file").unwrap();
        let db = UploadsDb::open(dir.path()).unwrap();
        assert!(
            !db.contains(&[0x11; 32]).unwrap(),
            "a corrupt sidecar recreates EMPTY (uploads are not derivable)"
        );
        // And it works after the recreate.
        db.record(rec(0x11)).unwrap();
        assert!(db.contains(&[0x11; 32]).unwrap());
    }

    #[test]
    fn schema_version_drift_recreates_empty() {
        let dir = tempfile::tempdir().unwrap();
        {
            let db = UploadsDb::open(dir.path()).unwrap();
            db.record(rec(0x22)).unwrap();
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "UPDATE meta SET value = 999 WHERE key = 'schema_version'",
                [],
            )
            .unwrap();
        }
        let db = UploadsDb::open(dir.path()).unwrap();
        assert!(
            !db.contains(&[0x22; 32]).unwrap(),
            "version drift drops the advisory rows (re-upload re-authorizes)"
        );
    }
}
