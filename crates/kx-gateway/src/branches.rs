//! The D155 Phase-A branch sidecar: `branches.db` under `--catalog-dir`, backing
//! the [`BranchStore`] seam — `CreateBranch` / `SnapshotInto` manifests + the
//! caller-scoped branch read surface.
//!
//! ## Rebuildable-to-EMPTY (the `bundles.db` posture)
//! A branch manifest records which content-store blobs a snapshot grouped under a
//! `{path -> ContentRef}` handle; it is NOT derivable from the journal. Truth
//! (the blobs) lives in the content store, so on corruption or a schema-version
//! drift this ledger recreates EMPTY — the only loss is the manifest index, and
//! re-snapshotting the SAME files restores the SAME `branch_ref` (content-
//! addressed). Never journaled, never a `MoteId` input, never a digest input —
//! dropping the file cannot move the canonical projection digest (D160).
//!
//! ## Server-derived id (SN-8)
//! `branch_ref = blake3("kx-branch\0" ‖ handle ‖ parent ‖ items)[..16]` via
//! [`kx_content::ContentRef::of`]. The client names a `handle`; the server
//! derives the identity from the path-sorted resolved item set.
//!
//! ## Caller-scoped
//! The primary key is `(principal, handle)` — a branch is visible only to the
//! SERVER-RESOLVED party that authored it (uniform not-found for absent OR
//! not-owned; no cross-party existence oracle).
//!
//! ## Phase-A is READ-ONLY w.r.t. the host
//! `SnapshotInto` READS confined host files INTO the content store (gated by
//! `KX_SERVE_FS_ROOT`, default-OFF) and records `{path -> ref}`; it NEVER writes
//! the host. The path confinement reuses `fs-list`'s airtight canonicalize +
//! in-mount prefix-check ([`kx_capability::resolve_confined_file`]) — one shared
//! source of truth. Governed host write-back is Phase-B (after PR-8).

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use kx_capability::{resolve_confined_file, DEFAULT_MAX_READ_BYTES};
use kx_content::ContentStore;
use kx_gateway_core::GatewayError as CoreError;
use kx_gateway_core::{BranchItemRecord, BranchManifest, BranchStore};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::error::GatewayError;

/// Bump on any table-shape change. Unknown/missing version ⇒ recreate EMPTY
/// (branches are not journal-derivable, so there is no rebuild — re-snapshot).
const SCHEMA_VERSION: i64 = 1;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS branches (
    principal     TEXT NOT NULL,   -- server-resolved caller party (scope)
    handle        TEXT NOT NULL,   -- AssetPath 'ns/coll/name' (upsert key within principal)
    branch_ref    BLOB NOT NULL,   -- 16B server-derived manifest hash (display/dedup)
    parent_handle TEXT NOT NULL,   -- the CoW parent handle (lineage); '' = a root branch
    description   TEXT NOT NULL,    -- advisory, never parsed for enforcement
    items_json    TEXT NOT NULL,   -- JSON [{path, ref(hex)}], path-sorted
    PRIMARY KEY (principal, handle)
);
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value INTEGER NOT NULL);
";

/// On-disk item row (content_ref carried as 64-char hex in `items_json`).
#[derive(Serialize, Deserialize)]
struct ItemRow {
    path: String,
    #[serde(rename = "ref")]
    ref_hex: String,
}

/// `branch_ref = blake3("kx-branch\0" ‖ handle ‖ parent ‖ items)[..16]` (SN-8).
/// `items` MUST be path-sorted (the resolved manifest's canonical order) so the
/// id is content-addressed — identical resolved content ⇒ identical ref.
fn branch_ref_of(handle: &str, parent: &str, items: &[BranchItemRecord]) -> [u8; 16] {
    let mut keyed = Vec::with_capacity(64 + items.len() * 48);
    keyed.extend_from_slice(b"kx-branch\0");
    keyed.extend_from_slice(handle.as_bytes());
    keyed.push(0);
    keyed.extend_from_slice(parent.as_bytes());
    keyed.push(0);
    for it in items {
        keyed.extend_from_slice(it.path.as_bytes());
        keyed.push(0);
        keyed.extend_from_slice(&it.content_ref);
    }
    let mut id = [0u8; 16];
    id.copy_from_slice(&kx_content::ContentRef::of(&keyed).0[..16]);
    id
}

fn items_to_json(items: &[BranchItemRecord]) -> String {
    let rows: Vec<ItemRow> = items
        .iter()
        .map(|it| ItemRow {
            path: it.path.clone(),
            ref_hex: hex_lower(&it.content_ref),
        })
        .collect();
    serde_json::to_string(&rows).unwrap_or_else(|_| "[]".to_string())
}

fn items_from_json(s: &str) -> Vec<BranchItemRecord> {
    let rows: Vec<ItemRow> = serde_json::from_str(s).unwrap_or_default();
    rows.into_iter()
        .filter_map(|r| {
            hex_to_32(&r.ref_hex).map(|content_ref| BranchItemRecord {
                path: r.path,
                content_ref,
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

/// Sort a manifest's items by path (the canonical resolved order).
fn sort_items(items: &mut [BranchItemRecord]) {
    items.sort_by(|a, b| a.path.cmp(&b.path));
}

/// The durable branch store over `branches.db` plus the content store (the CAS
/// write target for `SnapshotInto`) and the optional operator FS root
/// (`KX_SERVE_FS_ROOT`; `None` ⇒ snapshot is default-OFF). A single mutex'd
/// connection: branch authoring is interactive-rate, never contended.
pub(crate) struct BranchesDb<S: ContentStore> {
    conn: Mutex<Connection>,
    content: std::sync::Arc<S>,
    /// The operator read root; `None` ⇒ `SnapshotInto` returns failed-precondition.
    fs_root: Option<PathBuf>,
    /// Per-file byte ceiling for a snapshot read (DoS guard; mirrors `fs-read@1`).
    max_bytes: u64,
}

impl<S: ContentStore> BranchesDb<S> {
    /// Open (or create) `branches.db` under `dir`, bound to the content store and
    /// the optional operator FS root. A corrupt/foreign file or a `schema_version`
    /// drift recreates the ledger EMPTY (module doc).
    ///
    /// # Errors
    /// [`GatewayError::Catalog`] on an unrecoverable open/pragma failure.
    pub(crate) fn open(
        dir: &Path,
        content: std::sync::Arc<S>,
        fs_root: Option<PathBuf>,
    ) -> Result<Self, GatewayError> {
        std::fs::create_dir_all(dir)
            .map_err(|e| GatewayError::Catalog(format!("branches dir: {e}")))?;
        let db_path = dir.join("branches.db");
        let conn = if let Ok(c) = Self::open_with_pragma(&db_path) {
            c
        } else {
            let _ = std::fs::remove_file(&db_path);
            let _ = std::fs::remove_file(dir.join("branches.db-wal"));
            let _ = std::fs::remove_file(dir.join("branches.db-shm"));
            Self::open_with_pragma(&db_path)
                .map_err(|e| GatewayError::Catalog(format!("branches reopen: {e}")))?
        };
        let fresh_or_stale = match Self::read_schema_version(&conn) {
            Ok(Some(v)) => v != SCHEMA_VERSION,
            Ok(None) | Err(_) => true,
        };
        if fresh_or_stale {
            conn.execute_batch(
                "DROP TABLE IF EXISTS branches;
                 DROP TABLE IF EXISTS meta;",
            )
            .map_err(|e| GatewayError::Catalog(format!("branches rebuild: {e}")))?;
        }
        conn.execute_batch(SCHEMA)
            .map_err(|e| GatewayError::Catalog(format!("branches schema: {e}")))?;
        conn.execute(
            "INSERT OR IGNORE INTO meta(key, value) VALUES ('schema_version', ?1)",
            params![SCHEMA_VERSION],
        )
        .map_err(|e| GatewayError::Catalog(format!("branches meta init: {e}")))?;
        Ok(Self {
            conn: Mutex::new(conn),
            content,
            fs_root,
            max_bytes: DEFAULT_MAX_READ_BYTES,
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
        branch_ref: &[u8],
        parent_handle: String,
        description: String,
        items_json: &str,
    ) -> BranchManifest {
        let mut id = [0u8; 16];
        let n = branch_ref.len().min(16);
        id[..n].copy_from_slice(&branch_ref[..n]);
        BranchManifest {
            branch_ref: id,
            handle,
            parent_handle,
            description,
            items: items_from_json(items_json),
        }
    }

    /// Load the stored `(parent_handle, items)` of `(principal, handle)`, if any.
    fn load_row(
        conn: &Connection,
        principal: &str,
        handle: &str,
    ) -> Result<Option<(String, Vec<BranchItemRecord>)>, CoreError> {
        conn.query_row(
            "SELECT parent_handle, items_json FROM branches WHERE principal = ?1 AND handle = ?2",
            params![principal, handle],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        )
        .map(|(parent, items_json)| Some((parent, items_from_json(&items_json))))
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(CoreError::Internal(format!("branches load: {other}"))),
        })
    }

    /// Upsert a resolved manifest (items are path-sorted here) and return it with
    /// the dedup signal.
    fn upsert_manifest(
        conn: &Connection,
        principal: &str,
        handle: &str,
        parent: &str,
        description: &str,
        mut items: Vec<BranchItemRecord>,
    ) -> Result<(BranchManifest, bool), CoreError> {
        sort_items(&mut items);
        let branch_ref = branch_ref_of(handle, parent, &items);
        let existing: Option<Vec<u8>> = conn
            .query_row(
                "SELECT branch_ref FROM branches WHERE principal = ?1 AND handle = ?2",
                params![principal, handle],
                |r| r.get(0),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })
            .map_err(|e| CoreError::Internal(format!("branches dedup probe: {e}")))?;
        let deduplicated = existing.as_deref() == Some(&branch_ref[..]);
        conn.execute(
            "INSERT OR REPLACE INTO branches(principal, handle, branch_ref, parent_handle, description, items_json) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                principal,
                handle,
                branch_ref.to_vec(),
                parent,
                description,
                items_to_json(&items),
            ],
        )
        .map_err(|e| CoreError::Internal(format!("branches upsert: {e}")))?;
        Ok((
            BranchManifest {
                branch_ref,
                handle: handle.to_string(),
                parent_handle: parent.to_string(),
                description: description.to_string(),
                items,
            },
            deduplicated,
        ))
    }
}

impl<S: ContentStore + Send + Sync + 'static> BranchStore for BranchesDb<S> {
    fn create(
        &self,
        principal: &str,
        handle: &str,
        parent_handle: Option<&str>,
        description: &str,
    ) -> Result<(BranchManifest, bool), CoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| CoreError::Internal("branches lock poisoned".into()))?;
        // A CoW fork inherits the parent's resolved items at create time (a
        // point-in-time snapshot; later parent edits do NOT propagate).
        let (parent, items) = match parent_handle {
            Some(p) => {
                let row = Self::load_row(&conn, principal, p)?
                    .ok_or(CoreError::NotFound("parent branch not found"))?;
                (p.to_string(), row.1)
            }
            None => (String::new(), Vec::new()),
        };
        Self::upsert_manifest(&conn, principal, handle, &parent, description, items)
    }

    fn snapshot_into(
        &self,
        principal: &str,
        handle: &str,
        parent_handle: Option<&str>,
        description: &str,
        paths: &[String],
    ) -> Result<(BranchManifest, usize, bool), CoreError> {
        // Host read is default-OFF — gated by the operator FS root.
        let root = self
            .fs_root
            .as_deref()
            .ok_or(CoreError::FailedPrecondition(
                "snapshot requires KX_SERVE_FS_ROOT (host snapshot is default-OFF)",
            ))?;
        let conn = self
            .conn
            .lock()
            .map_err(|_| CoreError::Internal("branches lock poisoned".into()))?;

        // Start from the existing branch's items (override the given paths), or a
        // fresh fork of the parent, or empty.
        let (parent, base_items) = match Self::load_row(&conn, principal, handle)? {
            Some((existing_parent, items)) => (existing_parent, items),
            None => match parent_handle {
                Some(p) => {
                    let row = Self::load_row(&conn, principal, p)?
                        .ok_or(CoreError::NotFound("parent branch not found"))?;
                    (p.to_string(), row.1)
                }
                None => (String::new(), Vec::new()),
            },
        };
        let mut by_path: std::collections::BTreeMap<String, [u8; 32]> = base_items
            .into_iter()
            .map(|it| (it.path, it.content_ref))
            .collect();

        let mut ingested = 0usize;
        for p in paths {
            // Confine + canonicalize + prefix-check (shared with fs-list; `..` /
            // symlink escapes refused). A uniform invalid-argument keeps no host
            // existence oracle.
            let target = resolve_confined_file(root, Some(p)).map_err(|_| {
                CoreError::InvalidArgument(
                    "a snapshot path escaped KX_SERVE_FS_ROOT or is not a regular file",
                )
            })?;
            // Byte cap BEFORE the read (no unbounded host read).
            let meta = std::fs::metadata(&target)
                .map_err(|e| CoreError::Internal(format!("snapshot metadata: {e}")))?;
            if meta.len() > self.max_bytes {
                return Err(CoreError::InvalidArgument(
                    "a snapshot file exceeds the per-file byte cap",
                ));
            }
            let bytes = std::fs::read(&target)
                .map_err(|e| CoreError::Internal(format!("snapshot read: {e}")))?;
            // Content-address into the SAME store the runtime commits to (dedup).
            let cref = self
                .content
                .put(&bytes)
                .map_err(|e| CoreError::Internal(format!("snapshot put: {e}")))?;
            by_path.insert(p.clone(), cref.0);
            ingested += 1;
        }

        let items: Vec<BranchItemRecord> = by_path
            .into_iter()
            .map(|(path, content_ref)| BranchItemRecord { path, content_ref })
            .collect();
        let (manifest, deduplicated) =
            Self::upsert_manifest(&conn, principal, handle, &parent, description, items)?;
        Ok((manifest, ingested, deduplicated))
    }

    fn get(&self, principal: &str, handle: &str) -> Result<Option<BranchManifest>, CoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| CoreError::Internal("branches lock poisoned".into()))?;
        conn.query_row(
            "SELECT handle, branch_ref, parent_handle, description, items_json FROM branches \
             WHERE principal = ?1 AND handle = ?2",
            params![principal, handle],
            |r| {
                let branch_ref = r.get::<_, Vec<u8>>(1)?;
                let items_json = r.get::<_, String>(4)?;
                Ok(Self::row_to_manifest(
                    r.get::<_, String>(0)?,
                    &branch_ref,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    &items_json,
                ))
            },
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(CoreError::Internal(format!("branches get: {other}"))),
        })
    }

    fn list(
        &self,
        principal: &str,
        limit: usize,
        after_handle: Option<&str>,
    ) -> Result<(Vec<BranchManifest>, bool), CoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| CoreError::Internal("branches lock poisoned".into()))?;
        let cursor = after_handle.unwrap_or("");
        let mut stmt = conn
            .prepare(
                "SELECT handle, branch_ref, parent_handle, description, items_json FROM branches \
                 WHERE principal = ?1 AND handle > ?2 ORDER BY handle ASC LIMIT ?3",
            )
            .map_err(|e| CoreError::Internal(format!("branches list prepare: {e}")))?;
        let fetch = i64::try_from(limit.saturating_add(1)).unwrap_or(i64::MAX);
        let rows = stmt
            .query_map(params![principal, cursor, fetch], |r| {
                let branch_ref = r.get::<_, Vec<u8>>(1)?;
                let items_json = r.get::<_, String>(4)?;
                Ok(Self::row_to_manifest(
                    r.get::<_, String>(0)?,
                    &branch_ref,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    &items_json,
                ))
            })
            .map_err(|e| CoreError::Internal(format!("branches list query: {e}")))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| CoreError::Internal(format!("branches list row: {e}")))?);
        }
        let has_more = out.len() > limit;
        out.truncate(limit);
        Ok((out, has_more))
    }

    fn delete(&self, principal: &str, handle: &str) -> Result<bool, CoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| CoreError::Internal("branches lock poisoned".into()))?;
        let n = conn
            .execute(
                "DELETE FROM branches WHERE principal = ?1 AND handle = ?2",
                params![principal, handle],
            )
            .map_err(|e| CoreError::Internal(format!("branches delete: {e}")))?;
        Ok(n > 0)
    }

    fn advance(
        &self,
        principal: &str,
        handle: &str,
        path: &str,
        content_ref: [u8; 32],
    ) -> Result<(BranchManifest, bool), CoreError> {
        // Strictly IN-CAS (the D155 Phase-3 edit step): the edited body is ALREADY
        // a committed `result_ref`. Fail-closed verify it resolves BEFORE touching
        // the manifest — a branch must never point at an unresolvable blob (the
        // F-7 / PR-7 `UpstreamMissing` posture). NO host read: `advance` never uses
        // `self.fs_root`, so it works even when `KX_SERVE_FS_ROOT` is unset.
        if !self.content.contains(&kx_content::ContentRef(content_ref)) {
            return Err(CoreError::InvalidArgument(
                "advance content_ref does not resolve in the content store",
            ));
        }
        let conn = self
            .conn
            .lock()
            .map_err(|_| CoreError::Internal("branches lock poisoned".into()))?;
        // Load the full row (preserve `parent_handle` + the advisory `description` —
        // re-pointing one path must not re-fork or blank the description).
        let (parent, description, base_items) = conn
            .query_row(
                "SELECT parent_handle, description, items_json FROM branches \
                 WHERE principal = ?1 AND handle = ?2",
                params![principal, handle],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        items_from_json(&r.get::<_, String>(2)?),
                    ))
                },
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => CoreError::NotFound("branch not found"),
                other => CoreError::Internal(format!("branches advance load: {other}")),
            })?;
        // Re-point `path` (or insert it — "enrich" per the B-spec). Re-pointing to
        // the CURRENT ref is a no-op that dedups (idempotent).
        let mut by_path: std::collections::BTreeMap<String, [u8; 32]> = base_items
            .into_iter()
            .map(|it| (it.path, it.content_ref))
            .collect();
        by_path.insert(path.to_string(), content_ref);
        let items: Vec<BranchItemRecord> = by_path
            .into_iter()
            .map(|(path, content_ref)| BranchItemRecord { path, content_ref })
            .collect();
        Self::upsert_manifest(&conn, principal, handle, &parent, &description, items)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kx_content::InMemoryContentStore;
    use std::sync::Arc;

    fn db_with_root(root: Option<PathBuf>) -> BranchesDb<InMemoryContentStore> {
        let dir = tempfile::tempdir().unwrap();
        // leak the tempdir guard for the test's lifetime via Box::leak-free: keep it.
        let content = Arc::new(InMemoryContentStore::default());
        let db = BranchesDb::open(dir.path(), content, root).unwrap();
        std::mem::forget(dir); // keep the sqlite file alive for the test
        db
    }

    #[test]
    fn snapshot_unset_root_fails_precondition() {
        let db = db_with_root(None);
        let err = db
            .snapshot_into("alice", "ns/coll/b", None, "", &["f.txt".to_string()])
            .unwrap_err();
        assert!(matches!(err, CoreError::FailedPrecondition(_)));
    }

    #[test]
    fn snapshot_reads_confined_files_into_cas_and_lists() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("a.txt"), b"alpha").unwrap();
        std::fs::write(root.path().join("b.txt"), b"beta").unwrap();
        let db = db_with_root(Some(root.path().to_path_buf()));

        let (m, ingested, dedup) = db
            .snapshot_into(
                "alice",
                "ns/coll/work",
                None,
                "my work",
                &["a.txt".to_string(), "b.txt".to_string()],
            )
            .unwrap();
        assert_eq!(ingested, 2);
        assert!(!dedup);
        // path-sorted manifest of {path -> ref}; the ref IS the file's ContentRef.
        assert_eq!(m.items.len(), 2);
        assert_eq!(m.items[0].path, "a.txt");
        assert_eq!(
            m.items[0].content_ref,
            kx_content::ContentRef::of(b"alpha").0
        );
        assert_eq!(
            m.items[1].content_ref,
            kx_content::ContentRef::of(b"beta").0
        );

        // visible only to the author; caller-scoped.
        assert!(db.get("alice", "ns/coll/work").unwrap().is_some());
        assert!(db.get("bob", "ns/coll/work").unwrap().is_none());

        // a re-snapshot of the SAME bytes dedups (same branch_ref).
        let (_, _, dedup2) = db
            .snapshot_into(
                "alice",
                "ns/coll/work",
                None,
                "my work",
                &["a.txt".to_string(), "b.txt".to_string()],
            )
            .unwrap();
        assert!(dedup2);
    }

    #[test]
    fn snapshot_refuses_escape_and_byte_cap() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("ok.txt"), b"ok").unwrap();
        let db = db_with_root(Some(root.path().to_path_buf()));
        let escape = db.snapshot_into(
            "alice",
            "ns/coll/x",
            None,
            "",
            &["../../etc/hosts".to_string()],
        );
        assert!(matches!(escape.unwrap_err(), CoreError::InvalidArgument(_)));
    }

    #[test]
    fn sub_branch_is_a_cow_fork_then_re_points_changed_paths() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("a.txt"), b"v1").unwrap();
        std::fs::write(root.path().join("b.txt"), b"keep").unwrap();
        let db = db_with_root(Some(root.path().to_path_buf()));

        // parent snapshot of two files.
        db.snapshot_into(
            "alice",
            "ns/coll/main",
            None,
            "",
            &["a.txt".to_string(), "b.txt".to_string()],
        )
        .unwrap();
        // fork: the sub-branch inherits BOTH paths at create.
        let (forked, _) = db
            .create("alice", "ns/coll/feature", Some("ns/coll/main"), "fork")
            .unwrap();
        assert_eq!(forked.items.len(), 2);
        assert_eq!(forked.parent_handle, "ns/coll/main");

        // change a.txt on disk, re-snapshot ONLY a.txt into the sub-branch.
        std::fs::write(root.path().join("a.txt"), b"v2").unwrap();
        let (re, ingested, _) = db
            .snapshot_into("alice", "ns/coll/feature", None, "", &["a.txt".to_string()])
            .unwrap();
        assert_eq!(ingested, 1);
        // a.txt re-points; b.txt keeps the parent's ref (zero-copy CoW).
        let a = re.items.iter().find(|i| i.path == "a.txt").unwrap();
        let b = re.items.iter().find(|i| i.path == "b.txt").unwrap();
        assert_eq!(a.content_ref, kx_content::ContentRef::of(b"v2").0);
        assert_eq!(b.content_ref, kx_content::ContentRef::of(b"keep").0);
        // the parent is unchanged (a branch is a point-in-time fork).
        let main = db.get("alice", "ns/coll/main").unwrap().unwrap();
        let main_a = main.items.iter().find(|i| i.path == "a.txt").unwrap();
        assert_eq!(main_a.content_ref, kx_content::ContentRef::of(b"v1").0);
    }

    #[test]
    fn list_paginates_and_delete_unbinds() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("f"), b"x").unwrap();
        let db = db_with_root(Some(root.path().to_path_buf()));
        for h in ["ns/c/a", "ns/c/b", "ns/c/c"] {
            db.snapshot_into("alice", h, None, "", &["f".to_string()])
                .unwrap();
        }
        let (page, has_more) = db.list("alice", 2, None).unwrap();
        assert_eq!(page.len(), 2);
        assert!(has_more);
        assert!(db.delete("alice", "ns/c/a").unwrap());
        assert!(!db.delete("alice", "ns/c/a").unwrap());
    }

    // ---- D155 Phase-3: in-CAS edit (`advance`) -----------------------------

    #[test]
    fn advance_re_points_a_path_and_keeps_others() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("a.txt"), b"v1").unwrap();
        std::fs::write(root.path().join("b.txt"), b"keep").unwrap();
        let db = db_with_root(Some(root.path().to_path_buf()));
        db.snapshot_into(
            "alice",
            "ns/coll/main",
            None,
            "desc",
            &["a.txt".to_string(), "b.txt".to_string()],
        )
        .unwrap();
        let before = db.get("alice", "ns/coll/main").unwrap().unwrap();

        // an agentic edit committed a NEW body to CAS — advance re-points a.txt.
        let edited = db.content.put(b"v2-edited").unwrap();
        let (m, dedup) = db
            .advance("alice", "ns/coll/main", "a.txt", edited.0)
            .unwrap();
        assert!(!dedup);
        let a = m.items.iter().find(|i| i.path == "a.txt").unwrap();
        let b = m.items.iter().find(|i| i.path == "b.txt").unwrap();
        assert_eq!(a.content_ref, edited.0);
        assert_eq!(b.content_ref, kx_content::ContentRef::of(b"keep").0);
        assert_ne!(m.branch_ref, before.branch_ref); // manifest advanced
        assert_eq!(m.description, "desc"); // advisory description preserved
        assert_eq!(m.parent_handle, ""); // not re-forked
    }

    #[test]
    fn advance_is_idempotent_and_dedups() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("a.txt"), b"v1").unwrap();
        let db = db_with_root(Some(root.path().to_path_buf()));
        db.snapshot_into("alice", "ns/coll/main", None, "", &["a.txt".to_string()])
            .unwrap();
        let edited = db.content.put(b"v2").unwrap();
        let (m1, dedup1) = db
            .advance("alice", "ns/coll/main", "a.txt", edited.0)
            .unwrap();
        assert!(!dedup1);
        // re-pointing to the SAME ref is a no-op that dedups (idempotent).
        let (m2, dedup2) = db
            .advance("alice", "ns/coll/main", "a.txt", edited.0)
            .unwrap();
        assert!(dedup2);
        assert_eq!(m1.branch_ref, m2.branch_ref);
    }

    #[test]
    fn advance_inserts_a_new_path_enrich_and_recomputes_ref() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("a.txt"), b"v1").unwrap();
        let db = db_with_root(Some(root.path().to_path_buf()));
        db.snapshot_into("alice", "ns/coll/main", None, "", &["a.txt".to_string()])
            .unwrap();
        let added = db.content.put(b"new-file").unwrap();
        let (m, _) = db
            .advance("alice", "ns/coll/main", "z.txt", added.0)
            .unwrap();
        assert_eq!(m.items.len(), 2);
        assert_eq!(m.items[0].path, "a.txt"); // items stay path-sorted
        assert_eq!(m.items[1].path, "z.txt");
        assert_eq!(m.items[1].content_ref, added.0);
        // branch_ref matches a fresh recompute over the advanced, sorted items.
        assert_eq!(m.branch_ref, branch_ref_of("ns/coll/main", "", &m.items));
    }

    #[test]
    fn advance_unknown_handle_or_principal_not_found() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("a.txt"), b"v1").unwrap();
        let db = db_with_root(Some(root.path().to_path_buf()));
        db.snapshot_into("alice", "ns/coll/main", None, "", &["a.txt".to_string()])
            .unwrap();
        let r = db.content.put(b"x").unwrap();
        assert!(matches!(
            db.advance("alice", "ns/coll/missing", "a.txt", r.0)
                .unwrap_err(),
            CoreError::NotFound(_)
        ));
        // caller-scoped: no cross-party advance.
        assert!(matches!(
            db.advance("bob", "ns/coll/main", "a.txt", r.0).unwrap_err(),
            CoreError::NotFound(_)
        ));
    }

    #[test]
    fn advance_unresolvable_ref_invalid_argument() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("a.txt"), b"v1").unwrap();
        let db = db_with_root(Some(root.path().to_path_buf()));
        db.snapshot_into("alice", "ns/coll/main", None, "", &["a.txt".to_string()])
            .unwrap();
        // a ref never put into the store — fail-closed (no dangling manifest).
        let bogus = [0xABu8; 32];
        assert!(matches!(
            db.advance("alice", "ns/coll/main", "a.txt", bogus)
                .unwrap_err(),
            CoreError::InvalidArgument(_)
        ));
    }

    #[test]
    fn advance_is_host_free_works_without_fs_root() {
        // `advance` never reads the host, so it works when KX_SERVE_FS_ROOT is
        // unset (where `snapshot_into` would FAILED_PRECONDITION).
        let db = db_with_root(None);
        db.create("alice", "ns/coll/empty", None, "").unwrap();
        let body = db.content.put(b"generated").unwrap();
        let (m, _) = db
            .advance("alice", "ns/coll/empty", "out.txt", body.0)
            .unwrap();
        assert_eq!(m.items.len(), 1);
        assert_eq!(m.items[0].path, "out.txt");
        assert_eq!(m.items[0].content_ref, body.0);
    }
}
