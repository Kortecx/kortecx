//! The PR-7 context-bundle sidecar: `bundles.db` under `--catalog-dir`, backing
//! the [`BundleStore`] seam — `PutContextBundle`'s manifests + the bind-time
//! resolution of the `context_bundles` field on Invoke / SubmitWorkflow.
//!
//! ## Rebuildable-to-EMPTY (the `uploads.db` posture)
//! A bundle manifest records which content-store blobs a caller grouped under a
//! handle; it is NOT derivable from the journal. Truth (the blobs) lives in the
//! content store, so on corruption or a schema-version drift this ledger recreates
//! EMPTY — the only loss is the manifest index, and re-authoring the same items
//! restores the SAME `bundle_ref` (content-addressed). Never journaled, never a
//! `MoteId` input, never a digest input — dropping the file cannot move the
//! canonical projection digest.
//!
//! ## Server-derived id (SN-8)
//! `bundle_ref = blake3("kx-bundle\0" ‖ handle ‖ items)[..16]` via
//! [`kx_content::ContentRef::of`] (the `alerts.db` keyed-hash precedent). The
//! client names a `handle`; the server derives the identity.
//!
//! ## Caller-scoped
//! The primary key is `(principal, handle)` — a bundle is visible only to the
//! SERVER-RESOLVED party that authored it (uniform not-found for absent OR
//! not-owned; no cross-party existence oracle).

use std::path::Path;
use std::sync::Mutex;

use kx_gateway_core::GatewayError as CoreError;
use kx_gateway_core::{BundleItemRecord, BundleManifest, BundleStore};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::error::GatewayError;

/// Bump on any table-shape change. Unknown/missing version ⇒ recreate EMPTY
/// (bundles are not journal-derivable, so there is no rebuild — re-author).
const SCHEMA_VERSION: i64 = 1;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS bundles (
    principal   TEXT NOT NULL,   -- server-resolved caller party (scope)
    handle      TEXT NOT NULL,   -- AssetPath 'ns/coll/name' (upsert key within principal)
    bundle_ref  BLOB NOT NULL,   -- 16B server-derived manifest hash (display/dedup)
    description TEXT NOT NULL,    -- advisory, never parsed for enforcement
    items_json  TEXT NOT NULL,   -- JSON [{name, ref(hex), media}], author order
    PRIMARY KEY (principal, handle)
);
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value INTEGER NOT NULL);
";

/// On-disk item row (content_ref carried as 64-char hex in `items_json`).
#[derive(Serialize, Deserialize)]
struct ItemRow {
    name: String,
    #[serde(rename = "ref")]
    ref_hex: String,
    media: String,
}

/// `bundle_ref = blake3("kx-bundle\0" ‖ handle ‖ items)[..16]` (SN-8). Folds the
/// items in AUTHOR order (the manifest's identity reflects the exact sequence the
/// caller bound). The bind-time `config_subset` injection canonicalises separately.
fn bundle_ref_of(handle: &str, items: &[BundleItemRecord]) -> [u8; 16] {
    let mut keyed = Vec::with_capacity(64 + items.len() * 48);
    keyed.extend_from_slice(b"kx-bundle\0");
    keyed.extend_from_slice(handle.as_bytes());
    keyed.push(0);
    for it in items {
        keyed.extend_from_slice(it.name.as_bytes());
        keyed.push(0);
        keyed.extend_from_slice(&it.content_ref);
        keyed.extend_from_slice(it.media_type.as_bytes());
        keyed.push(0);
    }
    let mut id = [0u8; 16];
    id.copy_from_slice(&kx_content::ContentRef::of(&keyed).0[..16]);
    id
}

fn items_to_json(items: &[BundleItemRecord]) -> String {
    let rows: Vec<ItemRow> = items
        .iter()
        .map(|it| ItemRow {
            name: it.name.clone(),
            ref_hex: hex_lower(&it.content_ref),
            media: it.media_type.clone(),
        })
        .collect();
    serde_json::to_string(&rows).unwrap_or_else(|_| "[]".to_string())
}

fn items_from_json(s: &str) -> Vec<BundleItemRecord> {
    let rows: Vec<ItemRow> = serde_json::from_str(s).unwrap_or_default();
    rows.into_iter()
        .filter_map(|r| {
            hex_to_32(&r.ref_hex).map(|content_ref| BundleItemRecord {
                name: r.name,
                content_ref,
                media_type: r.media,
            })
        })
        .collect()
}

fn hex_lower(bytes: &[u8; 32]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(64);
    for &b in bytes {
        s.push(DIGITS[(b >> 4) as usize] as char);
        s.push(DIGITS[(b & 0x0f) as usize] as char);
    }
    s
}

fn hex_to_32(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, chunk) in s.as_bytes().chunks(2).enumerate() {
        let hi = (chunk[0] as char).to_digit(16)?;
        let lo = (chunk[1] as char).to_digit(16)?;
        out[i] = u8::try_from(hi * 16 + lo).ok()?;
    }
    Some(out)
}

/// The durable context-bundle store over `bundles.db`. A single mutex'd connection:
/// bundle authoring is interactive-rate (a CLI/SDK put, an attach), never contended.
pub(crate) struct BundlesDb {
    conn: Mutex<Connection>,
}

impl BundlesDb {
    /// Open (or create) `bundles.db` under `dir`. A corrupt/foreign file or a
    /// `schema_version` drift recreates the ledger EMPTY (module doc).
    ///
    /// # Errors
    /// [`GatewayError::Catalog`] on an unrecoverable open/pragma failure.
    pub(crate) fn open(dir: &Path) -> Result<Self, GatewayError> {
        std::fs::create_dir_all(dir)
            .map_err(|e| GatewayError::Catalog(format!("bundles dir: {e}")))?;
        let db_path = dir.join("bundles.db");
        let conn = if let Ok(c) = Self::open_with_pragma(&db_path) {
            c
        } else {
            let _ = std::fs::remove_file(&db_path);
            let _ = std::fs::remove_file(dir.join("bundles.db-wal"));
            let _ = std::fs::remove_file(dir.join("bundles.db-shm"));
            Self::open_with_pragma(&db_path)
                .map_err(|e| GatewayError::Catalog(format!("bundles reopen: {e}")))?
        };
        let fresh_or_stale = match Self::read_schema_version(&conn) {
            Ok(Some(v)) => v != SCHEMA_VERSION,
            Ok(None) | Err(_) => true,
        };
        if fresh_or_stale {
            conn.execute_batch(
                "DROP TABLE IF EXISTS bundles;
                 DROP TABLE IF EXISTS meta;",
            )
            .map_err(|e| GatewayError::Catalog(format!("bundles rebuild: {e}")))?;
        }
        conn.execute_batch(SCHEMA)
            .map_err(|e| GatewayError::Catalog(format!("bundles schema: {e}")))?;
        conn.execute(
            "INSERT OR IGNORE INTO meta(key, value) VALUES ('schema_version', ?1)",
            params![SCHEMA_VERSION],
        )
        .map_err(|e| GatewayError::Catalog(format!("bundles meta init: {e}")))?;
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

    fn row_to_manifest(
        handle: String,
        bundle_ref: &[u8],
        description: String,
        items_json: &str,
    ) -> BundleManifest {
        let mut id = [0u8; 16];
        let n = bundle_ref.len().min(16);
        id[..n].copy_from_slice(&bundle_ref[..n]);
        BundleManifest {
            bundle_ref: id,
            handle,
            description,
            items: items_from_json(items_json),
        }
    }
}

impl BundleStore for BundlesDb {
    fn upsert(
        &self,
        principal: &str,
        handle: &str,
        description: &str,
        items: &[BundleItemRecord],
    ) -> Result<([u8; 16], bool), CoreError> {
        let bundle_ref = bundle_ref_of(handle, items);
        let conn = self
            .conn
            .lock()
            .map_err(|_| CoreError::Internal("bundles lock poisoned".into()))?;
        // Dedup signal: an identical manifest already bound to (principal, handle).
        let existing: Option<Vec<u8>> = conn
            .query_row(
                "SELECT bundle_ref FROM bundles WHERE principal = ?1 AND handle = ?2",
                params![principal, handle],
                |r| r.get(0),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })
            .map_err(|e| CoreError::Internal(format!("bundles dedup probe: {e}")))?;
        let deduplicated = existing.as_deref() == Some(&bundle_ref[..]);
        conn.execute(
            "INSERT OR REPLACE INTO bundles(principal, handle, bundle_ref, description, items_json) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                principal,
                handle,
                bundle_ref.to_vec(),
                description,
                items_to_json(items),
            ],
        )
        .map_err(|e| CoreError::Internal(format!("bundles upsert: {e}")))?;
        Ok((bundle_ref, deduplicated))
    }

    fn get(&self, principal: &str, handle: &str) -> Result<Option<BundleManifest>, CoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| CoreError::Internal("bundles lock poisoned".into()))?;
        conn.query_row(
            "SELECT handle, bundle_ref, description, items_json FROM bundles \
             WHERE principal = ?1 AND handle = ?2",
            params![principal, handle],
            |r| {
                let bundle_ref = r.get::<_, Vec<u8>>(1)?;
                let items_json = r.get::<_, String>(3)?;
                Ok(Self::row_to_manifest(
                    r.get::<_, String>(0)?,
                    &bundle_ref,
                    r.get::<_, String>(2)?,
                    &items_json,
                ))
            },
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })
        .map_err(|e| CoreError::Internal(format!("bundles get: {e}")))
    }

    fn list(
        &self,
        principal: &str,
        limit: usize,
        after_handle: Option<&str>,
    ) -> Result<(Vec<BundleManifest>, bool), CoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| CoreError::Internal("bundles lock poisoned".into()))?;
        let cursor = after_handle.unwrap_or("");
        // Fetch limit+1 to learn has_more without a second query.
        let mut stmt = conn
            .prepare(
                "SELECT handle, bundle_ref, description, items_json FROM bundles \
                 WHERE principal = ?1 AND handle > ?2 ORDER BY handle ASC LIMIT ?3",
            )
            .map_err(|e| CoreError::Internal(format!("bundles list prepare: {e}")))?;
        let fetch = i64::try_from(limit.saturating_add(1)).unwrap_or(i64::MAX);
        let rows = stmt
            .query_map(params![principal, cursor, fetch], |r| {
                let bundle_ref = r.get::<_, Vec<u8>>(1)?;
                let items_json = r.get::<_, String>(3)?;
                Ok(Self::row_to_manifest(
                    r.get::<_, String>(0)?,
                    &bundle_ref,
                    r.get::<_, String>(2)?,
                    &items_json,
                ))
            })
            .map_err(|e| CoreError::Internal(format!("bundles list query: {e}")))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| CoreError::Internal(format!("bundles list row: {e}")))?);
        }
        let has_more = out.len() > limit;
        out.truncate(limit);
        Ok((out, has_more))
    }

    fn delete(&self, principal: &str, handle: &str) -> Result<bool, CoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| CoreError::Internal("bundles lock poisoned".into()))?;
        let n = conn
            .execute(
                "DELETE FROM bundles WHERE principal = ?1 AND handle = ?2",
                params![principal, handle],
            )
            .map_err(|e| CoreError::Internal(format!("bundles delete: {e}")))?;
        Ok(n > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(tag: u8, name: &str) -> BundleItemRecord {
        BundleItemRecord {
            name: name.to_string(),
            content_ref: [tag; 32],
            media_type: "text/plain".into(),
        }
    }

    #[test]
    fn upsert_get_round_trips_and_dedups() {
        let dir = tempfile::tempdir().unwrap();
        let db = BundlesDb::open(dir.path()).unwrap();
        let items = vec![item(0x11, "a"), item(0x22, "b")];
        let (r1, dedup1) = db.upsert("alice", "ns/coll/x", "first", &items).unwrap();
        assert!(!dedup1, "first write is not a dedup");
        let got = db.get("alice", "ns/coll/x").unwrap().unwrap();
        assert_eq!(got.bundle_ref, r1);
        assert_eq!(got.items, items, "items round-trip in author order");
        // Re-put identical → deduplicated true, same ref.
        let (r2, dedup2) = db.upsert("alice", "ns/coll/x", "first", &items).unwrap();
        assert_eq!(r1, r2);
        assert!(dedup2, "identical re-put is a dedup");
    }

    #[test]
    fn bundle_ref_is_deterministic_and_content_addressed() {
        let items = vec![item(0x01, "a"), item(0x02, "b")];
        assert_eq!(
            bundle_ref_of("ns/coll/x", &items),
            bundle_ref_of("ns/coll/x", &items),
            "same (handle, items) ⇒ same ref"
        );
        assert_ne!(
            bundle_ref_of("ns/coll/x", &items),
            bundle_ref_of("ns/coll/y", &items),
            "handle is part of identity"
        );
        let reordered = vec![item(0x02, "b"), item(0x01, "a")];
        assert_ne!(
            bundle_ref_of("ns/coll/x", &items),
            bundle_ref_of("ns/coll/x", &reordered),
            "item order is part of the manifest identity"
        );
    }

    #[test]
    fn caller_scoped_isolation() {
        let dir = tempfile::tempdir().unwrap();
        let db = BundlesDb::open(dir.path()).unwrap();
        db.upsert("alice", "ns/coll/x", "", &[item(0x11, "a")])
            .unwrap();
        assert!(
            db.get("bob", "ns/coll/x").unwrap().is_none(),
            "bob cannot see alice's bundle"
        );
        assert!(db.get("alice", "ns/coll/x").unwrap().is_some());
    }

    #[test]
    fn list_paginates_in_handle_order() {
        let dir = tempfile::tempdir().unwrap();
        let db = BundlesDb::open(dir.path()).unwrap();
        for h in ["ns/c/a", "ns/c/b", "ns/c/c"] {
            db.upsert("alice", h, "", &[item(0x11, "x")]).unwrap();
        }
        let (page1, more1) = db.list("alice", 2, None).unwrap();
        assert_eq!(page1.len(), 2);
        assert!(more1);
        assert_eq!(page1[0].handle, "ns/c/a");
        let (page2, more2) = db.list("alice", 2, Some("ns/c/b")).unwrap();
        assert_eq!(page2.len(), 1);
        assert!(!more2);
        assert_eq!(page2[0].handle, "ns/c/c");
    }

    #[test]
    fn delete_unbinds() {
        let dir = tempfile::tempdir().unwrap();
        let db = BundlesDb::open(dir.path()).unwrap();
        db.upsert("alice", "ns/c/x", "", &[item(0x11, "x")])
            .unwrap();
        assert!(db.delete("alice", "ns/c/x").unwrap());
        assert!(db.get("alice", "ns/c/x").unwrap().is_none());
        assert!(
            !db.delete("alice", "ns/c/x").unwrap(),
            "second delete is a no-op"
        );
    }

    #[test]
    fn corrupt_file_recreates_empty() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("bundles.db"), b"not a sqlite file").unwrap();
        let db = BundlesDb::open(dir.path()).unwrap();
        assert!(
            db.get("alice", "ns/c/x").unwrap().is_none(),
            "corrupt sidecar recreates EMPTY"
        );
        db.upsert("alice", "ns/c/x", "", &[item(0x11, "x")])
            .unwrap();
        assert!(db.get("alice", "ns/c/x").unwrap().is_some());
    }
}
