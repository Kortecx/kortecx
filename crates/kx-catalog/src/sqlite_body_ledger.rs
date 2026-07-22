// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! [`SqliteBodyLedger`] — a durable [`BodyLedger`] (G1 / D94). Published recipe
//! bodies survive a restart, so a published snapshot stays invocable end-to-end
//! across a process kill. A SQLite `bodies` table (`ManifestId` PK + a
//! canonical-bincode `WorkflowDef` BLOB) is the truth; an in-memory cache is
//! rebuilt on open and **re-verifies each row compiles to its stored key**
//! (fail-closed on a corrupt/tampered body — never serve a mismatched recipe).

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Mutex, RwLock};

use kx_workflow::{ManifestId, WorkflowDef};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

use crate::body::{body_manifest_id, BodyLedger, BodyLedgerError, BodyOutcome};
use crate::signature::canonical_config;
use crate::sqlite_util::{hash32, open_db, open_db_in_memory};

/// The durable body-ledger schema version.
pub const BODY_LEDGER_SCHEMA_VERSION: u16 = 1;

const DDL: &str =
    "CREATE TABLE IF NOT EXISTS bodies (manifest_id BLOB PRIMARY KEY, body_bytes BLOB NOT NULL);";

/// A durable, SQLite-backed [`BodyLedger`].
pub struct SqliteBodyLedger {
    conn: Mutex<Connection>,
    cache: RwLock<BTreeMap<ManifestId, WorkflowDef>>,
}

fn store_err<E: std::fmt::Display>(err: &E) -> BodyLedgerError {
    BodyLedgerError::Storage(err.to_string())
}

fn encode(body: &WorkflowDef) -> Result<Vec<u8>, BodyLedgerError> {
    bincode::serde::encode_to_vec(body, canonical_config())
        .map_err(|e| BodyLedgerError::Storage(format!("encode WorkflowDef: {e}")))
}

impl SqliteBodyLedger {
    /// Open (creating if absent) a durable body ledger at `path`.
    ///
    /// # Errors
    /// [`BodyLedgerError::Storage`] on a SQLite / schema / content-verification
    /// failure.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, BodyLedgerError> {
        Self::from_conn(open_db(path, BODY_LEDGER_SCHEMA_VERSION, DDL).map_err(|e| store_err(&e))?)
    }

    /// Open an ephemeral in-memory durable body ledger.
    ///
    /// # Errors
    /// [`BodyLedgerError::Storage`] on a SQLite failure.
    pub fn open_in_memory() -> Result<Self, BodyLedgerError> {
        Self::from_conn(
            open_db_in_memory(BODY_LEDGER_SCHEMA_VERSION, DDL).map_err(|e| store_err(&e))?,
        )
    }

    fn from_conn(conn: Connection) -> Result<Self, BodyLedgerError> {
        let cache = rebuild(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
            cache: RwLock::new(cache),
        })
    }
}

/// Replay the durable table into the cache, re-verifying each body compiles to
/// its stored key (content-verification survives a corrupt/tampered row).
fn rebuild(conn: &Connection) -> Result<BTreeMap<ManifestId, WorkflowDef>, BodyLedgerError> {
    let mut map = BTreeMap::new();
    let mut stmt = conn
        .prepare("SELECT manifest_id, body_bytes FROM bodies ORDER BY manifest_id")
        .map_err(|e| store_err(&e))?;
    let rows = stmt
        .query_map([], |r| {
            Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, Vec<u8>>(1)?))
        })
        .map_err(|e| store_err(&e))?;
    for row in rows {
        let (k, b) = row.map_err(|e| store_err(&e))?;
        let key = ManifestId(hash32(&k).map_err(|e| store_err(&e))?);
        let (body, _): (WorkflowDef, usize) =
            bincode::serde::decode_from_slice(&b, canonical_config())
                .map_err(|e| BodyLedgerError::Storage(format!("decode WorkflowDef: {e}")))?;
        // Tamper-evidence: the stored body MUST compile to its stored key.
        let derived = body_manifest_id(&body)?;
        if derived != key {
            return Err(BodyLedgerError::Storage(format!(
                "stored body for {} compiles to {} (content mismatch)",
                key.to_hex(),
                derived.to_hex()
            )));
        }
        map.insert(key, body);
    }
    Ok(map)
}

impl BodyLedger for SqliteBodyLedger {
    fn publish_body(
        &self,
        body: WorkflowDef,
    ) -> Result<(ManifestId, BodyOutcome), BodyLedgerError> {
        let id = body_manifest_id(&body)?;
        let bytes = encode(&body)?;
        let mut conn = self.conn.lock().expect("poisoned mutex");
        let txn = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| store_err(&e))?;
        let existing: Option<Vec<u8>> = txn
            .query_row(
                "SELECT body_bytes FROM bodies WHERE manifest_id = ?1",
                params![id.0.as_slice()],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| store_err(&e))?;
        let outcome = match existing {
            Some(b) if b == bytes => BodyOutcome::AlreadyPresent(id),
            Some(_) => return Err(BodyLedgerError::ImmutabilityConflict(id.to_hex())),
            None => {
                txn.execute(
                    "INSERT INTO bodies (manifest_id, body_bytes) VALUES (?1, ?2)",
                    params![id.0.as_slice(), &bytes[..]],
                )
                .map_err(|e| store_err(&e))?;
                BodyOutcome::Inserted(id)
            }
        };
        txn.commit().map_err(|e| store_err(&e))?;
        if matches!(outcome, BodyOutcome::Inserted(_)) {
            self.cache.write().expect("poisoned lock").insert(id, body);
        }
        Ok((id, outcome))
    }

    fn get_body(&self, manifest_id: &ManifestId) -> Option<WorkflowDef> {
        self.cache
            .read()
            .expect("poisoned lock")
            .get(manifest_id)
            .cloned()
    }

    fn len(&self) -> usize {
        self.cache.read().expect("poisoned lock").len()
    }
}

const _: fn() = || {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<SqliteBodyLedger>();
};
