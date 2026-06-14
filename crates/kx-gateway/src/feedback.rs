//! The PR-4.1 feedback sidecar: `feedback.db` under `--catalog-dir`, backing the
//! [`FeedbackStore`] seam — the `SubmitFeedback` 👍/👎 rows + their `ListFeedback`
//! read-back.
//!
//! ## Rebuildable-to-EMPTY (the uploads.db posture, not capture.db)
//! Feedback is CLIENT-ORIGIN product signal that never touches the journal, so
//! (unlike capture, which is journal-derived) it is NOT derivable from anything.
//! On corruption or a schema-version drift this ledger recreates EMPTY: the only
//! loss is the product-signal rows. Never journaled, never a `MoteId` input,
//! never a digest input — dropping the file cannot move the canonical projection
//! digest.
//!
//! ## Advisory only
//! `rating`/`comment`/the target+context keys are advisory: identity is the
//! server-derived `feedback_id` alone (SN-8). `INSERT OR REPLACE` keys on that
//! id, which the handler derives deterministically over `(message_id, principal)`
//! — so a party re-rating the same answer OVERWRITES (the "changed my mind" UX).

use std::path::Path;
use std::sync::Mutex;

use kx_gateway_core::GatewayError as CoreError;
use kx_gateway_core::{FeedbackEntry, FeedbackRecord, FeedbackStore};
use rusqlite::{params, Connection};

use crate::error::GatewayError;

/// Bump on any table-shape change. Unknown/missing version ⇒ recreate EMPTY
/// (feedback is not derivable, so there is no rebuild).
const SCHEMA_VERSION: i64 = 1;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS feedback (
    feedback_id   BLOB PRIMARY KEY,   -- 16B server-derived (over message_id+principal)
    rating        INTEGER NOT NULL,   -- 1=UP 2=DOWN
    message_id    TEXT NOT NULL,      -- the rated chat message id
    instance_id   BLOB NOT NULL,      -- 16B run (all-zero when no run)
    mote_id       BLOB NOT NULL,      -- 32B terminal mote (all-zero when absent)
    content_ref   BLOB NOT NULL,      -- 32B answer ref (all-zero when absent)
    comment       TEXT NOT NULL,      -- optional note (handler-capped)
    recipe_handle TEXT NOT NULL,      -- advisory context
    model_id      TEXT NOT NULL,      -- advisory context
    principal     TEXT NOT NULL,      -- server-resolved caller party (audit)
    submitted_ms  INTEGER NOT NULL    -- wall clock, audit only
);
CREATE INDEX IF NOT EXISTS feedback_by_instance ON feedback(instance_id);
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value INTEGER NOT NULL);
";

/// The durable feedback ledger over `feedback.db`. A single mutex'd connection:
/// feedback is interactive-rate (a chat 👍, a CLI submit), never contended.
pub(crate) struct FeedbackDb {
    conn: Mutex<Connection>,
}

impl FeedbackDb {
    /// Open (or create) `feedback.db` under `dir`. A corrupt/foreign file or a
    /// `schema_version` drift recreates the ledger EMPTY (module doc).
    ///
    /// # Errors
    /// [`GatewayError::Catalog`] on an unrecoverable open/pragma failure.
    pub(crate) fn open(dir: &Path) -> Result<Self, GatewayError> {
        std::fs::create_dir_all(dir)
            .map_err(|e| GatewayError::Catalog(format!("feedback dir: {e}")))?;
        let db_path = dir.join("feedback.db");
        // A non-SQLite file fails even the pragma — delete + recreate (the rows
        // are advisory product signal; nothing here is truth).
        let conn = if let Ok(c) = Self::open_with_pragma(&db_path) {
            c
        } else {
            let _ = std::fs::remove_file(&db_path);
            let _ = std::fs::remove_file(dir.join("feedback.db-wal"));
            let _ = std::fs::remove_file(dir.join("feedback.db-shm"));
            Self::open_with_pragma(&db_path)
                .map_err(|e| GatewayError::Catalog(format!("feedback reopen: {e}")))?
        };
        let fresh_or_stale = match Self::read_schema_version(&conn) {
            Ok(Some(v)) => v != SCHEMA_VERSION,
            Ok(None) | Err(_) => true,
        };
        if fresh_or_stale {
            conn.execute_batch(
                "DROP TABLE IF EXISTS feedback;
                 DROP TABLE IF EXISTS meta;",
            )
            .map_err(|e| GatewayError::Catalog(format!("feedback rebuild: {e}")))?;
        }
        conn.execute_batch(SCHEMA)
            .map_err(|e| GatewayError::Catalog(format!("feedback schema: {e}")))?;
        conn.execute(
            "INSERT OR IGNORE INTO meta(key, value) VALUES ('schema_version', ?1)",
            params![SCHEMA_VERSION],
        )
        .map_err(|e| GatewayError::Catalog(format!("feedback meta init: {e}")))?;
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
        .map_err(|_| CoreError::Internal(format!("feedback row: expected {N}-byte blob")))
}

impl FeedbackStore for FeedbackDb {
    fn record(&self, rec: FeedbackRecord) -> Result<(), CoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| CoreError::Internal("feedback lock poisoned".into()))?;
        conn.execute(
            "INSERT OR REPLACE INTO feedback(feedback_id, rating, message_id, instance_id, \
             mote_id, content_ref, comment, recipe_handle, model_id, principal, submitted_ms) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                rec.feedback_id.to_vec(),
                rec.rating,
                rec.message_id,
                rec.instance_id.to_vec(),
                rec.mote_id.to_vec(),
                rec.content_ref.to_vec(),
                rec.comment,
                rec.recipe_handle,
                rec.model_id,
                rec.principal,
                i64::try_from(rec.submitted_unix_ms).unwrap_or(i64::MAX),
            ],
        )
        .map_err(|e| CoreError::Internal(format!("feedback record: {e}")))?;
        Ok(())
    }

    fn list(
        &self,
        limit: usize,
        instance_id: Option<[u8; 16]>,
        before_rowid: Option<u64>,
    ) -> Result<(Vec<FeedbackEntry>, bool), CoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| CoreError::Internal("feedback lock poisoned".into()))?;
        // Composable filter; over-fetch by one to compute `has_more` without a
        // second COUNT query (the telemetry.rs pagination trick).
        let mut sql = String::from(
            "SELECT rowid, feedback_id, rating, message_id, instance_id, mote_id, \
             content_ref, comment, recipe_handle, model_id, submitted_ms FROM feedback WHERE 1=1",
        );
        let mut binds: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        if let Some(iid) = instance_id {
            sql.push_str(" AND instance_id = ?");
            binds.push(Box::new(iid.to_vec()));
        }
        if let Some(before) = before_rowid {
            sql.push_str(" AND rowid < ?");
            binds.push(Box::new(i64::try_from(before).unwrap_or(i64::MAX)));
        }
        sql.push_str(" ORDER BY rowid DESC LIMIT ?");
        binds.push(Box::new(i64::try_from(limit + 1).unwrap_or(i64::MAX)));

        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| CoreError::Internal(format!("feedback list prepare: {e}")))?;
        let bind_refs: Vec<&dyn rusqlite::types::ToSql> =
            binds.iter().map(std::convert::AsRef::as_ref).collect();
        let rows = stmt
            .query_map(bind_refs.as_slice(), |r| {
                Ok((
                    r.get::<_, i64>(0)?,     // rowid
                    r.get::<_, Vec<u8>>(1)?, // feedback_id
                    r.get::<_, i64>(2)?,     // rating
                    r.get::<_, String>(3)?,  // message_id
                    r.get::<_, Vec<u8>>(4)?, // instance_id
                    r.get::<_, Vec<u8>>(5)?, // mote_id
                    r.get::<_, Vec<u8>>(6)?, // content_ref
                    r.get::<_, String>(7)?,  // comment
                    r.get::<_, String>(8)?,  // recipe_handle
                    r.get::<_, String>(9)?,  // model_id
                    r.get::<_, i64>(10)?,    // submitted_ms
                ))
            })
            .map_err(|e| CoreError::Internal(format!("feedback list query: {e}")))?;

        let mut out = Vec::with_capacity(limit + 1);
        for row in rows {
            let (rowid, fid, rating, message_id, iid, mid, cref, comment, handle, model, ms) =
                row.map_err(|e| CoreError::Internal(format!("feedback list row: {e}")))?;
            out.push(FeedbackEntry {
                feedback_id: fixed::<16>(&fid)?,
                rating: i32::try_from(rating).unwrap_or(0),
                message_id,
                instance_id: fixed::<16>(&iid)?,
                mote_id: fixed::<32>(&mid)?,
                content_ref: fixed::<32>(&cref)?,
                comment,
                recipe_handle: handle,
                model_id: model,
                submitted_unix_ms: u64::try_from(ms).unwrap_or(0),
                rowid: u64::try_from(rowid).unwrap_or(0),
            });
        }
        let has_more = out.len() > limit;
        out.truncate(limit);
        Ok((out, has_more))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(tag: u8) -> FeedbackRecord {
        FeedbackRecord {
            feedback_id: [tag; 16],
            rating: 1,
            message_id: format!("msg-{tag}"),
            instance_id: [tag; 16],
            mote_id: [tag; 32],
            content_ref: [tag; 32],
            comment: String::new(),
            recipe_handle: "kx/recipes/chat".into(),
            model_id: "qwen3".into(),
            principal: "tester".into(),
            submitted_unix_ms: 1_000 + u64::from(tag),
        }
    }

    #[test]
    fn record_then_list_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let db = FeedbackDb::open(dir.path()).unwrap();
        let (rows, has_more) = db.list(50, None, None).unwrap();
        assert!(rows.is_empty() && !has_more);
        db.record(rec(0x42)).unwrap();
        let (rows, has_more) = db.list(50, None, None).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].feedback_id, [0x42; 16]);
        assert_eq!(rows[0].rating, 1);
        assert_eq!(rows[0].message_id, "msg-66");
        assert!(!has_more);
    }

    #[test]
    fn re_rating_overwrites_on_feedback_id() {
        let dir = tempfile::tempdir().unwrap();
        let db = FeedbackDb::open(dir.path()).unwrap();
        db.record(rec(0x10)).unwrap();
        // Same feedback_id, flipped rating ("changed my mind").
        let mut down = rec(0x10);
        down.rating = 2;
        db.record(down).unwrap();
        let (rows, _) = db.list(50, None, None).unwrap();
        assert_eq!(rows.len(), 1, "the re-rating overwrote, not appended");
        assert_eq!(rows[0].rating, 2);
    }

    #[test]
    fn instance_filter_scopes() {
        let dir = tempfile::tempdir().unwrap();
        let db = FeedbackDb::open(dir.path()).unwrap();
        db.record(rec(0x01)).unwrap();
        db.record(rec(0x02)).unwrap();
        let (rows, _) = db.list(50, Some([0x01; 16]), None).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].instance_id, [0x01; 16]);
    }

    #[test]
    fn pagination_walks_without_dup_or_miss() {
        let dir = tempfile::tempdir().unwrap();
        let db = FeedbackDb::open(dir.path()).unwrap();
        for i in 0u16..1_200 {
            let mut r = rec(0);
            // distinct feedback_id per row (vary the 16 bytes)
            r.feedback_id = [(i & 0xff) as u8; 16];
            r.feedback_id[0] = (i >> 8) as u8;
            r.feedback_id[1] = (i & 0xff) as u8;
            r.message_id = format!("m-{i}");
            db.record(r).unwrap();
        }
        let mut seen = std::collections::HashSet::new();
        let mut cursor = None;
        loop {
            let (rows, has_more) = db.list(100, None, cursor).unwrap();
            if rows.is_empty() {
                break;
            }
            for row in &rows {
                assert!(seen.insert(row.message_id.clone()), "no duplicate row");
            }
            cursor = rows.last().map(|r| r.rowid);
            if !has_more {
                break;
            }
        }
        assert_eq!(seen.len(), 1_200, "every row visited exactly once");
    }

    #[test]
    fn reopen_preserves_rows() {
        let dir = tempfile::tempdir().unwrap();
        {
            let db = FeedbackDb::open(dir.path()).unwrap();
            db.record(rec(0x11)).unwrap();
        }
        let db = FeedbackDb::open(dir.path()).unwrap();
        let (rows, _) = db.list(50, None, None).unwrap();
        assert_eq!(rows.len(), 1, "rows survive a restart");
    }

    #[test]
    fn corrupt_file_recreates_empty() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("feedback.db"), b"not a sqlite file").unwrap();
        let db = FeedbackDb::open(dir.path()).unwrap();
        let (rows, _) = db.list(50, None, None).unwrap();
        assert!(rows.is_empty(), "a corrupt sidecar recreates EMPTY");
        db.record(rec(0x11)).unwrap();
        assert_eq!(db.list(50, None, None).unwrap().0.len(), 1);
    }

    #[test]
    fn schema_version_drift_recreates_empty() {
        let dir = tempfile::tempdir().unwrap();
        {
            let db = FeedbackDb::open(dir.path()).unwrap();
            db.record(rec(0x22)).unwrap();
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "UPDATE meta SET value = 999 WHERE key = 'schema_version'",
                [],
            )
            .unwrap();
        }
        let db = FeedbackDb::open(dir.path()).unwrap();
        assert!(
            db.list(50, None, None).unwrap().0.is_empty(),
            "version drift drops the product-signal rows"
        );
    }
}
