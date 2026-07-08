//! The POC-4 App-catalog sidecar: `apps.db` under `--catalog-dir`, backing the
//! [`AppCatalog`] seam — `SaveApp` / `ListApps` / `GetApp`.
//!
//! ## Rebuildable-to-EMPTY (the `bundles.db` posture)
//! An App envelope references content-store blobs + registry ids; it is NOT
//! derivable from the journal. Truth (the referenced blobs + the registries) lives
//! elsewhere, so on corruption or a schema-version drift this ledger recreates
//! EMPTY — the only loss is the catalog index, and re-saving the same envelope
//! restores the SAME `app_ref` (content-addressed). Never journaled, never a
//! `MoteId` input, never a digest input — dropping the file cannot move the
//! canonical projection digest.
//!
//! ## Server-derived id (SN-8)
//! `app_ref = blake3("kx-app\0" ‖ handle ‖ 0 ‖ canonical(envelope))[..16]` via
//! [`kx_content::ContentRef::of`] (the `bundle_ref_of` precedent). The host
//! RE-CANONICALIZES the received bytes ([`kx_app::canonical_json`]) so client
//! byte-ordering never affects identity, and validates the envelope — it carries
//! NO authority (a bad envelope ⇒ `InvalidArgument`).
//!
//! ## Caller-scoped
//! The primary key is `(principal, handle)` — an App is visible only to the
//! SERVER-RESOLVED party that authored it (uniform not-found for absent OR
//! not-owned; no cross-party existence oracle).

use std::path::Path;
use std::sync::Mutex;

use kx_gateway_core::GatewayError as CoreError;
use kx_gateway_core::{AppCatalog, AppRecord};
use rusqlite::{params, Connection};

use crate::error::GatewayError;

/// Bump on any table-shape change. Unknown/missing version ⇒ recreate EMPTY
/// (apps are not journal-derivable, so there is no rebuild — re-save).
const SCHEMA_VERSION: i64 = 1;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS apps (
    principal     TEXT NOT NULL,   -- server-resolved caller party (scope)
    handle        TEXT NOT NULL,   -- AssetPath 'ns/coll/name' (upsert key within principal)
    app_ref       BLOB NOT NULL,   -- 16B server-derived canonical-envelope hash (display/dedup)
    name          TEXT NOT NULL,   -- envelope name (denormalized summary)
    version       TEXT NOT NULL,   -- envelope version
    description   TEXT NOT NULL,   -- advisory, never parsed for enforcement
    tags_json     TEXT NOT NULL,   -- JSON [string] (denormalized summary)
    step_count    INTEGER NOT NULL,-- blueprint step count (display)
    envelope_json TEXT NOT NULL,   -- the CANONICAL kortecx.app/v1 envelope bytes
    PRIMARY KEY (principal, handle)
);
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value INTEGER NOT NULL);
";

/// `app_ref = blake3("kx-app\0" ‖ handle ‖ 0 ‖ canonical_envelope)[..16]` (SN-8).
fn app_ref_of(handle: &str, canonical: &[u8]) -> [u8; 16] {
    let mut keyed = Vec::with_capacity(16 + handle.len() + canonical.len());
    keyed.extend_from_slice(b"kx-app\0");
    keyed.extend_from_slice(handle.as_bytes());
    keyed.push(0);
    keyed.extend_from_slice(canonical);
    let mut id = [0u8; 16];
    id.copy_from_slice(&kx_content::ContentRef::of(&keyed).0[..16]);
    id
}

/// The durable App catalog over `apps.db`. A single mutex'd connection: App
/// authoring is interactive-rate (a CLI/SDK save / a catalog list), never contended.
pub(crate) struct AppsDb {
    conn: Mutex<Connection>,
}

impl AppsDb {
    /// Open (or create) `apps.db` under `dir`. A corrupt/foreign file or a
    /// `schema_version` drift recreates the catalog EMPTY (module doc).
    ///
    /// # Errors
    /// [`GatewayError::Catalog`] on an unrecoverable open/pragma failure.
    pub(crate) fn open(dir: &Path) -> Result<Self, GatewayError> {
        std::fs::create_dir_all(dir)
            .map_err(|e| GatewayError::Catalog(format!("apps dir: {e}")))?;
        let db_path = dir.join("apps.db");
        let conn = if let Ok(c) = Self::open_with_pragma(&db_path) {
            c
        } else {
            let _ = std::fs::remove_file(&db_path);
            let _ = std::fs::remove_file(dir.join("apps.db-wal"));
            let _ = std::fs::remove_file(dir.join("apps.db-shm"));
            Self::open_with_pragma(&db_path)
                .map_err(|e| GatewayError::Catalog(format!("apps reopen: {e}")))?
        };
        let fresh_or_stale = match Self::read_schema_version(&conn) {
            Ok(Some(v)) => v != SCHEMA_VERSION,
            Ok(None) | Err(_) => true,
        };
        if fresh_or_stale {
            conn.execute_batch("DROP TABLE IF EXISTS apps; DROP TABLE IF EXISTS meta;")
                .map_err(|e| GatewayError::Catalog(format!("apps rebuild: {e}")))?;
        }
        conn.execute_batch(SCHEMA)
            .map_err(|e| GatewayError::Catalog(format!("apps schema: {e}")))?;
        conn.execute(
            "INSERT OR IGNORE INTO meta(key, value) VALUES ('schema_version', ?1)",
            params![SCHEMA_VERSION],
        )
        .map_err(|e| GatewayError::Catalog(format!("apps meta init: {e}")))?;
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

    fn row_to_record(
        handle: String,
        app_ref: &[u8],
        name: String,
        version: String,
        description: String,
        tags_json: &str,
        step_count: i64,
    ) -> AppRecord {
        let mut id = [0u8; 16];
        let n = app_ref.len().min(16);
        id[..n].copy_from_slice(&app_ref[..n]);
        AppRecord {
            app_ref: id,
            handle,
            name,
            version,
            description,
            tags: serde_json::from_str(tags_json).unwrap_or_default(),
            step_count: u32::try_from(step_count).unwrap_or(u32::MAX),
        }
    }
}

impl AppCatalog for AppsDb {
    fn save(
        &self,
        principal: &str,
        handle: &str,
        envelope_json: &[u8],
    ) -> Result<(AppRecord, bool), CoreError> {
        // Validate + canonicalize the envelope (it carries NO authority); a bad
        // envelope is a client error, not an internal one.
        let canonical = kx_app::canonical_json(envelope_json)
            .map_err(|_| CoreError::InvalidArgument("invalid app envelope"))?;
        let summary = kx_app::summary_of(envelope_json)
            .map_err(|_| CoreError::InvalidArgument("invalid app envelope"))?;
        let app_ref = app_ref_of(handle, &canonical);
        let canonical_str = String::from_utf8(canonical)
            .map_err(|_| CoreError::Internal("canonical envelope is not UTF-8".into()))?;
        let tags_json = serde_json::to_string(&summary.tags)
            .map_err(|e| CoreError::Internal(format!("apps tags encode: {e}")))?;
        let conn = self
            .conn
            .lock()
            .map_err(|_| CoreError::Internal("apps lock poisoned".into()))?;
        // Dedup signal: an identical canonical envelope already bound to (principal, handle).
        let existing: Option<Vec<u8>> = conn
            .query_row(
                "SELECT app_ref FROM apps WHERE principal = ?1 AND handle = ?2",
                params![principal, handle],
                |r| r.get(0),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })
            .map_err(|e| CoreError::Internal(format!("apps dedup probe: {e}")))?;
        let deduplicated = existing.as_deref() == Some(&app_ref[..]);
        conn.execute(
            "INSERT OR REPLACE INTO apps(principal, handle, app_ref, name, version, description, \
             tags_json, step_count, envelope_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                principal,
                handle,
                app_ref.to_vec(),
                summary.name,
                summary.version,
                summary.description,
                tags_json,
                i64::from(summary.step_count),
                canonical_str,
            ],
        )
        .map_err(|e| CoreError::Internal(format!("apps upsert: {e}")))?;
        Ok((
            AppRecord {
                app_ref,
                handle: handle.to_string(),
                name: summary.name,
                version: summary.version,
                description: summary.description,
                tags: summary.tags,
                step_count: summary.step_count,
            },
            deduplicated,
        ))
    }

    fn list(
        &self,
        principal: &str,
        limit: usize,
        after_handle: Option<&str>,
    ) -> Result<(Vec<AppRecord>, bool), CoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| CoreError::Internal("apps lock poisoned".into()))?;
        let cursor = after_handle.unwrap_or("");
        let mut stmt = conn
            .prepare(
                "SELECT handle, app_ref, name, version, description, tags_json, step_count \
                 FROM apps WHERE principal = ?1 AND handle > ?2 ORDER BY handle ASC LIMIT ?3",
            )
            .map_err(|e| CoreError::Internal(format!("apps list prepare: {e}")))?;
        let fetch = i64::try_from(limit.saturating_add(1)).unwrap_or(i64::MAX);
        let rows = stmt
            .query_map(params![principal, cursor, fetch], |r| {
                let app_ref = r.get::<_, Vec<u8>>(1)?;
                let tags_json = r.get::<_, String>(5)?;
                Ok(Self::row_to_record(
                    r.get::<_, String>(0)?,
                    &app_ref,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    &tags_json,
                    r.get::<_, i64>(6)?,
                ))
            })
            .map_err(|e| CoreError::Internal(format!("apps list query: {e}")))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| CoreError::Internal(format!("apps list row: {e}")))?);
        }
        let has_more = out.len() > limit;
        out.truncate(limit);
        Ok((out, has_more))
    }

    fn get(
        &self,
        principal: &str,
        handle: &str,
    ) -> Result<Option<(AppRecord, Vec<u8>)>, CoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| CoreError::Internal("apps lock poisoned".into()))?;
        conn.query_row(
            "SELECT handle, app_ref, name, version, description, tags_json, step_count, envelope_json \
             FROM apps WHERE principal = ?1 AND handle = ?2",
            params![principal, handle],
            |r| {
                let app_ref = r.get::<_, Vec<u8>>(1)?;
                let tags_json = r.get::<_, String>(5)?;
                let record = Self::row_to_record(
                    r.get::<_, String>(0)?,
                    &app_ref,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    &tags_json,
                    r.get::<_, i64>(6)?,
                );
                let envelope_json = r.get::<_, String>(7)?.into_bytes();
                Ok((record, envelope_json))
            },
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })
        .map_err(|e| CoreError::Internal(format!("apps get: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn envelope(name: &str) -> Vec<u8> {
        // A minimal valid kortecx.app/v1 envelope authored via the type crate.
        let env = kx_app::AppEnvelope::new(name, serde_json::json!({ "steps": [] }));
        env.to_canonical_json().unwrap()
    }

    fn tmp_dir() -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        // unique-enough per test process (no Date/rand needed: pid + addr).
        let stamp = format!("kx-apps-test-{}-{:p}", std::process::id(), &p);
        p.push(stamp);
        p
    }

    #[test]
    fn app_ref_is_handle_scoped_but_app_digest_is_not() {
        use kx_gateway_core::app_digest_of;
        let canonical = envelope("echo-app");
        // Same envelope under two handles: app_ref DIFFERS (the handle is folded in) ...
        assert_ne!(
            app_ref_of("team/apps/a", &canonical),
            app_ref_of("team/apps/b", &canonical)
        );
        // ... but app_digest is the SAME portable, handle-free id (full 32B) and is
        // domain-separated from app_ref (its leading 16B are not the app_ref).
        let digest = app_digest_of(&canonical);
        assert_eq!(digest.len(), 32);
        assert_ne!(&digest[..16], &app_ref_of("team/apps/a", &canonical)[..]);
    }

    #[test]
    fn save_get_list_round_trip() {
        let dir = tmp_dir();
        let db = AppsDb::open(&dir).unwrap();
        let (rec, dedup) = db
            .save("alice", "team/apps/echo", &envelope("echo"))
            .unwrap();
        assert!(!dedup);
        assert_eq!(rec.name, "echo");
        assert_eq!(rec.step_count, 0);
        // identical re-save dedups.
        let (_, dedup2) = db
            .save("alice", "team/apps/echo", &envelope("echo"))
            .unwrap();
        assert!(dedup2);
        // get returns the canonical bytes.
        let (got, bytes) = db.get("alice", "team/apps/echo").unwrap().unwrap();
        assert_eq!(got.app_ref, rec.app_ref);
        assert_eq!(bytes, envelope("echo"));
        // list shows it.
        let (apps, has_more) = db.list("alice", 100, None).unwrap();
        assert_eq!(apps.len(), 1);
        assert!(!has_more);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cross_party_isolation_is_uniform_not_found() {
        let dir = tmp_dir();
        let db = AppsDb::open(&dir).unwrap();
        db.save("alice", "team/apps/secret", &envelope("secret"))
            .unwrap();
        // bob cannot see alice's app (uniform not-found).
        assert!(db.get("bob", "team/apps/secret").unwrap().is_none());
        let (apps, _) = db.list("bob", 100, None).unwrap();
        assert!(apps.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_bad_envelope_is_invalid_argument() {
        let dir = tmp_dir();
        let db = AppsDb::open(&dir).unwrap();
        let err = db.save("alice", "team/apps/bad", b"{not json").unwrap_err();
        assert!(matches!(err, CoreError::InvalidArgument(_)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn schema_drift_rebuilds_empty() {
        let dir = tmp_dir();
        {
            let db = AppsDb::open(&dir).unwrap();
            db.save("alice", "team/apps/x", &envelope("x")).unwrap();
        }
        // Corrupt the meta version → reopen recreates EMPTY (rebuildable-to-empty).
        {
            let conn = Connection::open(dir.join("apps.db")).unwrap();
            conn.execute(
                "UPDATE meta SET value = 999 WHERE key = 'schema_version'",
                [],
            )
            .unwrap();
        }
        let db = AppsDb::open(&dir).unwrap();
        let (apps, _) = db.list("alice", 100, None).unwrap();
        assert!(apps.is_empty(), "schema drift must rebuild empty");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
