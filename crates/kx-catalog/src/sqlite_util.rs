// SPDX-License-Identifier: Apache-2.0
//! Shared SQLite plumbing for the durable catalog ledgers (G1 / D94) — open,
//! configure (WAL + FULL synchronous), run the per-ledger DDL, and stamp + verify
//! a schema-version row in a `metadata` table. Mirrors `kx_journal`'s
//! `SqliteJournal` open/configure/verify discipline so the durability + atomicity
//! guarantees are identical. Each ledger supplies its own `schema_version` + DDL
//! and maps [`StoreError`] onto its own typed `Storage(String)` variant.

use std::path::Path;

use rusqlite::{params, Connection, OpenFlags};

/// The metadata key holding the LE-`u16` schema version (one per ledger DB).
const METADATA_SCHEMA_VERSION_KEY: &str = "schema_version";

/// A durable-store open/IO failure, rendered onto each ledger's `Storage` variant.
#[derive(Debug, thiserror::Error)]
pub(crate) enum StoreError {
    /// A SQLite error (open / I/O / SQL).
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// The on-disk schema version is not this binary's — refuse loudly rather than
    /// mis-read an incompatible file (mirrors `JournalError::SchemaVersionMismatch`).
    #[error("schema_version mismatch: expected {expected}, found {found}")]
    SchemaMismatch {
        /// This binary's schema version.
        expected: u16,
        /// The version stored in the file.
        found: u16,
    },
    /// A structurally corrupt durable store (a malformed metadata/fact row).
    #[error("corrupt durable store: {0}")]
    Corrupt(String),
}

/// PRAGMA configuration applied at open time — identical to the journal's
/// (`WAL` + `synchronous=FULL` ⇒ fsync per commit, atomic rollback).
fn configure(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = FULL;
         PRAGMA foreign_keys = ON;
         PRAGMA temp_store = MEMORY;",
    )
}

/// Create the shared `metadata` table + the ledger's `ddl` (idempotent), stamp the
/// schema version once, then verify it equals `schema_version` (loud refusal).
fn init_and_verify(conn: &Connection, schema_version: u16, ddl: &str) -> Result<(), StoreError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS metadata (key TEXT PRIMARY KEY, value BLOB NOT NULL);",
    )?;
    conn.execute_batch(ddl)?;
    let version_bytes: [u8; 2] = schema_version.to_le_bytes();
    conn.execute(
        "INSERT OR IGNORE INTO metadata (key, value) VALUES (?1, ?2)",
        params![METADATA_SCHEMA_VERSION_KEY, &version_bytes[..]],
    )?;
    let stored: Vec<u8> = conn.query_row(
        "SELECT value FROM metadata WHERE key = ?1",
        params![METADATA_SCHEMA_VERSION_KEY],
        |r| r.get(0),
    )?;
    if stored.len() != 2 {
        return Err(StoreError::Corrupt(
            "metadata.schema_version is not 2 bytes".to_string(),
        ));
    }
    let found = u16::from_le_bytes([stored[0], stored[1]]);
    if found != schema_version {
        return Err(StoreError::SchemaMismatch {
            expected: schema_version,
            found,
        });
    }
    Ok(())
}

/// Open (creating if absent) a durable ledger DB at `path`, configured + schema
/// verified.
pub(crate) fn open_db(
    path: impl AsRef<Path>,
    schema_version: u16,
    ddl: &str,
) -> Result<Connection, StoreError> {
    let conn = Connection::open_with_flags(
        path.as_ref(),
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    configure(&conn)?;
    init_and_verify(&conn, schema_version, ddl)?;
    Ok(conn)
}

/// Open an ephemeral in-memory durable ledger DB (for tests + the
/// backend-agnostic `run_with_each_backend` harness).
pub(crate) fn open_db_in_memory(schema_version: u16, ddl: &str) -> Result<Connection, StoreError> {
    let conn = Connection::open_in_memory()?;
    configure(&conn)?;
    init_and_verify(&conn, schema_version, ddl)?;
    Ok(conn)
}

/// Parse a 32-byte content id from a BLOB column, or a corrupt-store error.
pub(crate) fn hash32(bytes: &[u8]) -> Result<[u8; 32], StoreError> {
    bytes
        .try_into()
        .map_err(|_| StoreError::Corrupt("content-id column is not 32 bytes".to_string()))
}
