//! MM-3 (D110): the LOCAL OS-keychain secret store + the `SecretAdmin` write surface.
//!
//! Today a connector's `credential_ref` resolves only against a host environment
//! variable ([`kx_mcp::EnvSecretStore`]). MM-3 adds a LOCAL secret store backed by
//! the OS keychain (macOS Keychain / Windows Credential Manager / Linux kernel
//! keyutils) so a local agent can authenticate real services without exporting
//! credentials into the process environment.
//!
//! ## Security posture (D81/D110, SN-8)
//! - Secrets are resolved **by NAME** ([`kx_warrant::SecretRef`]); the value is read
//!   transiently at transport setup, injected into a header / child env, and dropped.
//!   It is NEVER journaled, in a `MoteId`/`StepRecord`, or the model's context. The
//!   broker `secret_scope` precheck (`kx-capability`) remains the sole authorization
//!   gate — this module is the resolve/store MECHANISM only.
//! - [`KeyringSecretStore`] is the keychain arm of the host `ChainedSecretStore
//!   { keychain → env }`: a name in the keychain wins; a name present only in the
//!   environment still resolves (pre-MM-3 back-compat).
//! - OS keychains are not portably enumerable through a single key handle, so the
//!   NAME index for `ListSecretNames` lives in an off-journal `secret_index.db`
//!   sidecar (NAMES + timestamps only, never a value). [`KeyringSecretStore::resolve`]
//!   reads the keychain directly and never consults the index, so a lost/rebuilt
//!   index degrades ENUMERATION only, never resolution.
//!
//! The keychain is fronted by a small [`KeychainBackend`] seam so the store + admin
//! are unit-testable headless (an in-memory backend) without touching a developer's
//! real keychain — the production backend ([`OsKeychain`]) is a thin `keyring` wrapper.
//!
//! The hardened multi-tenant KMS/HSM vault (rotation, audit, envelope encryption) is a
//! `kx-cloud` concern behind the same `SecretStore` seam (D94); OSS ships the honest
//! local store and makes no "best-cryptography vault" claim.

use std::path::Path;
use std::sync::{Arc, Mutex};

use kx_gateway_core::{SecretAdmin, SecretAdminError, SecretNameView};
use rusqlite::{params, Connection};

use crate::error::GatewayError;

/// The OS-keychain service namespace under which every kortecx secret is stored, so
/// kortecx items never collide with other applications' keychain entries.
const KEYCHAIN_SERVICE: &str = "kortecx";

/// Default / maximum `ListSecretNames` page size.
const DEFAULT_LIST_LIMIT: u32 = 200;
const MAX_LIST_LIMIT: u32 = 1000;

/// The keychain seam: get/set/delete a secret value by NAME. Fronts the OS keychain
/// in production ([`OsKeychain`]) and an in-memory map in tests — so the store + admin
/// are testable headless without a real keychain backend.
pub(crate) trait KeychainBackend: Send + Sync {
    /// Read a value by name. `Ok(None)` ⇒ absent; `Err(Unavailable)` ⇒ no backend.
    fn get(&self, name: &str) -> Result<Option<String>, SecretAdminError>;
    /// Store (or overwrite) a value by name.
    fn set(&self, name: &str, value: &str) -> Result<(), SecretAdminError>;
    /// Delete a value by name. `Ok(true)` iff an entry existed.
    fn delete(&self, name: &str) -> Result<bool, SecretAdminError>;
}

/// The production keychain backend — a thin `keyring` wrapper. Stateless: each call
/// opens a fresh OS keychain handle, performs the op, and drops the value.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct OsKeychain;

impl KeychainBackend for OsKeychain {
    fn get(&self, name: &str) -> Result<Option<String>, SecretAdminError> {
        let entry = keyring::Entry::new(KEYCHAIN_SERVICE, name)
            .map_err(|e| SecretAdminError::Unavailable(e.to_string()))?;
        match entry.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(SecretAdminError::Storage(e.to_string())),
        }
    }

    fn set(&self, name: &str, value: &str) -> Result<(), SecretAdminError> {
        let entry = keyring::Entry::new(KEYCHAIN_SERVICE, name)
            .map_err(|e| SecretAdminError::Unavailable(e.to_string()))?;
        entry
            .set_password(value)
            .map_err(|e| SecretAdminError::Storage(e.to_string()))
    }

    fn delete(&self, name: &str) -> Result<bool, SecretAdminError> {
        let entry = keyring::Entry::new(KEYCHAIN_SERVICE, name)
            .map_err(|e| SecretAdminError::Unavailable(e.to_string()))?;
        match entry.delete_credential() {
            Ok(()) => Ok(true),
            Err(keyring::Error::NoEntry) => Ok(false),
            Err(e) => Err(SecretAdminError::Storage(e.to_string())),
        }
    }
}

/// The keychain arm of the host secret resolver (MM-3). Implements the `kx-mcp`
/// [`SecretStore`] seam so a connection's `credential_ref` NAME resolves from the OS
/// keychain at transport setup. Gated on `mcp-gateway` because that is the feature
/// under which `kx-mcp` (and the `SecretStore` trait it provides) is in the build.
///
/// [`SecretStore`]: kx_mcp::SecretStore
#[cfg(feature = "mcp-gateway")]
#[derive(Clone)]
pub(crate) struct KeyringSecretStore {
    backend: Arc<dyn KeychainBackend>,
}

#[cfg(feature = "mcp-gateway")]
impl KeyringSecretStore {
    /// Build over the OS keychain (the production backend).
    pub(crate) fn os() -> Self {
        Self {
            backend: Arc::new(OsKeychain),
        }
    }
}

#[cfg(feature = "mcp-gateway")]
impl kx_mcp::SecretStore for KeyringSecretStore {
    fn resolve(&self, secret_ref: &kx_warrant::SecretRef) -> Option<String> {
        // A backend error (no keychain on this host) is an honest miss — the host's
        // ChainedSecretStore then falls back to the environment; never fabricated.
        self.backend.get(&secret_ref.0).ok().flatten()
    }
}

/// Resolve a secret VALUE by NAME through the keychain-then-environment chain — the
/// same precedence the connector transport resolver uses (MM-3). Used by the D113
/// webhook listener to fetch a trigger's HMAC/bearer verify key. `None` ⇒ unresolvable
/// (the webhook then fails closed; never a fabricated credential). Always-on (no
/// `kx-mcp` dependency), so the trigger listener can verify auth in any build.
pub(crate) fn resolve_secret_value(name: &str) -> Option<String> {
    OsKeychain
        .get(name)
        .ok()
        .flatten()
        .or_else(|| std::env::var(name).ok())
}

/// Wall-clock ms since epoch (off-digest; advisory timestamps only).
fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// Bump on any table-shape change. Unknown/missing ⇒ recreate EMPTY. The index is a
/// NAME convenience cache (resolution reads the keychain directly), so a rebuild loses
/// only enumeration of names stored by an older binary — `put` re-indexes on next write.
const INDEX_SCHEMA_VERSION: i64 = 1;

const INDEX_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS secret_names (
    name            TEXT PRIMARY KEY,   -- the SecretRef NAME (never a value)
    created_unix_ms INTEGER NOT NULL,
    updated_unix_ms INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value INTEGER NOT NULL);
";

/// The keychain-backed [`SecretAdmin`] (MM-3): the OS keychain holds the VALUES; an
/// off-journal `secret_index.db` holds the NAMES (so `ListSecretNames` can enumerate,
/// which the keychain cannot do portably). The index NEVER holds a value.
pub(crate) struct KeychainSecretAdmin {
    backend: Arc<dyn KeychainBackend>,
    index: Mutex<Connection>,
}

impl KeychainSecretAdmin {
    /// Open (or create) the `secret_index.db` NAME index under `dir`, over the OS
    /// keychain backend.
    ///
    /// # Errors
    /// [`GatewayError::Catalog`] on an unrecoverable open/pragma failure.
    pub(crate) fn open(dir: &Path) -> Result<Self, GatewayError> {
        Self::open_with_backend(dir, Arc::new(OsKeychain))
    }

    fn open_with_backend(
        dir: &Path,
        backend: Arc<dyn KeychainBackend>,
    ) -> Result<Self, GatewayError> {
        std::fs::create_dir_all(dir)
            .map_err(|e| GatewayError::Catalog(format!("secret_index dir: {e}")))?;
        let db_path = dir.join("secret_index.db");
        let conn = if let Ok(c) = Self::open_with_pragma(&db_path) {
            c
        } else {
            let _ = std::fs::remove_file(&db_path);
            let _ = std::fs::remove_file(dir.join("secret_index.db-wal"));
            let _ = std::fs::remove_file(dir.join("secret_index.db-shm"));
            Self::open_with_pragma(&db_path)
                .map_err(|e| GatewayError::Catalog(format!("secret_index reopen: {e}")))?
        };
        let fresh_or_stale = match Self::read_schema_version(&conn) {
            Ok(Some(v)) => v != INDEX_SCHEMA_VERSION,
            Ok(None) | Err(_) => true,
        };
        if fresh_or_stale {
            conn.execute_batch(
                "DROP TABLE IF EXISTS secret_names;
                 DROP TABLE IF EXISTS meta;",
            )
            .map_err(|e| GatewayError::Catalog(format!("secret_index rebuild: {e}")))?;
        }
        conn.execute_batch(INDEX_SCHEMA)
            .map_err(|e| GatewayError::Catalog(format!("secret_index schema: {e}")))?;
        conn.execute(
            "INSERT OR IGNORE INTO meta(key, value) VALUES ('schema_version', ?1)",
            params![INDEX_SCHEMA_VERSION],
        )
        .map_err(|e| GatewayError::Catalog(format!("secret_index meta init: {e}")))?;
        Ok(Self {
            backend,
            index: Mutex::new(conn),
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

impl SecretAdmin for KeychainSecretAdmin {
    fn put(&self, name: &str, value: &str) -> Result<(), SecretAdminError> {
        // Write the keychain FIRST (the authoritative store); only then index the NAME.
        self.backend.set(name, value)?;
        let now = now_unix_ms();
        let conn = self
            .index
            .lock()
            .map_err(|_| SecretAdminError::Storage("secret_index lock poisoned".into()))?;
        conn.execute(
            "INSERT INTO secret_names(name, created_unix_ms, updated_unix_ms) VALUES (?1, ?2, ?2)
             ON CONFLICT(name) DO UPDATE SET updated_unix_ms = ?2",
            params![name, i64::try_from(now).unwrap_or(i64::MAX)],
        )
        .map_err(|e| SecretAdminError::Storage(format!("secret_index put: {e}")))?;
        Ok(())
    }

    fn list_names(
        &self,
        limit: u32,
        after_name: &str,
    ) -> Result<(Vec<SecretNameView>, bool), SecretAdminError> {
        let lim = match limit {
            0 => DEFAULT_LIST_LIMIT,
            n => n.min(MAX_LIST_LIMIT),
        };
        let conn = self
            .index
            .lock()
            .map_err(|_| SecretAdminError::Storage("secret_index lock poisoned".into()))?;
        // Fetch lim+1 to detect has_more without a second query.
        let fetch = i64::from(lim) + 1;
        let mut stmt = conn
            .prepare(
                "SELECT name, created_unix_ms, updated_unix_ms FROM secret_names \
                 WHERE name > ?1 ORDER BY name ASC LIMIT ?2",
            )
            .map_err(|e| SecretAdminError::Storage(format!("secret_index list prepare: {e}")))?;
        let mut rows: Vec<SecretNameView> = stmt
            .query_map(params![after_name, fetch], |r| {
                Ok(SecretNameView {
                    name: r.get::<_, String>(0)?,
                    created_unix_ms: u64::try_from(r.get::<_, i64>(1)?).unwrap_or(0),
                    updated_unix_ms: u64::try_from(r.get::<_, i64>(2)?).unwrap_or(0),
                })
            })
            .map_err(|e| SecretAdminError::Storage(format!("secret_index list: {e}")))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| SecretAdminError::Storage(format!("secret_index list row: {e}")))?;
        let has_more = rows.len() > lim as usize;
        rows.truncate(lim as usize);
        Ok((rows, has_more))
    }

    fn delete(&self, name: &str) -> Result<bool, SecretAdminError> {
        // Delete the keychain entry (authoritative) AND the index row; the secret is
        // "removed" if EITHER existed (an index-only or keychain-only remnant still counts).
        let keychain_had = self.backend.delete(name)?;
        let conn = self
            .index
            .lock()
            .map_err(|_| SecretAdminError::Storage("secret_index lock poisoned".into()))?;
        let index_changed = conn
            .execute("DELETE FROM secret_names WHERE name = ?1", params![name])
            .map_err(|e| SecretAdminError::Storage(format!("secret_index delete: {e}")))?;
        Ok(keychain_had || index_changed > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// In-memory keychain backend for headless tests (no real OS keychain touched).
    #[derive(Default)]
    struct MemKeychain(Mutex<BTreeMap<String, String>>);
    impl KeychainBackend for MemKeychain {
        fn get(&self, name: &str) -> Result<Option<String>, SecretAdminError> {
            Ok(self.0.lock().unwrap().get(name).cloned())
        }
        fn set(&self, name: &str, value: &str) -> Result<(), SecretAdminError> {
            self.0
                .lock()
                .unwrap()
                .insert(name.to_string(), value.to_string());
            Ok(())
        }
        fn delete(&self, name: &str) -> Result<bool, SecretAdminError> {
            Ok(self.0.lock().unwrap().remove(name).is_some())
        }
    }

    fn admin(dir: &Path) -> (KeychainSecretAdmin, Arc<MemKeychain>) {
        let mem = Arc::new(MemKeychain::default());
        let admin = KeychainSecretAdmin::open_with_backend(dir, mem.clone()).unwrap();
        (admin, mem)
    }

    #[test]
    fn put_indexes_name_and_stores_value_in_backend() {
        let dir = tempfile::tempdir().unwrap();
        let (a, mem) = admin(dir.path());
        a.put("GITHUB_TOKEN", "ghp_secret").unwrap();
        // The VALUE lives only in the keychain backend, never in the index DB.
        assert_eq!(
            mem.0.lock().unwrap().get("GITHUB_TOKEN").unwrap(),
            "ghp_secret"
        );
        let (names, has_more) = a.list_names(0, "").unwrap();
        assert_eq!(names.len(), 1);
        assert_eq!(names[0].name, "GITHUB_TOKEN");
        assert!(!has_more);
    }

    #[test]
    fn secret_value_never_written_to_the_index_db() {
        let dir = tempfile::tempdir().unwrap();
        {
            let (a, _) = admin(dir.path());
            a.put("API_KEY", "TOP-SECRET-VALUE-12345").unwrap();
        }
        // Scan every byte of the on-disk index sidecar — the value must be ABSENT.
        for suffix in ["", "-wal", "-shm"] {
            let p = dir.path().join(format!("secret_index.db{suffix}"));
            if let Ok(bytes) = std::fs::read(&p) {
                assert!(
                    !bytes
                        .windows(b"TOP-SECRET-VALUE-12345".len())
                        .any(|w| w == b"TOP-SECRET-VALUE-12345"),
                    "secret value leaked into {p:?}"
                );
            }
        }
    }

    #[test]
    fn list_names_keyset_pages_and_reports_has_more() {
        let dir = tempfile::tempdir().unwrap();
        let (a, _) = admin(dir.path());
        for n in ["a", "b", "c", "d"] {
            a.put(n, "v").unwrap();
        }
        let (page1, has_more) = a.list_names(2, "").unwrap();
        assert_eq!(
            page1.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
            ["a", "b"]
        );
        assert!(has_more);
        let (page2, has_more2) = a.list_names(2, "b").unwrap();
        assert_eq!(
            page2.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
            ["c", "d"]
        );
        assert!(!has_more2);
    }

    #[test]
    fn delete_removes_from_backend_and_index() {
        let dir = tempfile::tempdir().unwrap();
        let (a, mem) = admin(dir.path());
        a.put("TOK", "v").unwrap();
        assert!(a.delete("TOK").unwrap(), "existing secret reports removed");
        assert!(mem.0.lock().unwrap().get("TOK").is_none());
        assert!(a.list_names(0, "").unwrap().0.is_empty());
        assert!(
            !a.delete("TOK").unwrap(),
            "absent secret reports not-removed"
        );
    }

    #[test]
    fn put_overwrites_value_keeps_created_ms() {
        let dir = tempfile::tempdir().unwrap();
        let (a, mem) = admin(dir.path());
        a.put("K", "v1").unwrap();
        let created = a.list_names(0, "").unwrap().0[0].created_unix_ms;
        a.put("K", "v2").unwrap();
        assert_eq!(
            mem.0.lock().unwrap().get("K").unwrap(),
            "v2",
            "value overwritten"
        );
        let row = a.list_names(0, "").unwrap().0;
        assert_eq!(row.len(), 1, "still one NAME");
        assert_eq!(
            row[0].created_unix_ms, created,
            "created_ms preserved on overwrite"
        );
    }

    #[test]
    fn corrupt_index_recreates_empty() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("secret_index.db"), b"not a sqlite file").unwrap();
        let (a, _) = admin(dir.path());
        assert!(a.list_names(0, "").unwrap().0.is_empty());
        a.put("X", "v").unwrap();
        assert_eq!(a.list_names(0, "").unwrap().0.len(), 1);
    }
}
