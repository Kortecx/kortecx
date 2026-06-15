//! The PR-D run-inputs sidecar: `run_inputs.db` under `--catalog-dir`, backing
//! the [`RunInputsStore`] seam — the `Invoke` args captured at submit so a run
//! recovered from `ListRuns` (with no client-side localStorage) can pre-fill its
//! recipe form and be re-invoked with edited params ("Re-run with changes").
//!
//! ## Rebuildable-to-EMPTY (the uploads.db posture)
//! The captured args are NOT derivable from anything — they record the client's
//! `Invoke` input, which never touches the journal. Truth (the committed run +
//! its results) lives in the journal/content store, so on corruption or a
//! schema-version drift this sidecar recreates EMPTY: the only loss is the
//! re-run pre-fill convenience (older runs then have no captured args and the
//! form opens blank). Never journaled, never a `MoteId` input, never a digest
//! input — dropping the file cannot move the canonical projection digest.
//!
//! ## Keyed by `instance_id`, latest-args-win
//! `kx serve` shares one journal, so all invokes share one `instance_id`
//! (one-run-per-journal). `INSERT OR REPLACE` keeps capture idempotent — the
//! latest invoke's args win per run, which is the sensible pre-fill default.
//! `principal` is a server-resolved audit field (SN-8), never a read filter.

use std::path::Path;
use std::sync::Mutex;

use kx_gateway_core::GatewayError as CoreError;
use kx_gateway_core::{RunInputsEntry, RunInputsRecord, RunInputsStore};
use rusqlite::{params, Connection};

use crate::error::GatewayError;

/// Bump on any table-shape change. Unknown/missing version ⇒ recreate EMPTY
/// (see the module doc — captured args are not derivable, so there is no rebuild).
const SCHEMA_VERSION: i64 = 1;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS run_inputs (
    instance_id        BLOB PRIMARY KEY,   -- 16B run id (the ListRuns recovery key)
    recipe_fingerprint BLOB NOT NULL,      -- 32B recipe identity (advisory)
    handle             TEXT NOT NULL,      -- the Invoke handle (for GetRecipeForm)
    args               BLOB NOT NULL,      -- opaque JSON object bytes (the Invoke args)
    principal          TEXT NOT NULL,      -- server-resolved caller party (audit only)
    captured_ms        INTEGER NOT NULL    -- wall clock, audit only
);
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value INTEGER NOT NULL);
";

/// The durable run-inputs sidecar over `run_inputs.db`. A single mutex'd
/// connection: captures are interactive-rate (one per `Invoke`), never contended.
pub(crate) struct RunInputsDb {
    conn: Mutex<Connection>,
}

impl RunInputsDb {
    /// Open (or create) `run_inputs.db` under `dir`. A corrupt/foreign file or a
    /// `schema_version` drift recreates the sidecar EMPTY (module doc).
    ///
    /// # Errors
    /// [`GatewayError::Catalog`] on an unrecoverable open/pragma failure.
    pub(crate) fn open(dir: &Path) -> Result<Self, GatewayError> {
        std::fs::create_dir_all(dir)
            .map_err(|e| GatewayError::Catalog(format!("run_inputs dir: {e}")))?;
        let db_path = dir.join("run_inputs.db");
        // A non-SQLite file fails even the pragma — delete + recreate (the rows
        // are convenience capture; the run itself lives in the journal).
        let conn = if let Ok(c) = Self::open_with_pragma(&db_path) {
            c
        } else {
            let _ = std::fs::remove_file(&db_path);
            let _ = std::fs::remove_file(dir.join("run_inputs.db-wal"));
            let _ = std::fs::remove_file(dir.join("run_inputs.db-shm"));
            Self::open_with_pragma(&db_path)
                .map_err(|e| GatewayError::Catalog(format!("run_inputs reopen: {e}")))?
        };
        let fresh_or_stale = match Self::read_schema_version(&conn) {
            Ok(Some(v)) => v != SCHEMA_VERSION,
            Ok(None) | Err(_) => true,
        };
        if fresh_or_stale {
            conn.execute_batch(
                "DROP TABLE IF EXISTS run_inputs;
                 DROP TABLE IF EXISTS meta;",
            )
            .map_err(|e| GatewayError::Catalog(format!("run_inputs rebuild: {e}")))?;
        }
        conn.execute_batch(SCHEMA)
            .map_err(|e| GatewayError::Catalog(format!("run_inputs schema: {e}")))?;
        conn.execute(
            "INSERT OR IGNORE INTO meta(key, value) VALUES ('schema_version', ?1)",
            params![SCHEMA_VERSION],
        )
        .map_err(|e| GatewayError::Catalog(format!("run_inputs meta init: {e}")))?;
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

/// Map a sqlite BLOB column to a fixed `[u8; N]` (our own writes are always the
/// right length; a malformed row ⇒ `Internal`).
fn fixed<const N: usize>(v: &[u8]) -> Result<[u8; N], CoreError> {
    <[u8; N]>::try_from(v)
        .map_err(|_| CoreError::Internal(format!("run_inputs row: expected {N}-byte blob")))
}

impl RunInputsStore for RunInputsDb {
    fn record(&self, rec: RunInputsRecord) -> Result<(), CoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| CoreError::Internal("run_inputs lock poisoned".into()))?;
        conn.execute(
            "INSERT OR REPLACE INTO run_inputs(instance_id, recipe_fingerprint, handle, args, \
             principal, captured_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                rec.instance_id.to_vec(),
                rec.recipe_fingerprint.to_vec(),
                rec.handle,
                rec.args,
                rec.principal,
                i64::try_from(rec.captured_unix_ms).unwrap_or(i64::MAX),
            ],
        )
        .map_err(|e| CoreError::Internal(format!("run_inputs record: {e}")))?;
        Ok(())
    }

    fn get(&self, instance_id: &[u8; 16]) -> Result<Option<RunInputsEntry>, CoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| CoreError::Internal("run_inputs lock poisoned".into()))?;
        let row: Option<(Vec<u8>, String, Vec<u8>)> = conn
            .query_row(
                "SELECT recipe_fingerprint, handle, args FROM run_inputs WHERE instance_id = ?1",
                params![instance_id.to_vec()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })
            .map_err(|e| CoreError::Internal(format!("run_inputs get: {e}")))?;
        match row {
            None => Ok(None),
            Some((fp, handle, args)) => Ok(Some(RunInputsEntry {
                instance_id: *instance_id,
                recipe_fingerprint: fixed::<32>(&fp)?,
                handle,
                args,
            })),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(tag: u8) -> RunInputsRecord {
        RunInputsRecord {
            instance_id: [tag; 16],
            recipe_fingerprint: [tag; 32],
            handle: format!("kx/recipes/echo-{tag}"),
            args: format!("{{\"topic\":\"t{tag}\"}}").into_bytes(),
            principal: "tester".into(),
            captured_unix_ms: 1_000 + u64::from(tag),
        }
    }

    #[test]
    fn record_then_get_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let db = RunInputsDb::open(dir.path()).unwrap();
        assert!(db.get(&[0x42; 16]).unwrap().is_none());
        db.record(rec(0x42)).unwrap();
        let got = db.get(&[0x42; 16]).unwrap().expect("captured");
        assert_eq!(got.instance_id, [0x42; 16]);
        assert_eq!(got.recipe_fingerprint, [0x42; 32]);
        assert_eq!(got.handle, "kx/recipes/echo-66");
        assert_eq!(got.args, b"{\"topic\":\"t66\"}");
    }

    #[test]
    fn reopen_preserves_rows() {
        let dir = tempfile::tempdir().unwrap();
        {
            let db = RunInputsDb::open(dir.path()).unwrap();
            db.record(rec(0x11)).unwrap();
        }
        let db = RunInputsDb::open(dir.path()).unwrap();
        assert!(
            db.get(&[0x11; 16]).unwrap().is_some(),
            "rows survive a restart"
        );
    }

    #[test]
    fn insert_or_replace_latest_args_win() {
        let dir = tempfile::tempdir().unwrap();
        let db = RunInputsDb::open(dir.path()).unwrap();
        db.record(rec(0x07)).unwrap();
        // Same instance_id, edited args (the "re-run with changes" case).
        db.record(RunInputsRecord {
            args: b"{\"topic\":\"edited\"}".to_vec(),
            ..rec(0x07)
        })
        .unwrap();
        let got = db.get(&[0x07; 16]).unwrap().expect("captured");
        assert_eq!(got.args, b"{\"topic\":\"edited\"}", "latest args win");
    }

    #[test]
    fn corrupt_file_recreates_empty() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("run_inputs.db"), b"not a sqlite file").unwrap();
        let db = RunInputsDb::open(dir.path()).unwrap();
        assert!(
            db.get(&[0x11; 16]).unwrap().is_none(),
            "a corrupt sidecar recreates EMPTY (captured args are not derivable)"
        );
        // And it works after the recreate.
        db.record(rec(0x11)).unwrap();
        assert!(db.get(&[0x11; 16]).unwrap().is_some());
    }

    #[test]
    fn schema_version_drift_recreates_empty() {
        let dir = tempfile::tempdir().unwrap();
        {
            let db = RunInputsDb::open(dir.path()).unwrap();
            db.record(rec(0x22)).unwrap();
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "UPDATE meta SET value = 999 WHERE key = 'schema_version'",
                [],
            )
            .unwrap();
        }
        let db = RunInputsDb::open(dir.path()).unwrap();
        assert!(
            db.get(&[0x22; 16]).unwrap().is_none(),
            "version drift drops the captured args (the form opens blank)"
        );
    }
}
