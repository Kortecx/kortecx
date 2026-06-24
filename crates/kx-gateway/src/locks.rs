//! POC-5b — the per-App lock sidecar: `locks.db` under `--catalog-dir`, backing the
//! [`LockStore`] seam (`LockApp` / `UnlockApp` + the `AdvanceBranch` chokepoint).
//!
//! ## Rebuildable-to-EMPTY, fails OPEN
//! A lock is a per-party POLICY decision on a branch — NOT journal-derivable. On
//! corruption or a schema-version drift this ledger recreates EMPTY: every branch
//! reads as UNLOCKED (editing is restored). A lock is an AVAILABILITY gate, not an
//! integrity gate, so failing open is the safe direction — losing the file can never
//! brick an App's editing, it only drops the (re-settable) policy. Never journaled,
//! never a `MoteId` input, never a digest input.
//!
//! ## Caller-scoped
//! The primary key is `(principal, branch_handle)` — a party can only lock / unlock /
//! observe its OWN branches (no cross-party existence oracle).

use std::path::Path;
use std::sync::Mutex;

use kx_gateway_core::GatewayError as CoreError;
use kx_gateway_core::LockStore;
use rusqlite::{params, Connection};

use crate::error::GatewayError;

/// Bump on any table-shape change. Unknown/missing version ⇒ recreate EMPTY (locks
/// are not journal-derivable; rebuild = every branch unlocked).
const SCHEMA_VERSION: i64 = 1;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS locks (
    principal     TEXT NOT NULL,   -- server-resolved caller party (scope)
    branch_handle TEXT NOT NULL,   -- the locked branch (= the App handle, one-App-one-branch)
    PRIMARY KEY (principal, branch_handle)
);
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value INTEGER NOT NULL);
";

/// The durable per-App lock store over `locks.db`. A single mutex'd connection: lock
/// toggles are interactive-rate (a CLI/UI lock/unlock), the chokepoint read is a
/// point lookup.
pub(crate) struct LocksDb {
    conn: Mutex<Connection>,
}

impl LocksDb {
    /// Open (or create) `locks.db` under `dir`. A corrupt/foreign file or a
    /// `schema_version` drift recreates the ledger EMPTY (module doc — fails open).
    ///
    /// # Errors
    /// [`GatewayError::Catalog`] on an unrecoverable open/pragma failure.
    pub(crate) fn open(dir: &Path) -> Result<Self, GatewayError> {
        std::fs::create_dir_all(dir)
            .map_err(|e| GatewayError::Catalog(format!("locks dir: {e}")))?;
        let db_path = dir.join("locks.db");
        let conn = if let Ok(c) = Self::open_with_pragma(&db_path) {
            c
        } else {
            let _ = std::fs::remove_file(&db_path);
            let _ = std::fs::remove_file(dir.join("locks.db-wal"));
            let _ = std::fs::remove_file(dir.join("locks.db-shm"));
            Self::open_with_pragma(&db_path)
                .map_err(|e| GatewayError::Catalog(format!("locks reopen: {e}")))?
        };
        let fresh_or_stale = match Self::read_schema_version(&conn) {
            Ok(Some(v)) => v != SCHEMA_VERSION,
            Ok(None) | Err(_) => true,
        };
        if fresh_or_stale {
            conn.execute_batch("DROP TABLE IF EXISTS locks; DROP TABLE IF EXISTS meta;")
                .map_err(|e| GatewayError::Catalog(format!("locks rebuild: {e}")))?;
        }
        conn.execute_batch(SCHEMA)
            .map_err(|e| GatewayError::Catalog(format!("locks schema: {e}")))?;
        conn.execute(
            "INSERT OR IGNORE INTO meta(key, value) VALUES ('schema_version', ?1)",
            params![SCHEMA_VERSION],
        )
        .map_err(|e| GatewayError::Catalog(format!("locks meta init: {e}")))?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn open_with_pragma(db_path: &Path) -> rusqlite::Result<Connection> {
        let conn = Connection::open(db_path)?;
        conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;")?;
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

impl LockStore for LocksDb {
    fn is_locked(&self, principal: &str, branch_handle: &str) -> Result<bool, CoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| CoreError::Internal("locks lock poisoned".into()))?;
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(1) FROM locks WHERE principal = ?1 AND branch_handle = ?2",
                params![principal, branch_handle],
                |r| r.get(0),
            )
            .map_err(|e| CoreError::Internal(format!("locks probe: {e}")))?;
        Ok(count > 0)
    }

    fn lock(&self, principal: &str, branch_handle: &str) -> Result<bool, CoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| CoreError::Internal("locks lock poisoned".into()))?;
        conn.execute(
            "INSERT OR IGNORE INTO locks(principal, branch_handle) VALUES (?1, ?2)",
            params![principal, branch_handle],
        )
        .map_err(|e| CoreError::Internal(format!("locks insert: {e}")))?;
        Ok(true)
    }

    fn unlock(&self, principal: &str, branch_handle: &str) -> Result<bool, CoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| CoreError::Internal("locks lock poisoned".into()))?;
        conn.execute(
            "DELETE FROM locks WHERE principal = ?1 AND branch_handle = ?2",
            params![principal, branch_handle],
        )
        .map_err(|e| CoreError::Internal(format!("locks delete: {e}")))?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir() -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        let stamp = format!("kx-locks-test-{}-{:p}", std::process::id(), &p);
        p.push(stamp);
        p
    }

    #[test]
    fn lock_unlock_round_trip() {
        let dir = tmp_dir();
        let db = LocksDb::open(&dir).unwrap();
        assert!(!db.is_locked("alice", "team/apps/x").unwrap());
        assert!(db.lock("alice", "team/apps/x").unwrap());
        assert!(db.is_locked("alice", "team/apps/x").unwrap());
        // idempotent lock
        assert!(db.lock("alice", "team/apps/x").unwrap());
        assert!(db.is_locked("alice", "team/apps/x").unwrap());
        // unlock
        assert!(db.unlock("alice", "team/apps/x").unwrap());
        assert!(!db.is_locked("alice", "team/apps/x").unwrap());
        // idempotent unlock of an already-unlocked branch
        assert!(db.unlock("alice", "team/apps/x").unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn lock_is_caller_scoped() {
        let dir = tmp_dir();
        let db = LocksDb::open(&dir).unwrap();
        db.lock("alice", "team/apps/x").unwrap();
        // bob does not see alice's lock; bob unlocking is a no-op on alice's lock.
        assert!(!db.is_locked("bob", "team/apps/x").unwrap());
        db.unlock("bob", "team/apps/x").unwrap();
        assert!(db.is_locked("alice", "team/apps/x").unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn schema_drift_rebuilds_empty_fails_open() {
        let dir = tmp_dir();
        {
            let db = LocksDb::open(&dir).unwrap();
            db.lock("alice", "team/apps/x").unwrap();
        }
        {
            let conn = Connection::open(dir.join("locks.db")).unwrap();
            conn.execute("UPDATE meta SET value = 999 WHERE key = 'schema_version'", [])
                .unwrap();
        }
        // Reopen recreates EMPTY ⇒ the branch reads UNLOCKED (fails open).
        let db = LocksDb::open(&dir).unwrap();
        assert!(
            !db.is_locked("alice", "team/apps/x").unwrap(),
            "schema drift must rebuild empty (fail open)"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
