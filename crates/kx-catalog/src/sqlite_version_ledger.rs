// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! [`SqliteVersionLedger`] — a durable [`VersionLedger`] (G1 / D94). Published
//! versions + the mutable handle survive a restart. A SQLite `versions` table
//! (`seq` PK preserving append order + a content-id unique index + a
//! canonical-bincode version BLOB) is the truth; the in-memory [`Inner`] is rebuilt
//! on open by replaying through the SHARED [`Inner::apply_version`], which
//! recomputes the handle-move-by-rank from current state — so the resolved handle
//! is identical across a restart (the rank is a total, order-independent order).

use std::path::Path;
use std::sync::{Mutex, RwLock};

use rusqlite::{params, Connection, TransactionBehavior};

use crate::in_memory_version_ledger::{
    fold_descendants, fold_lineage, read_resolve, snapshot_versions, validate_lineage, version_at,
    Inner,
};
use crate::path::AssetPath;
use crate::signature::canonical_config;
use crate::sqlite_util::{open_db, open_db_in_memory};
use crate::version::{AssetVersion, VersionId, VersionedContent};
use crate::version_ledger::{PublishOutcome, VersionLedger, VersionLedgerError};

/// The durable version-ledger schema version.
pub const VERSION_LEDGER_SCHEMA_VERSION: u16 = 1;

const DDL: &str = "CREATE TABLE IF NOT EXISTS versions (
    seq           INTEGER PRIMARY KEY,
    version_id    BLOB NOT NULL,
    version_bytes BLOB NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_version_id ON versions (version_id);";

/// A durable, SQLite-backed [`VersionLedger`].
pub struct SqliteVersionLedger {
    conn: Mutex<Connection>,
    inner: RwLock<Inner>,
}

fn store_err<E: std::fmt::Display>(err: &E) -> VersionLedgerError {
    VersionLedgerError::Storage(err.to_string())
}

fn encode(version: &AssetVersion) -> Result<Vec<u8>, VersionLedgerError> {
    bincode::serde::encode_to_vec(version, canonical_config())
        .map_err(|e| VersionLedgerError::Storage(format!("encode AssetVersion: {e}")))
}

fn next_seq(txn: &rusqlite::Transaction<'_>) -> Result<i64, VersionLedgerError> {
    let max: Option<i64> = txn
        .query_row("SELECT MAX(seq) FROM versions", [], |r| r.get(0))
        .map_err(|e| store_err(&e))?;
    Ok(max.unwrap_or(0) + 1)
}

impl SqliteVersionLedger {
    /// Open (creating if absent) a durable version ledger at `path`.
    ///
    /// # Errors
    /// [`VersionLedgerError::Storage`] on a SQLite / schema / corrupt-row failure.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, VersionLedgerError> {
        Self::from_conn(
            open_db(path, VERSION_LEDGER_SCHEMA_VERSION, DDL).map_err(|e| store_err(&e))?,
        )
    }

    /// Open an ephemeral in-memory durable version ledger.
    ///
    /// # Errors
    /// [`VersionLedgerError::Storage`] on a SQLite failure.
    pub fn open_in_memory() -> Result<Self, VersionLedgerError> {
        Self::from_conn(
            open_db_in_memory(VERSION_LEDGER_SCHEMA_VERSION, DDL).map_err(|e| store_err(&e))?,
        )
    }

    fn from_conn(conn: Connection) -> Result<Self, VersionLedgerError> {
        let inner = rebuild(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
            inner: RwLock::new(inner),
        })
    }
}

/// Replay the durable log into a fresh [`Inner`] via the shared apply (append order).
fn rebuild(conn: &Connection) -> Result<Inner, VersionLedgerError> {
    let mut inner = Inner::default();
    let mut stmt = conn
        .prepare("SELECT version_bytes FROM versions ORDER BY seq")
        .map_err(|e| store_err(&e))?;
    let rows = stmt
        .query_map([], |r| r.get::<_, Vec<u8>>(0))
        .map_err(|e| store_err(&e))?;
    for row in rows {
        let b = row.map_err(|e| store_err(&e))?;
        let (version, _): (AssetVersion, usize) =
            bincode::serde::decode_from_slice(&b, canonical_config())
                .map_err(|e| VersionLedgerError::Storage(format!("decode AssetVersion: {e}")))?;
        inner.apply_version(version);
    }
    Ok(inner)
}

impl VersionLedger for SqliteVersionLedger {
    fn publish(&self, version: AssetVersion) -> Result<PublishOutcome, VersionLedgerError> {
        let vid = version.version_id();
        let mut inner = self.inner.write().expect("poisoned lock");
        if let Some(existing) = inner.contains_version(&vid) {
            return if *existing == version {
                Ok(PublishOutcome::AlreadyPresent(vid))
            } else {
                Err(VersionLedgerError::ImmutabilityConflict(vid.to_hex()))
            };
        }
        validate_lineage(&inner, &version)?;
        // Durable-first: append under the write lock, then replay through the
        // shared apply (which recomputes the handle move).
        let bytes = encode(&version)?;
        let mut conn = self.conn.lock().expect("poisoned mutex");
        let txn = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| store_err(&e))?;
        let seq = next_seq(&txn)?;
        txn.execute(
            "INSERT INTO versions (seq, version_id, version_bytes) VALUES (?1, ?2, ?3)",
            params![seq, vid.as_bytes().as_slice(), &bytes[..]],
        )
        .map_err(|e| store_err(&e))?;
        txn.commit().map_err(|e| store_err(&e))?;
        inner.apply_version(version);
        Ok(PublishOutcome::Published(vid))
    }

    fn resolve(&self, handle: &AssetPath) -> Option<(VersionedContent, VersionId)> {
        read_resolve(&self.inner.read().expect("poisoned lock"), handle)
    }

    fn get_version(&self, id: &VersionId) -> Option<AssetVersion> {
        version_at(&self.inner.read().expect("poisoned lock"), id).cloned()
    }

    fn lineage(&self, id: &VersionId) -> Vec<AssetVersion> {
        fold_lineage(&self.inner.read().expect("poisoned lock"), *id)
    }

    fn descendants(&self, id: &VersionId) -> Vec<VersionId> {
        fold_descendants(&self.inner.read().expect("poisoned lock"), *id)
    }

    fn list_versions<'a>(&'a self) -> Box<dyn Iterator<Item = AssetVersion> + 'a> {
        let versions = snapshot_versions(&self.inner.read().expect("poisoned lock"));
        Box::new(versions.into_iter())
    }

    fn len(&self) -> usize {
        self.inner.read().expect("poisoned lock").len_versions()
    }
}

const _: fn() = || {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<SqliteVersionLedger>();
};
