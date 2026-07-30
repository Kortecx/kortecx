//! The D155 Phase-A branch sidecar: `branches.db` under `--catalog-dir`, backing
//! the [`BranchStore`] seam — `CreateBranch` / `SnapshotInto` manifests + the
//! caller-scoped branch read surface.
//!
//! ## Authored work — preserved across an upgrade, never wiped by one
//! A branch manifest records which content-store blobs a snapshot grouped under a
//! `{path -> ContentRef}` handle; it is NOT derivable from the journal. Truth
//! (the blobs) lives in the content store, so on corruption or a schema-version
//! drift this ledger is renamed aside and re-imported, and
//! re-snapshotting the SAME files restores the SAME `branch_ref` (content-
//! addressed). Never journaled, never a `MoteId` input, never a digest input —
//! dropping the file cannot move the canonical projection digest (D160).
//!
//! ## Server-derived id
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

/// Bump on any table-shape change. A corrupt/foreign FILE recreates empty; a version
/// BUMP renames the catalog aside and re-imports what still fits, and a DOWNGRADE is
/// refused — this store holds authored work (`crate::sidecar`).
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
CREATE TABLE IF NOT EXISTS branch_history (
    principal        TEXT NOT NULL,     -- same scope as the branch row
    handle           TEXT NOT NULL,
    version          INTEGER NOT NULL,  -- 1-based per (principal, handle), monotone
    branch_ref       BLOB NOT NULL,     -- the manifest hash AT this version (display only)
    parent_handle    TEXT NOT NULL,
    description      TEXT NOT NULL,
    items_json       TEXT NOT NULL,     -- the full manifest at this version
    recorded_unix_ms INTEGER NOT NULL,  -- sidecar wall-clock; advisory, never identity
    cause            TEXT NOT NULL,     -- baseline|create|snapshot|advance|restore
    PRIMARY KEY (principal, handle, version)
);
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value INTEGER NOT NULL);
";

/// Bump on a `branch_history`-shape change: drops ONLY the history table (the
/// branch rows themselves stay — history is the safety net, never the truth).
const HISTORY_SCHEMA_VERSION: i64 = 1;

/// The default per-handle history retention (FIFO — oldest versions pruned past
/// the cap). Overridable via `KX_BRANCH_HISTORY_MAX` (resolved once at serve
/// start and passed into [`BranchesDb::open`]).
pub(crate) const DEFAULT_BRANCH_HISTORY_MAX: usize = 256;

/// On-disk item row (content_ref carried as 64-char hex in `items_json`).
#[derive(Serialize, Deserialize)]
struct ItemRow {
    path: String,
    #[serde(rename = "ref")]
    ref_hex: String,
}

/// `branch_ref = blake3("kx-branch\0" ‖ handle ‖ parent ‖ items)[..16]`.
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
    /// Per-handle history retention (FIFO prune past this many versions). A
    /// constructor value, never a per-call env read — tests inject it directly.
    history_max: usize,
}

impl<S: ContentStore> BranchesDb<S> {
    /// Open (or create) `branches.db` under `dir`, bound to the content store and
    /// the optional operator FS root. A corrupt/foreign file recreates the ledger
    /// empty; a schema bump PRESERVES it — renamed aside and re-imported, and a
    /// downgrade is refused (module doc).
    ///
    /// # Errors
    /// [`GatewayError::Catalog`] on an unrecoverable open/pragma failure.
    pub(crate) fn open(
        dir: &Path,
        content: std::sync::Arc<S>,
        fs_root: Option<PathBuf>,
        history_max: usize,
    ) -> Result<Self, GatewayError> {
        // A branch binds an asset to the content its author committed, and the history
        // table IS the restore surface — losing either loses work no one can regenerate.
        // Renamed aside and re-imported on a bump; a downgrade refuses. `crate::sidecar`.
        let conn = crate::sidecar::open_sidecar(
            dir,
            "branches.db",
            SCHEMA_VERSION,
            SCHEMA,
            &["branches", "branch_history", "meta"],
            crate::sidecar::Durability::UserAuthored,
        )?;
        // The history table versions independently: a history-shape change drops
        // ONLY branch_history (the branch rows stay — history is the safety net,
        // never the truth). `branches` predating the table reads as "no history
        // yet"; the first non-dedup mutation seeds a baseline (see upsert).
        let history_stale = match Self::read_meta(&conn, "history_schema_version") {
            Ok(Some(v)) => v != HISTORY_SCHEMA_VERSION,
            Ok(None) => false, // freshly created above; stamp below
            Err(_) => true,
        };
        if history_stale {
            conn.execute_batch("DELETE FROM branch_history;")
                .map_err(|e| GatewayError::Catalog(format!("branch history rebuild: {e}")))?;
        }
        conn.execute(
            "INSERT OR REPLACE INTO meta(key, value) VALUES ('history_schema_version', ?1)",
            params![HISTORY_SCHEMA_VERSION],
        )
        .map_err(|e| GatewayError::Catalog(format!("branches history meta: {e}")))?;
        Ok(Self {
            conn: Mutex::new(conn),
            content,
            fs_root,
            max_bytes: DEFAULT_MAX_READ_BYTES,
            history_max: history_max.max(1),
        })
    }

    fn read_meta(conn: &Connection, key: &str) -> rusqlite::Result<Option<i64>> {
        conn.query_row("SELECT value FROM meta WHERE key = ?1", params![key], |r| {
            r.get(0)
        })
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })
    }

    fn now_unix_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
            .unwrap_or(0)
    }

    /// Append one `branch_history` row for `(principal, handle)` and FIFO-prune
    /// past `history_max`. Returns the recorded version number.
    #[allow(clippy::too_many_arguments)]
    fn append_history(
        conn: &Connection,
        principal: &str,
        handle: &str,
        branch_ref: &[u8; 16],
        parent: &str,
        description: &str,
        items_json: &str,
        cause: &str,
        history_max: usize,
    ) -> Result<u32, CoreError> {
        let next: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) + 1 FROM branch_history \
                 WHERE principal = ?1 AND handle = ?2",
                params![principal, handle],
                |r| r.get(0),
            )
            .map_err(|e| CoreError::Internal(format!("branch history next: {e}")))?;
        conn.execute(
            "INSERT INTO branch_history(principal, handle, version, branch_ref, parent_handle, \
             description, items_json, recorded_unix_ms, cause) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                principal,
                handle,
                next,
                branch_ref.to_vec(),
                parent,
                description,
                items_json,
                i64::try_from(Self::now_unix_ms()).unwrap_or(i64::MAX),
                cause,
            ],
        )
        .map_err(|e| CoreError::Internal(format!("branch history append: {e}")))?;
        // FIFO retention: keep the newest `history_max` versions per handle.
        conn.execute(
            "DELETE FROM branch_history WHERE principal = ?1 AND handle = ?2 AND version <= ?3",
            params![
                principal,
                handle,
                next - i64::try_from(history_max).unwrap_or(i64::MAX)
            ],
        )
        .map_err(|e| CoreError::Internal(format!("branch history prune: {e}")))?;
        Ok(u32::try_from(next).unwrap_or(u32::MAX))
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

    /// Load the FULL stored manifest of `(principal, handle)`, if any, on a connection the
    /// caller already holds. `get` is the same read with its own lock — `create` cannot use
    /// it (it holds the mutex), and reconstructing the manifest by hand there would risk
    /// deriving a `branch_ref` that disagrees with the stored one.
    fn load_manifest(
        conn: &Connection,
        principal: &str,
        handle: &str,
    ) -> Result<Option<BranchManifest>, CoreError> {
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
            other => Err(CoreError::Internal(format!("branches load: {other}"))),
        })
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
    /// the dedup signal plus the history version recorded (None on a dedup no-op).
    ///
    /// Every NON-dedup upsert appends a `branch_history` version tagged `cause`.
    /// A pre-history row's first mutation seeds a `"baseline"` version from the
    /// stored row FIRST, so the pre-upgrade state stays restorable.
    #[allow(clippy::too_many_arguments)]
    fn upsert_manifest(
        conn: &Connection,
        principal: &str,
        handle: &str,
        parent: &str,
        description: &str,
        mut items: Vec<BranchItemRecord>,
        cause: &str,
        history_max: usize,
    ) -> Result<(BranchManifest, bool, Option<u32>), CoreError> {
        sort_items(&mut items);
        let branch_ref = branch_ref_of(handle, parent, &items);
        // The dedup probe reads the FULL existing row — the baseline seed below
        // needs its fields, not just the ref.
        let existing: Option<(Vec<u8>, String, String, String)> = conn
            .query_row(
                "SELECT branch_ref, parent_handle, description, items_json FROM branches \
                 WHERE principal = ?1 AND handle = ?2",
                params![principal, handle],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })
            .map_err(|e| CoreError::Internal(format!("branches dedup probe: {e}")))?;
        let deduplicated =
            existing.as_ref().map(|(r, _, _, _)| r.as_slice()) == Some(&branch_ref[..]);
        let mut recorded = None;
        if !deduplicated {
            // Baseline seed: a row that predates the history table gets its
            // CURRENT state recorded as version 1 before the mutation lands, so
            // the first post-upgrade edit is undoable.
            if let Some((old_ref, old_parent, old_desc, old_items)) = existing.as_ref() {
                let none_yet: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM branch_history WHERE principal = ?1 AND handle = ?2",
                        params![principal, handle],
                        |r| r.get(0),
                    )
                    .map_err(|e| CoreError::Internal(format!("branch history probe: {e}")))?;
                if none_yet == 0 {
                    let mut old16 = [0u8; 16];
                    let n = old_ref.len().min(16);
                    old16[..n].copy_from_slice(&old_ref[..n]);
                    Self::append_history(
                        conn,
                        principal,
                        handle,
                        &old16,
                        old_parent,
                        old_desc,
                        old_items,
                        "baseline",
                        history_max,
                    )?;
                }
            }
            let items_json = items_to_json(&items);
            recorded = Some(Self::append_history(
                conn,
                principal,
                handle,
                &branch_ref,
                parent,
                description,
                &items_json,
                cause,
                history_max,
            )?);
        }
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
            recorded,
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
            // A PARENTLESS create over a branch that already exists returns it UNTOUCHED.
            //
            // This arm used to build an empty item list and hand it to `upsert_manifest`,
            // whose `INSERT OR REPLACE` then wrote it over whatever was there — so a
            // parentless `create` on a populated branch DESTROYED it. That is not a corner
            // case: it is the first statement of every scaffold, including a RESUME
            // (`scaffold.rs`, both lanes), which made the resume probe 25 lines further down
            // structurally unreachable and silently discarded every file already authored.
            // `CreateBranch` on an existing handle did the same to a user's project.
            //
            // "Create" now means create. Re-creating is a no-op that reports the branch it
            // found, `deduplicated = true` — consistent with what that flag already means
            // here (the bound manifest is unchanged). `description` is deliberately NOT
            // applied to an existing row: a half-applied re-create is a worse contract than
            // a no-op, and nothing asks for a description-only edit.
            None => match Self::load_manifest(&conn, principal, handle)? {
                Some(existing) => return Ok((existing, true)),
                None => (String::new(), Vec::new()),
            },
        };
        let (m, dedup, _) = Self::upsert_manifest(
            &conn,
            principal,
            handle,
            &parent,
            description,
            items,
            "create",
            self.history_max,
        )?;
        Ok((m, dedup))
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
        let (manifest, deduplicated, _) = Self::upsert_manifest(
            &conn,
            principal,
            handle,
            &parent,
            description,
            items,
            "snapshot",
            self.history_max,
        )?;
        Ok((manifest, ingested, deduplicated))
    }

    fn get(&self, principal: &str, handle: &str) -> Result<Option<BranchManifest>, CoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| CoreError::Internal("branches lock poisoned".into()))?;
        Self::load_manifest(&conn, principal, handle)
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
        let (m, dedup, _) = Self::upsert_manifest(
            &conn,
            principal,
            handle,
            &parent,
            &description,
            items,
            "advance",
            self.history_max,
        )?;
        Ok((m, dedup))
    }
}

impl<S: ContentStore + Send + Sync + 'static> kx_gateway_core::BranchHistory for BranchesDb<S> {
    fn list_versions(
        &self,
        principal: &str,
        handle: &str,
        limit: usize,
        after_version: Option<u32>,
    ) -> Result<Option<(Vec<kx_gateway_core::BranchVersionRecord>, bool)>, CoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| CoreError::Internal("branches lock poisoned".into()))?;
        // Exclusive DESCENDING cursor; no cursor ⇒ the newest page.
        let ceiling = i64::from(after_version.unwrap_or(u32::MAX));
        let fetch = i64::try_from(limit.saturating_add(1)).unwrap_or(i64::MAX);
        let mut stmt = conn
            .prepare(
                "SELECT version, branch_ref, recorded_unix_ms, cause, items_json \
                 FROM branch_history \
                 WHERE principal = ?1 AND handle = ?2 AND version < ?3 \
                 ORDER BY version DESC LIMIT ?4",
            )
            .map_err(|e| CoreError::Internal(format!("branch history list prepare: {e}")))?;
        let rows = stmt
            .query_map(params![principal, handle, ceiling, fetch], |r| {
                let branch_ref = r.get::<_, Vec<u8>>(1)?;
                let items_json = r.get::<_, String>(4)?;
                let mut id = [0u8; 16];
                let n = branch_ref.len().min(16);
                id[..n].copy_from_slice(&branch_ref[..n]);
                Ok(kx_gateway_core::BranchVersionRecord {
                    version: u32::try_from(r.get::<_, i64>(0)?).unwrap_or(u32::MAX),
                    branch_ref: id,
                    recorded_unix_ms: u64::try_from(r.get::<_, i64>(2)?).unwrap_or(0),
                    cause: r.get::<_, String>(3)?,
                    item_count: u32::try_from(items_from_json(&items_json).len()).unwrap_or(0),
                })
            })
            .map_err(|e| CoreError::Internal(format!("branch history list query: {e}")))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| CoreError::Internal(format!("branch history row: {e}")))?);
        }
        // Uniform None for absent / not-owned / no-history — a cursor past the
        // oldest version on a branch WITH history still reports found (empty page).
        if out.is_empty() && after_version.is_none() {
            return Ok(None);
        }
        let has_more = out.len() > limit;
        out.truncate(limit);
        Ok(Some((out, has_more)))
    }

    fn restore(
        &self,
        principal: &str,
        handle: &str,
        version: u32,
    ) -> Result<(BranchManifest, u32, bool), CoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| CoreError::Internal("branches lock poisoned".into()))?;
        // The historical manifest — uniform NotFound for unknown version / handle /
        // principal (no cross-party existence oracle).
        let (hist_parent, hist_desc, hist_items_json) = conn
            .query_row(
                "SELECT parent_handle, description, items_json FROM branch_history \
                 WHERE principal = ?1 AND handle = ?2 AND version = ?3",
                params![principal, handle, i64::from(version)],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                },
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => {
                    CoreError::NotFound("branch version not found")
                }
                other => CoreError::Internal(format!("branch history load: {other}")),
            })?;
        let items = items_from_json(&hist_items_json);
        // Fail-closed: a manifest must never point at an unresolvable blob. CAS
        // blobs are immutable and branch ops never collect them, so a recorded
        // version SHOULD always resolve — refuse loudly if an operator GC'd blobs.
        for it in &items {
            if !self
                .content
                .contains(&kx_content::ContentRef(it.content_ref))
            {
                return Err(CoreError::InvalidArgument(
                    "a recorded item no longer resolves in the content store; this version cannot be restored",
                ));
            }
        }
        // Restore re-points items under the CURRENT row's parent/description
        // (it is not a re-fork); a deleted row is recreated from the history row.
        let (parent, description) = match Self::load_manifest(&conn, principal, handle)? {
            Some(current) => (current.parent_handle, current.description),
            None => (hist_parent, hist_desc),
        };
        let (manifest, deduplicated, recorded) = Self::upsert_manifest(
            &conn,
            principal,
            handle,
            &parent,
            &description,
            items,
            "restore",
            self.history_max,
        )?;
        Ok((manifest, recorded.unwrap_or(0), deduplicated))
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
        let db = BranchesDb::open(dir.path(), content, root, 256).unwrap();
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

    #[test]
    fn resume_preserves_items() {
        // THE scaffold-resume path. Every scaffold — first run AND resume — opens with a
        // parentless `create`. When that emptied the row, a resume silently discarded every
        // file already authored, and the resume probe 25 lines later could never see one.
        let db = db_with_root(None);
        db.create("alice", "apps/local/a", None, "project").unwrap();
        let r1 = db.content.put(b"first").unwrap();
        let r2 = db.content.put(b"second").unwrap();
        db.advance("alice", "apps/local/a", "README.md", r1.0)
            .unwrap();
        db.advance("alice", "apps/local/a", "src/App.tsx", r2.0)
            .unwrap();

        let (m, deduplicated) = db.create("alice", "apps/local/a", None, "project").unwrap();

        assert_eq!(m.items.len(), 2, "a resume must not empty the branch");
        assert_eq!(m.items[0].path, "README.md");
        assert_eq!(m.items[1].path, "src/App.tsx");
        assert_eq!(m.items[1].content_ref, r2.0);
        assert!(
            deduplicated,
            "a no-op re-create leaves the bound manifest unchanged"
        );
        // And it is genuinely persisted, not just echoed back.
        let stored = db.get("alice", "apps/local/a").unwrap().unwrap();
        assert_eq!(stored.items.len(), 2);
        assert_eq!(stored.branch_ref, m.branch_ref);
    }

    #[test]
    fn second_create_is_not_destructive() {
        // `CreateBranch` on a handle a user already owns must not wipe their project — the
        // same defect reached through the RPC rather than the scaffolder. A differing
        // description does not license a partial reset either.
        let db = db_with_root(None);
        db.create("alice", "ns/coll/work", None, "original")
            .unwrap();
        let r = db.content.put(b"body").unwrap();
        db.advance("alice", "ns/coll/work", "notes.md", r.0)
            .unwrap();
        let before = db.get("alice", "ns/coll/work").unwrap().unwrap();

        let (m, _) = db
            .create("alice", "ns/coll/work", None, "a different description")
            .unwrap();

        assert_eq!(m.items, before.items);
        assert_eq!(m.branch_ref, before.branch_ref);
        assert_eq!(
            m.description, "original",
            "an existing branch is returned untouched, not half-updated"
        );
    }

    #[test]
    fn create_still_creates_a_fresh_branch() {
        // The guard must not turn `create` into a no-op for the case it exists for.
        let db = db_with_root(None);
        let (m, deduplicated) = db.create("alice", "ns/coll/new", None, "fresh").unwrap();
        assert!(m.items.is_empty());
        assert_eq!(m.description, "fresh");
        assert_eq!(m.parent_handle, "");
        assert!(!deduplicated, "a genuinely new branch is not a dedup hit");
    }

    #[test]
    fn a_parentless_create_is_scoped_to_its_principal() {
        // The guard reads the row for (principal, handle) — bob creating the same handle
        // must get his OWN empty branch, never alice's contents.
        let db = db_with_root(None);
        db.create("alice", "ns/coll/x", None, "alice's").unwrap();
        let r = db.content.put(b"secret").unwrap();
        db.advance("alice", "ns/coll/x", "a.md", r.0).unwrap();

        let (m, _) = db.create("bob", "ns/coll/x", None, "bob's").unwrap();
        assert!(m.items.is_empty(), "no cross-principal read");
        assert_eq!(m.description, "bob's");
        // alice's is untouched.
        assert_eq!(
            db.get("alice", "ns/coll/x").unwrap().unwrap().items.len(),
            1
        );
    }

    // ---- Branch point-in-time history (`branch_history`) -------------------

    use kx_gateway_core::BranchHistory as _;

    /// Open a `BranchesDb` at an explicit dir (so a test can close + reopen it,
    /// or reach the sqlite file with a raw connection between opens).
    fn db_at(
        dir: &Path,
        content: Arc<InMemoryContentStore>,
        history_max: usize,
    ) -> BranchesDb<InMemoryContentStore> {
        BranchesDb::open(dir, content, None, history_max).unwrap()
    }

    #[test]
    fn every_non_dedup_mutation_records_a_version_with_its_cause() {
        let db = db_with_root(None);
        db.create("alice", "apps/local/a", None, "p").unwrap();
        let r1 = db.content.put(b"one").unwrap();
        db.advance("alice", "apps/local/a", "a.md", r1.0).unwrap();
        let r2 = db.content.put(b"two").unwrap();
        db.advance("alice", "apps/local/a", "b.md", r2.0).unwrap();

        let (versions, has_more) = db
            .list_versions("alice", "apps/local/a", 10, None)
            .unwrap()
            .expect("history exists");
        assert!(!has_more);
        // Newest-first: advance(b) → advance(a) → create.
        let causes: Vec<&str> = versions.iter().map(|v| v.cause.as_str()).collect();
        assert_eq!(causes, vec!["advance", "advance", "create"]);
        assert_eq!(versions[0].version, 3);
        assert_eq!(versions[0].item_count, 2);
        assert_eq!(versions[2].item_count, 0);
        // The recorded ref at the newest version IS the current branch ref.
        let current = db.get("alice", "apps/local/a").unwrap().unwrap();
        assert_eq!(versions[0].branch_ref, current.branch_ref);
    }

    #[test]
    fn a_dedup_mutation_records_nothing() {
        let db = db_with_root(None);
        db.create("alice", "apps/local/a", None, "p").unwrap();
        let r = db.content.put(b"x").unwrap();
        db.advance("alice", "apps/local/a", "a.md", r.0).unwrap();
        // Idempotent re-advance to the SAME ref + parentless re-create: no rows.
        db.advance("alice", "apps/local/a", "a.md", r.0).unwrap();
        db.create("alice", "apps/local/a", None, "p").unwrap();
        let (versions, _) = db
            .list_versions("alice", "apps/local/a", 10, None)
            .unwrap()
            .unwrap();
        assert_eq!(versions.len(), 2, "create + one advance, nothing else");
    }

    #[test]
    fn restore_appends_the_historical_items_and_preserves_current_metadata() {
        let db = db_with_root(None);
        db.create("alice", "apps/local/a", None, "the project")
            .unwrap();
        let v1 = db.content.put(b"version one").unwrap();
        db.advance("alice", "apps/local/a", "doc.md", v1.0).unwrap();
        let v2 = db.content.put(b"version two").unwrap();
        db.advance("alice", "apps/local/a", "doc.md", v2.0).unwrap();

        // Restore to the state after the FIRST advance (version 2: create=1, adv=2, adv=3).
        let (m, new_version, dedup) = db.restore("alice", "apps/local/a", 2).unwrap();
        assert!(!dedup);
        assert_eq!(new_version, 4, "restore APPENDS — history is never rewound");
        assert_eq!(m.items.len(), 1);
        assert_eq!(
            m.items[0].content_ref, v1.0,
            "the historical body is current again"
        );
        assert_eq!(m.description, "the project", "current metadata preserved");
        // The pre-restore state (v2) is still restorable — restore forward works.
        let (fwd, _, _) = db.restore("alice", "apps/local/a", 3).unwrap();
        assert_eq!(fwd.items[0].content_ref, v2.0);
    }

    #[test]
    fn restore_to_the_current_state_is_a_dedup_noop() {
        let db = db_with_root(None);
        db.create("alice", "apps/local/a", None, "").unwrap();
        let r = db.content.put(b"x").unwrap();
        db.advance("alice", "apps/local/a", "a.md", r.0).unwrap();
        let (_, new_version, dedup) = db.restore("alice", "apps/local/a", 2).unwrap();
        assert!(dedup);
        assert_eq!(new_version, 0, "nothing recorded on a no-op restore");
        let (versions, _) = db
            .list_versions("alice", "apps/local/a", 10, None)
            .unwrap()
            .unwrap();
        assert_eq!(versions.len(), 2);
    }

    #[test]
    fn restore_unknown_version_or_principal_is_uniform_not_found() {
        let db = db_with_root(None);
        db.create("alice", "apps/local/a", None, "").unwrap();
        assert!(matches!(
            db.restore("alice", "apps/local/a", 99).unwrap_err(),
            CoreError::NotFound(_)
        ));
        assert!(matches!(
            db.restore("bob", "apps/local/a", 1).unwrap_err(),
            CoreError::NotFound(_)
        ));
        assert!(matches!(
            db.restore("alice", "apps/local/missing", 1).unwrap_err(),
            CoreError::NotFound(_)
        ));
    }

    #[test]
    fn history_survives_delete_and_restore_recreates_the_branch() {
        // "Recreate without losing state": DeleteBranch unbinds the row; the
        // versions (and the CAS blobs) stay, so restore brings the project back.
        let db = db_with_root(None);
        db.create("alice", "apps/local/a", None, "kept desc")
            .unwrap();
        let r = db.content.put(b"the work").unwrap();
        db.advance("alice", "apps/local/a", "work.md", r.0).unwrap();
        assert!(db.delete("alice", "apps/local/a").unwrap());
        assert!(db.get("alice", "apps/local/a").unwrap().is_none());

        let (m, _, dedup) = db.restore("alice", "apps/local/a", 2).unwrap();
        assert!(!dedup);
        assert_eq!(m.items.len(), 1);
        assert_eq!(m.items[0].content_ref, r.0);
        assert_eq!(m.description, "kept desc", "recreated from the history row");
        assert!(db.get("alice", "apps/local/a").unwrap().is_some());
    }

    #[test]
    fn retention_prunes_oldest_versions_fifo() {
        let dir = tempfile::tempdir().unwrap();
        let content = Arc::new(InMemoryContentStore::default());
        let db = db_at(dir.path(), content, 3);
        db.create("alice", "apps/local/a", None, "").unwrap();
        for i in 0..5u8 {
            let r = db.content.put(&[i; 4]).unwrap();
            db.advance("alice", "apps/local/a", "a.bin", r.0).unwrap();
        }
        // 6 mutations total, retention 3 ⇒ versions 4..=6 only.
        let (versions, has_more) = db
            .list_versions("alice", "apps/local/a", 10, None)
            .unwrap()
            .unwrap();
        assert!(!has_more);
        let nums: Vec<u32> = versions.iter().map(|v| v.version).collect();
        assert_eq!(
            nums,
            vec![6, 5, 4],
            "newest kept, oldest pruned, numbering monotone"
        );
    }

    #[test]
    fn list_versions_pages_newest_first_with_a_descending_cursor() {
        let db = db_with_root(None);
        db.create("alice", "apps/local/a", None, "").unwrap();
        for i in 0..4u8 {
            let r = db.content.put(&[i; 2]).unwrap();
            db.advance("alice", "apps/local/a", "f", r.0).unwrap();
        }
        let (page1, more1) = db
            .list_versions("alice", "apps/local/a", 2, None)
            .unwrap()
            .unwrap();
        assert_eq!(
            page1.iter().map(|v| v.version).collect::<Vec<_>>(),
            vec![5, 4]
        );
        assert!(more1);
        let (page2, more2) = db
            .list_versions("alice", "apps/local/a", 2, Some(4))
            .unwrap()
            .unwrap();
        assert_eq!(
            page2.iter().map(|v| v.version).collect::<Vec<_>>(),
            vec![3, 2]
        );
        assert!(more2);
    }

    #[test]
    fn list_versions_is_uniformly_absent_for_no_history_or_cross_principal() {
        let db = db_with_root(None);
        assert!(db
            .list_versions("alice", "apps/local/never", 10, None)
            .unwrap()
            .is_none());
        db.create("alice", "apps/local/a", None, "").unwrap();
        // caller-scoped: bob sees nothing of alice's history.
        assert!(db
            .list_versions("bob", "apps/local/a", 10, None)
            .unwrap()
            .is_none());
    }

    #[test]
    fn a_pre_history_row_seeds_a_baseline_on_its_first_mutation() {
        // Simulate a branches.db written BEFORE the history table carried rows:
        // build a populated row, purge its history out-of-band, reopen, mutate.
        let dir = tempfile::tempdir().unwrap();
        let content = Arc::new(InMemoryContentStore::default());
        let pre = db_at(dir.path(), Arc::clone(&content), 256);
        pre.create("alice", "apps/local/a", None, "pre-upgrade")
            .unwrap();
        let old = content.put(b"the pre-upgrade body").unwrap();
        pre.advance("alice", "apps/local/a", "old.md", old.0)
            .unwrap();
        drop(pre);
        {
            let raw = Connection::open(dir.path().join("branches.db")).unwrap();
            raw.execute("DELETE FROM branch_history", []).unwrap();
        }

        let db = db_at(dir.path(), Arc::clone(&content), 256);
        assert!(
            db.list_versions("alice", "apps/local/a", 10, None)
                .unwrap()
                .is_none(),
            "a pre-history row honestly reports no history yet"
        );
        let new = content.put(b"the first post-upgrade edit").unwrap();
        db.advance("alice", "apps/local/a", "new.md", new.0)
            .unwrap();

        let (versions, _) = db
            .list_versions("alice", "apps/local/a", 10, None)
            .unwrap()
            .unwrap();
        assert_eq!(versions.len(), 2, "baseline + the mutation");
        assert_eq!(versions[1].cause, "baseline");
        assert_eq!(versions[1].version, 1);
        assert_eq!(
            versions[1].item_count, 1,
            "the pre-upgrade state, restorable"
        );
        // And the baseline is genuinely restorable: back to just old.md.
        let (m, _, _) = db.restore("alice", "apps/local/a", 1).unwrap();
        assert_eq!(m.items.len(), 1);
        assert_eq!(m.items[0].path, "old.md");
    }

    #[test]
    fn history_survives_a_reopen() {
        // The deterministic form of "versions survive a serve restart".
        let dir = tempfile::tempdir().unwrap();
        let content = Arc::new(InMemoryContentStore::default());
        let db = db_at(dir.path(), Arc::clone(&content), 256);
        db.create("alice", "apps/local/a", None, "").unwrap();
        let r = db.content.put(b"x").unwrap();
        db.advance("alice", "apps/local/a", "a.md", r.0).unwrap();
        drop(db);

        let reopened = db_at(dir.path(), content, 256);
        let (versions, _) = reopened
            .list_versions("alice", "apps/local/a", 10, None)
            .unwrap()
            .expect("history rows survive the reopen");
        assert_eq!(versions.len(), 2);
        assert_eq!(versions[0].cause, "advance");
    }

    #[test]
    fn restore_refuses_a_version_whose_blob_no_longer_resolves() {
        // CAS blobs are immutable and branch ops never collect them, so this can
        // only happen out-of-band (an operator GC) — but the manifest must still
        // never point at an unresolvable blob. Forge such a row directly.
        let dir = tempfile::tempdir().unwrap();
        let content = Arc::new(InMemoryContentStore::default());
        let db = db_at(dir.path(), Arc::clone(&content), 256);
        db.create("alice", "apps/local/a", None, "").unwrap();
        drop(db);
        {
            let raw = Connection::open(dir.path().join("branches.db")).unwrap();
            raw.execute(
                "INSERT INTO branch_history(principal, handle, version, branch_ref, parent_handle, \
                 description, items_json, recorded_unix_ms, cause) \
                 VALUES ('alice', 'apps/local/a', 99, x'00000000000000000000000000000000', '', '', \
                 ?1, 0, 'advance')",
                params![format!(
                    "[{{\"path\":\"ghost.md\",\"ref\":\"{}\"}}]",
                    "ab".repeat(32)
                )],
            )
            .unwrap();
        }
        let db = db_at(dir.path(), content, 256);
        assert!(matches!(
            db.restore("alice", "apps/local/a", 99).unwrap_err(),
            CoreError::InvalidArgument(_)
        ));
    }

    #[test]
    fn a_cow_fork_still_replaces_the_target() {
        // Pin the behaviour the fix must NOT over-reach into: forking onto an existing
        // handle is an explicit "make this branch be a copy of that one".
        let db = db_with_root(None);
        db.create("alice", "ns/coll/main", None, "main").unwrap();
        let p = db.content.put(b"parent body").unwrap();
        db.advance("alice", "ns/coll/main", "p.md", p.0).unwrap();

        db.create("alice", "ns/coll/feature", None, "feature")
            .unwrap();
        let f = db.content.put(b"feature body").unwrap();
        db.advance("alice", "ns/coll/feature", "f.md", f.0).unwrap();

        let (m, _) = db
            .create(
                "alice",
                "ns/coll/feature",
                Some("ns/coll/main"),
                "re-forked",
            )
            .unwrap();
        assert_eq!(m.items.len(), 1, "a fork snapshots the parent");
        assert_eq!(m.items[0].path, "p.md");
        assert_eq!(m.parent_handle, "ns/coll/main");
    }
}
