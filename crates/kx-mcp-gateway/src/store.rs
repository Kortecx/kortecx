//! The off-journal, rebuildable `connections.db` sidecar.
//!
//! Stores operator-registered external MCP servers (the [`Connection`] records).
//! Mirrors the durability discipline of `tools.db` (`kx-tool-registry`): WAL +
//! `synchronous=FULL`, a stamped schema version with loud-refusal on mismatch,
//! the credential ref NAME only (never the secret — D81), and a server-derived
//! `connection_id`. It is **off the digest/journal path**: deleting the file
//! loses connections (re-register to restore) but NEVER affects the canonical
//! projection digest. The SQLite plumbing is REPLICATED per-crate (not shared),
//! the same convention `kx-tool-registry`/`kx-fleet`/`kx-catalog` follow.

use std::path::Path;
use std::sync::Mutex;

use rusqlite::{params, Connection as SqliteConn, OpenFlags};

use crate::connection::{
    connection_id_of, Connection, ConnectionHealth, SessionMode, TransportSpec,
};
use crate::errors::GatewayError;

/// The connections sidecar schema version (LE-u16). PR-6b-3 bumped it 1 → 2 to add
/// the `session_mode` column; the migration is FORWARD + lossless (an idempotent
/// `ALTER TABLE ADD COLUMN` preserves every existing row), never a silent wipe.
const CONNECTIONS_SCHEMA_VERSION: u16 = 2;

const DDL: &str = "CREATE TABLE IF NOT EXISTS connections (
    name           TEXT PRIMARY KEY,
    connection_id  BLOB NOT NULL,
    transport_kind TEXT NOT NULL,
    endpoint       TEXT NOT NULL,
    args_json      TEXT NOT NULL DEFAULT '[]',
    tls_required   INTEGER NOT NULL DEFAULT 0,
    credential_ref TEXT,
    health         TEXT NOT NULL DEFAULT 'unknown',
    tool_count     INTEGER NOT NULL DEFAULT 0,
    session_mode   TEXT NOT NULL DEFAULT 'stateless'
);";

/// A durable SQLite store of registered external MCP server connections.
pub struct SqliteConnectionStore {
    conn: Mutex<SqliteConn>,
}

impl std::fmt::Debug for SqliteConnectionStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqliteConnectionStore")
            .finish_non_exhaustive()
    }
}

impl SqliteConnectionStore {
    /// Open (creating if absent) the connections sidecar at `path`.
    ///
    /// # Errors
    /// [`GatewayError::Storage`] on a SQLite / schema-mismatch failure.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, GatewayError> {
        let conn = SqliteConn::open_with_flags(
            path.as_ref(),
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(storage)?;
        Self::init(conn)
    }

    /// Open an ephemeral in-memory store (tests).
    ///
    /// # Errors
    /// [`GatewayError::Storage`] on a SQLite failure.
    pub fn open_in_memory() -> Result<Self, GatewayError> {
        let conn = SqliteConn::open_in_memory().map_err(storage)?;
        Self::init(conn)
    }

    fn init(conn: SqliteConn) -> Result<Self, GatewayError> {
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = FULL;
             PRAGMA temp_store = MEMORY;",
        )
        .map_err(storage)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS metadata (key TEXT PRIMARY KEY, value BLOB NOT NULL);",
        )
        .map_err(storage)?;
        conn.execute_batch(DDL).map_err(storage)?;
        let ver = CONNECTIONS_SCHEMA_VERSION.to_le_bytes();
        conn.execute(
            "INSERT OR IGNORE INTO metadata (key, value) VALUES ('schema_version', ?1)",
            params![&ver[..]],
        )
        .map_err(storage)?;
        let stored: Vec<u8> = conn
            .query_row(
                "SELECT value FROM metadata WHERE key = 'schema_version'",
                [],
                |r| r.get(0),
            )
            .map_err(storage)?;
        let found = if stored.len() == 2 {
            u16::from_le_bytes([stored[0], stored[1]])
        } else {
            return Err(GatewayError::Storage(
                "connections.db metadata.schema_version is not a 2-byte u16".to_string(),
            ));
        };
        // A NEWER on-disk schema than this binary understands is refused (a lossy
        // downgrade is never safe). An OLDER schema is FORWARD-MIGRATED in place.
        if found > CONNECTIONS_SCHEMA_VERSION {
            return Err(GatewayError::Storage(format!(
                "connections.db schema_version {found} is newer than this binary supports ({CONNECTIONS_SCHEMA_VERSION}) — refusing a lossy downgrade"
            )));
        }
        // PR-6b-3 v1 → v2: add `session_mode` (idempotent — a no-op on a fresh v2
        // table the DDL already created, and on re-open). Existing rows survive,
        // defaulting to the stateless-first posture.
        ensure_session_mode_column(&conn)?;
        if found < CONNECTIONS_SCHEMA_VERSION {
            conn.execute(
                "UPDATE metadata SET value = ?1 WHERE key = 'schema_version'",
                params![&CONNECTIONS_SCHEMA_VERSION.to_le_bytes()[..]],
            )
            .map_err(storage)?;
        }
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Insert or replace a connection (keyed by `name`). The `connection_id` is
    /// re-derived server-side from the name (SN-8).
    ///
    /// # Errors
    /// [`GatewayError::Storage`] on a durable-write failure.
    pub fn upsert(&self, conn: &Connection) -> Result<(), GatewayError> {
        let (kind, endpoint, args_json, tls) = encode_transport(&conn.transport)?;
        let db = self.conn.lock().map_err(|_| poisoned())?;
        db.execute(
            "INSERT OR REPLACE INTO connections
             (name, connection_id, transport_kind, endpoint, args_json, tls_required, credential_ref, health, tool_count, session_mode)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                conn.name,
                &conn.id[..],
                kind,
                endpoint,
                args_json,
                i64::from(tls),
                conn.credential_ref,
                conn.health.tag(),
                conn.tool_count,
                conn.session_mode.tag(),
            ],
        )
        .map_err(storage)?;
        Ok(())
    }

    /// Update the folded health + tool count for `name` (no-op if absent).
    ///
    /// # Errors
    /// [`GatewayError::Storage`] on a durable-write failure.
    pub fn set_health(
        &self,
        name: &str,
        health: ConnectionHealth,
        tool_count: u32,
    ) -> Result<(), GatewayError> {
        let db = self.conn.lock().map_err(|_| poisoned())?;
        db.execute(
            "UPDATE connections SET health = ?2, tool_count = ?3 WHERE name = ?1",
            params![name, health.tag(), tool_count],
        )
        .map_err(storage)?;
        Ok(())
    }

    /// Fetch one connection by name.
    ///
    /// # Errors
    /// [`GatewayError::Storage`] on a SQLite failure.
    pub fn get(&self, name: &str) -> Result<Option<Connection>, GatewayError> {
        let db = self.conn.lock().map_err(|_| poisoned())?;
        let raw = db
            .query_row(
                "SELECT name, transport_kind, endpoint, args_json, tls_required, credential_ref, health, tool_count, session_mode
                 FROM connections WHERE name = ?1",
                params![name],
                read_raw,
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(storage(other)),
            })?;
        raw.map(decode_raw).transpose()
    }

    /// List all connections, ordered by name (deterministic).
    ///
    /// # Errors
    /// [`GatewayError::Storage`] on a SQLite failure.
    pub fn list(&self) -> Result<Vec<Connection>, GatewayError> {
        let db = self.conn.lock().map_err(|_| poisoned())?;
        let mut stmt = db
            .prepare(
                "SELECT name, transport_kind, endpoint, args_json, tls_required, credential_ref, health, tool_count, session_mode
                 FROM connections ORDER BY name ASC",
            )
            .map_err(storage)?;
        let raws = stmt
            .query_map([], read_raw)
            .map_err(storage)?
            .collect::<Result<Vec<RawRow>, _>>()
            .map_err(storage)?;
        raws.into_iter().map(decode_raw).collect()
    }

    /// Remove a connection by name. Returns `true` iff a row was deleted.
    ///
    /// # Errors
    /// [`GatewayError::Storage`] on a durable-write failure.
    pub fn remove(&self, name: &str) -> Result<bool, GatewayError> {
        let db = self.conn.lock().map_err(|_| poisoned())?;
        let n = db
            .execute("DELETE FROM connections WHERE name = ?1", params![name])
            .map_err(storage)?;
        Ok(n > 0)
    }
}

/// Encode a transport into the four stored columns.
fn encode_transport(
    t: &TransportSpec,
) -> Result<(&'static str, String, String, bool), GatewayError> {
    match t {
        TransportSpec::Stdio { command, args } => {
            let args_json = serde_json::to_string(args)
                .map_err(|e| GatewayError::Storage(format!("encode args: {e}")))?;
            Ok(("stdio", command.clone(), args_json, false))
        }
        TransportSpec::Http { url, tls_required } => {
            Ok(("http", url.clone(), "[]".to_string(), *tls_required))
        }
    }
}

/// The raw, undecoded column tuple read from a row (rusqlite layer only).
struct RawRow {
    name: String,
    kind: String,
    endpoint: String,
    args_json: String,
    tls_required: bool,
    credential_ref: Option<String>,
    health: String,
    tool_count: u32,
    session_mode: String,
}

/// Read a row into [`RawRow`] (pure rusqlite — no `GatewayError` here).
fn read_raw(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawRow> {
    let tls_required: i64 = row.get(4)?;
    Ok(RawRow {
        name: row.get(0)?,
        kind: row.get(1)?,
        endpoint: row.get(2)?,
        args_json: row.get(3)?,
        tls_required: tls_required != 0,
        credential_ref: row.get(5)?,
        health: row.get(6)?,
        tool_count: row.get(7)?,
        session_mode: row.get(8)?,
    })
}

/// PR-6b-3: idempotently add the `session_mode` column to an existing v1
/// `connections` table (SQLite has no `ADD COLUMN IF NOT EXISTS`, so guard via
/// `PRAGMA table_info`). A no-op on a fresh v2 table (the DDL already has it) and
/// on every re-open — existing rows survive, defaulting to `'stateless'`.
fn ensure_session_mode_column(conn: &SqliteConn) -> Result<(), GatewayError> {
    let mut stmt = conn
        .prepare("PRAGMA table_info(connections)")
        .map_err(storage)?;
    let has_column = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(storage)?
        .collect::<Result<Vec<String>, _>>()
        .map_err(storage)?
        .iter()
        .any(|name| name == "session_mode");
    if !has_column {
        conn.execute_batch(
            "ALTER TABLE connections ADD COLUMN session_mode TEXT NOT NULL DEFAULT 'stateless';",
        )
        .map_err(storage)?;
    }
    Ok(())
}

/// Decode a [`RawRow`] into a [`Connection`] (the `connection_id` re-derives from
/// the name, server-side). Fail-closed on an unknown transport kind / bad args.
fn decode_raw(raw: RawRow) -> Result<Connection, GatewayError> {
    let transport = match raw.kind.as_str() {
        "stdio" => {
            let args: Vec<String> = serde_json::from_str(&raw.args_json)
                .map_err(|e| GatewayError::Storage(format!("decode args: {e}")))?;
            TransportSpec::Stdio {
                command: raw.endpoint,
                args,
            }
        }
        "http" => TransportSpec::Http {
            url: raw.endpoint,
            tls_required: raw.tls_required,
        },
        other => {
            return Err(GatewayError::Storage(format!(
                "unknown transport_kind {other:?} in connections.db"
            )))
        }
    };
    Ok(Connection {
        id: connection_id_of(&raw.name),
        name: raw.name,
        transport,
        credential_ref: raw.credential_ref,
        health: ConnectionHealth::from_tag(&raw.health),
        tool_count: raw.tool_count,
        session_mode: SessionMode::from_tag(&raw.session_mode),
    })
}

fn storage<E: std::fmt::Display>(e: E) -> GatewayError {
    GatewayError::Storage(e.to_string())
}

fn poisoned() -> GatewayError {
    GatewayError::Storage("connections.db mutex poisoned".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn http(name: &str, url: &str) -> Connection {
        Connection {
            id: connection_id_of(name),
            name: name.to_string(),
            transport: TransportSpec::Http {
                url: url.to_string(),
                tls_required: true,
            },
            credential_ref: Some("MCP_TOKEN".to_string()),
            health: ConnectionHealth::Connected,
            tool_count: 3,
            session_mode: SessionMode::Stateless,
        }
    }

    #[test]
    fn upsert_get_list_remove_roundtrip() {
        let store = SqliteConnectionStore::open_in_memory().unwrap();
        store
            .upsert(&http("github", "https://mcp.github.example/rpc"))
            .unwrap();
        store
            .upsert(&Connection {
                id: connection_id_of("local"),
                name: "local".into(),
                transport: TransportSpec::Stdio {
                    command: "my-server".into(),
                    args: vec!["--stdio".into(), "-v".into()],
                },
                credential_ref: None,
                health: ConnectionHealth::Unknown,
                tool_count: 0,
                session_mode: SessionMode::Stateful,
            })
            .unwrap();

        let got = store.get("github").unwrap().unwrap();
        assert_eq!(got.name, "github");
        assert_eq!(got.credential_ref.as_deref(), Some("MCP_TOKEN"));
        assert_eq!(got.health, ConnectionHealth::Connected);
        assert_eq!(got.tool_count, 3);
        assert_eq!(got.egress_host().as_deref(), Some("mcp.github.example"));
        // PR-6b-3: session_mode round-trips (default stateless for the http helper).
        assert_eq!(got.session_mode, SessionMode::Stateless);

        let local = store.get("local").unwrap().unwrap();
        match local.transport {
            TransportSpec::Stdio { command, args } => {
                assert_eq!(command, "my-server");
                assert_eq!(args, vec!["--stdio".to_string(), "-v".to_string()]);
            }
            TransportSpec::Http { .. } => panic!("expected stdio"),
        }
        // The explicitly-stateful stdio server round-trips as stateful.
        assert_eq!(local.session_mode, SessionMode::Stateful);

        // Deterministic ordering by name.
        let all = store.list().unwrap();
        assert_eq!(
            all.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
            vec!["github", "local"]
        );

        assert!(store.remove("github").unwrap());
        assert!(!store.remove("github").unwrap());
        assert!(store.get("github").unwrap().is_none());
    }

    #[test]
    fn migrates_v1_db_to_v2_preserving_rows() {
        use rusqlite::{params, Connection as SqliteConn};
        // Build a v1-shaped connections.db on disk (no session_mode column, version 1).
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("connections.db");
        {
            let c = SqliteConn::open(&path).unwrap();
            c.execute_batch(
                "CREATE TABLE metadata (key TEXT PRIMARY KEY, value BLOB NOT NULL);
                 CREATE TABLE connections (
                    name TEXT PRIMARY KEY, connection_id BLOB NOT NULL, transport_kind TEXT NOT NULL,
                    endpoint TEXT NOT NULL, args_json TEXT NOT NULL DEFAULT '[]',
                    tls_required INTEGER NOT NULL DEFAULT 0, credential_ref TEXT,
                    health TEXT NOT NULL DEFAULT 'unknown', tool_count INTEGER NOT NULL DEFAULT 0);",
            )
            .unwrap();
            let one: u16 = 1;
            c.execute(
                "INSERT INTO metadata (key, value) VALUES ('schema_version', ?1)",
                params![&one.to_le_bytes()[..]],
            )
            .unwrap();
            c.execute(
                "INSERT INTO connections (name, connection_id, transport_kind, endpoint, health, tool_count)
                 VALUES ('legacy', ?1, 'http', 'https://a.example/rpc', 'connected', 2)",
                params![&connection_id_of("legacy")[..]],
            )
            .unwrap();
        }
        // Re-open through the v2 store: the forward migration must preserve the row,
        // default its session_mode to stateless, and stamp schema_version = 2.
        let store = SqliteConnectionStore::open(&path).unwrap();
        let got = store.get("legacy").unwrap().expect("legacy row preserved");
        assert_eq!(got.tool_count, 2);
        assert_eq!(got.health, ConnectionHealth::Connected);
        assert_eq!(got.session_mode, SessionMode::Stateless);
        // Re-open AGAIN: the idempotent ALTER must be a no-op (no duplicate-column error).
        let store2 = SqliteConnectionStore::open(&path).unwrap();
        assert!(store2.get("legacy").unwrap().is_some());
    }

    #[test]
    fn upsert_replaces_and_set_health_updates() {
        let store = SqliteConnectionStore::open_in_memory().unwrap();
        store.upsert(&http("c", "https://a.example/rpc")).unwrap();
        store
            .set_health("c", ConnectionHealth::Unreachable, 0)
            .unwrap();
        let got = store.get("c").unwrap().unwrap();
        assert_eq!(got.health, ConnectionHealth::Unreachable);
        assert_eq!(got.tool_count, 0);
    }
}
