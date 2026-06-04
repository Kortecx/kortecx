// SPDX-License-Identifier: Apache-2.0
//! [`SqliteCatalog`] — a durable [`CatalogRegistry`] (G1 / D94). The signature
//! registry survives a restart. A SQLite `signatures` table (content id PK + a
//! canonical-bincode entry BLOB) is the truth; an in-memory `BTreeMap` cache is
//! rebuilt on open so reads are byte-identical to [`crate::InMemoryCatalog`] and
//! need zero SQL. Writes go durable-first (inside a `BEGIN IMMEDIATE` txn) then
//! update the cache, so a rolled-back write never dirties the cache.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Mutex, RwLock};

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

use crate::entry::SignatureEntry;
use crate::registry::{CatalogError, CatalogRegistry, RegistrationOutcome};
use crate::signature::{canonical_config, TaskSignatureHash};
use crate::sqlite_util::{hash32, open_db, open_db_in_memory};

/// The durable registry schema version (independent of the journal's).
pub const CATALOG_SCHEMA_VERSION: u16 = 1;

const DDL: &str =
    "CREATE TABLE IF NOT EXISTS signatures (sig_hash BLOB PRIMARY KEY, entry_bytes BLOB NOT NULL);";

/// A durable, SQLite-backed [`CatalogRegistry`].
pub struct SqliteCatalog {
    conn: Mutex<Connection>,
    cache: RwLock<BTreeMap<TaskSignatureHash, SignatureEntry>>,
}

fn store_err<E: std::fmt::Display>(err: &E) -> CatalogError {
    CatalogError::Storage(err.to_string())
}

impl SqliteCatalog {
    /// Open (creating if absent) a durable registry at `path`.
    ///
    /// # Errors
    /// [`CatalogError::Storage`] on a SQLite / schema-version / corrupt-row failure.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, CatalogError> {
        Self::from_conn(open_db(path, CATALOG_SCHEMA_VERSION, DDL).map_err(|e| store_err(&e))?)
    }

    /// Open an ephemeral in-memory durable registry (tests + the shared harness).
    ///
    /// # Errors
    /// [`CatalogError::Storage`] on a SQLite failure.
    pub fn open_in_memory() -> Result<Self, CatalogError> {
        Self::from_conn(open_db_in_memory(CATALOG_SCHEMA_VERSION, DDL).map_err(|e| store_err(&e))?)
    }

    fn from_conn(conn: Connection) -> Result<Self, CatalogError> {
        let cache = rebuild(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
            cache: RwLock::new(cache),
        })
    }
}

/// Replay the durable table into the in-memory cache (hash order).
fn rebuild(conn: &Connection) -> Result<BTreeMap<TaskSignatureHash, SignatureEntry>, CatalogError> {
    let mut map = BTreeMap::new();
    let mut stmt = conn
        .prepare("SELECT sig_hash, entry_bytes FROM signatures ORDER BY sig_hash")
        .map_err(|e| store_err(&e))?;
    let rows = stmt
        .query_map([], |r| {
            Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, Vec<u8>>(1)?))
        })
        .map_err(|e| store_err(&e))?;
    for row in rows {
        let (h, b) = row.map_err(|e| store_err(&e))?;
        let hash = TaskSignatureHash::from_bytes(hash32(&h).map_err(|e| store_err(&e))?);
        let (entry, _): (SignatureEntry, usize) =
            bincode::serde::decode_from_slice(&b, canonical_config())
                .map_err(|e| CatalogError::Storage(format!("decode SignatureEntry: {e}")))?;
        map.insert(hash, entry);
    }
    Ok(map)
}

impl CatalogRegistry for SqliteCatalog {
    fn register_signature(
        &self,
        entry: SignatureEntry,
    ) -> Result<RegistrationOutcome, CatalogError> {
        let hash = entry.hash();
        let bytes = bincode::serde::encode_to_vec(&entry, canonical_config())
            .map_err(|e| CatalogError::Storage(format!("encode SignatureEntry: {e}")))?;
        let mut conn = self.conn.lock().expect("poisoned mutex");
        let txn = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| store_err(&e))?;
        let existing: Option<Vec<u8>> = txn
            .query_row(
                "SELECT entry_bytes FROM signatures WHERE sig_hash = ?1",
                params![hash.as_bytes().as_slice()],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| store_err(&e))?;
        // Byte-comparison is content-addressed equality (canonical encoding is
        // deterministic + collision-free for distinct values) — equivalent to the
        // in-memory `*occupied.get() == entry` check.
        let outcome = match existing {
            Some(b) if b == bytes => RegistrationOutcome::AlreadyPresent(hash),
            Some(_) => return Err(CatalogError::ImmutabilityConflict(hash.to_hex())),
            None => {
                txn.execute(
                    "INSERT INTO signatures (sig_hash, entry_bytes) VALUES (?1, ?2)",
                    params![hash.as_bytes().as_slice(), &bytes[..]],
                )
                .map_err(|e| store_err(&e))?;
                RegistrationOutcome::Inserted(hash)
            }
        };
        txn.commit().map_err(|e| store_err(&e))?;
        if outcome.is_inserted() {
            self.cache
                .write()
                .expect("poisoned lock")
                .insert(hash, entry);
        }
        Ok(outcome)
    }

    fn lookup(&self, hash: &TaskSignatureHash) -> Option<SignatureEntry> {
        self.cache.read().expect("poisoned lock").get(hash).cloned()
    }

    fn list_signatures<'a>(&'a self) -> Box<dyn Iterator<Item = SignatureEntry> + 'a> {
        let entries: Vec<SignatureEntry> = self
            .cache
            .read()
            .expect("poisoned lock")
            .values()
            .cloned()
            .collect();
        Box::new(entries.into_iter())
    }

    fn len(&self) -> usize {
        self.cache.read().expect("poisoned lock").len()
    }
}

const _: fn() = || {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<SqliteCatalog>();
};
