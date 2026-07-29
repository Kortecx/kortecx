//! The Workflow-catalog sidecar: `workflows.db` under `--catalog-dir`, backing
//! the [`WorkflowCatalog`] seam — `SaveWorkflow` / `ListWorkflows` /
//! `GetWorkflow` / `DeleteWorkflow` (+ the restore-resync `set_lifecycle`).
//!
//! ## Rebuildable-to-EMPTY (the `apps.db` posture, verbatim)
//! A workflow envelope references content-store blobs + registry ids; it is NOT
//! derivable from the journal. On corruption or a schema-version drift this
//! ledger recreates EMPTY — the only loss is the catalog index, and re-saving
//! the same envelope restores the SAME `workflow_ref` (content-addressed).
//! Never journaled, never a `MoteId` input, never a digest input.
//!
//! ## Server-derived id
//! `workflow_ref = blake3("kx-workflow\0" ‖ handle ‖ 0 ‖ canonical(envelope))[..16]`.
//! The host RE-CANONICALIZES the received bytes
//! ([`kx_app::workflow_canonical_json`]) so client byte-ordering never affects
//! identity, and validates the envelope — App-tagged bytes are REFUSED at this
//! seam (the schema mutual-exclusion contract), so the two catalogs can never
//! swallow each other's envelopes.
//!
//! ## Caller-scoped
//! The primary key is `(principal, handle)` — uniform not-found for absent OR
//! not-owned; no cross-party existence oracle.

use std::path::Path;
use std::sync::{Arc, Mutex};

use kx_gateway_core::GatewayError as CoreError;
use kx_gateway_core::{AppAuthor, AppRunError, BoundRecipe, WorkflowCatalog, WorkflowRecord};
use rusqlite::{params, Connection};

use crate::app_run::HostAppAuthor;
use crate::error::GatewayError;

/// Bump on any table-shape change. Unknown/missing version ⇒ recreate EMPTY
/// (workflows are not journal-derivable, so there is no rebuild — re-save).
/// Additive columns NEVER bump this — they go through [`WorkflowsDb::ensure_column`].
const SCHEMA_VERSION: i64 = 1;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS workflows (
    principal     TEXT NOT NULL,   -- server-resolved caller party (scope)
    handle        TEXT NOT NULL,   -- AssetPath 'ns/coll/name' (upsert key within principal)
    workflow_ref  BLOB NOT NULL,   -- 16B server-derived canonical-envelope hash (display/dedup)
    name          TEXT NOT NULL,   -- envelope name (denormalized summary)
    version       TEXT NOT NULL,   -- envelope version
    description   TEXT NOT NULL,   -- advisory, never parsed for enforcement
    delivers      TEXT NOT NULL DEFAULT '', -- advisory one-line output contract; what a composition menu renders
    tags_json     TEXT NOT NULL,   -- JSON [string] (denormalized summary)
    step_count    INTEGER NOT NULL,-- blueprint step count (display)
    envelope_json TEXT NOT NULL,   -- the CANONICAL kortecx.workflow/v1 envelope bytes
    source_digest BLOB,            -- OPTIONAL 32B lineage hint (clone source workflow_digest); NULL = authored-here. Off-identity/off-journal/off-digest.
    lifecycle     TEXT NOT NULL DEFAULT '', -- catalog lifecycle ('' active / 'draft'); CALLER-STATED at save (no scaffold loop exists to own it). Display/routing only, off-identity.
    PRIMARY KEY (principal, handle)
);
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value INTEGER NOT NULL);
";

/// `workflow_ref = blake3("kx-workflow\0" ‖ handle ‖ 0 ‖ canonical_envelope)[..16]`.
pub(crate) fn workflow_ref_of(handle: &str, canonical: &[u8]) -> [u8; 16] {
    let mut keyed = Vec::with_capacity(16 + handle.len() + canonical.len());
    keyed.extend_from_slice(b"kx-workflow\0");
    keyed.extend_from_slice(handle.as_bytes());
    keyed.push(0);
    keyed.extend_from_slice(canonical);
    let mut id = [0u8; 16];
    id.copy_from_slice(&kx_content::ContentRef::of(&keyed).0[..16]);
    id
}

/// The durable Workflow catalog over `workflows.db`. A single mutex'd
/// connection: workflow authoring is interactive-rate, never contended.
pub(crate) struct WorkflowsDb {
    conn: Mutex<Connection>,
}

/// The workflow-pointer → run resolver behind `RunWorkflow`: a THIN wrapper
/// over the SAME [`HostAppAuthor`] the App path uses — one preparation /
/// composition / authoring pipeline, entered with the stored
/// `kortecx.workflow/v1` envelope's lossless Functional-App view.
///
/// Identity discipline: the conversion happens ONLY here, at author time.
/// `workflow_ref` / `workflow_digest` are computed over the WORKFLOW canonical
/// bytes at save; nothing downstream of this seam ever derives identity from
/// the converted form. Wishes-never-grants holds by construction — every
/// warrant is still minted inside the shared author from the party's own
/// grants, and the envelope carries no authority.
///
/// A workflow's `app`-composing steps resolve SAVED APPS through the exact
/// D198 seam (cycle guard, depth ceiling, step ceiling) — callees always come
/// from the App catalog, whatever entity the root graph came from.
pub(crate) struct HostWorkflowRunner {
    inner: Arc<HostAppAuthor>,
    workflows: Arc<WorkflowsDb>,
}

impl HostWorkflowRunner {
    pub(crate) fn new(inner: Arc<HostAppAuthor>, workflows: Arc<WorkflowsDb>) -> Self {
        Self { inner, workflows }
    }
}

#[tonic::async_trait]
impl AppAuthor for HostWorkflowRunner {
    async fn author_app(
        &self,
        party: &str,
        handle: &str,
        args: &[u8],
        require_approval: bool,
    ) -> Result<BoundRecipe, AppRunError> {
        // (1) Read the validated stored WORKFLOW envelope (uniform not-found so
        //     an unauthorized caller learns nothing about what exists).
        let (_, envelope_bytes) = self
            .workflows
            .get(party, handle)
            .map_err(|e| AppRunError::Internal(format!("workflows.db read: {e}")))?
            .ok_or(AppRunError::NotAuthorized)?;
        let wf = kx_app::WorkflowEnvelope::from_json_slice(&envelope_bytes)
            .map_err(|e| AppRunError::Internal(format!("stored workflow envelope invalid: {e}")))?;
        // (2) The lossless Functional-App view enters the SHARED pipeline:
        //     context rail, connection/secret resolution, canonical lowering,
        //     per-step binds, model-route wish ∩ served catalog, HITL stamping.
        let env = wf.into_app_envelope();
        let mut prepared = self
            .inner
            .prepare_env(party, env, handle, args, require_approval)
            .await?;
        // (3) Composed APP steps resolve through the D198 seam; the chain seeds
        //     with the workflow handle so a cycle refusal names the whole path.
        let mut chain = vec![handle.to_string()];
        self.inner
            .resolve_composes(&mut prepared, party, handle, require_approval, 0, &mut chain)
            .await?;
        // (4) One canonical lowering + server-side authoring (wishes ∩ grants).
        self.inner.author_prepared(party, prepared).await
    }
}

impl WorkflowsDb {
    /// Open (or create) `workflows.db` under `dir`. A corrupt/foreign file or a
    /// `schema_version` drift recreates the catalog EMPTY (module doc).
    ///
    /// # Errors
    /// [`GatewayError::Catalog`] on an unrecoverable open/pragma failure.
    pub(crate) fn open(dir: &Path) -> Result<Self, GatewayError> {
        std::fs::create_dir_all(dir)
            .map_err(|e| GatewayError::Catalog(format!("workflows dir: {e}")))?;
        let db_path = dir.join("workflows.db");
        let conn = if let Ok(c) = Self::open_with_pragma(&db_path) {
            c
        } else {
            let _ = std::fs::remove_file(&db_path);
            let _ = std::fs::remove_file(dir.join("workflows.db-wal"));
            let _ = std::fs::remove_file(dir.join("workflows.db-shm"));
            Self::open_with_pragma(&db_path)
                .map_err(|e| GatewayError::Catalog(format!("workflows reopen: {e}")))?
        };
        let fresh_or_stale = match Self::read_schema_version(&conn) {
            Ok(Some(v)) => v != SCHEMA_VERSION,
            Ok(None) | Err(_) => true,
        };
        if fresh_or_stale {
            conn.execute_batch("DROP TABLE IF EXISTS workflows; DROP TABLE IF EXISTS meta;")
                .map_err(|e| GatewayError::Catalog(format!("workflows rebuild: {e}")))?;
        }
        conn.execute_batch(SCHEMA)
            .map_err(|e| GatewayError::Catalog(format!("workflows schema: {e}")))?;
        conn.execute(
            "INSERT OR IGNORE INTO meta(key, value) VALUES ('schema_version', ?1)",
            params![SCHEMA_VERSION],
        )
        .map_err(|e| GatewayError::Catalog(format!("workflows meta init: {e}")))?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn open_with_pragma(db_path: &Path) -> rusqlite::Result<Connection> {
        let conn = Connection::open(db_path)?;
        conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;")?;
        Ok(conn)
    }

    /// Idempotently add one additive column, given its `ALTER TABLE` declaration.
    ///
    /// Pre-installed from day one (the `apps.db` discipline): every future
    /// column addition goes through here and NEVER bumps [`SCHEMA_VERSION`] —
    /// a bump would drop saved workflows, and old binaries ignore an unknown
    /// column because every SELECT/INSERT names its columns explicitly.
    /// v1 has no migrations yet, so nothing calls it outside tests.
    #[allow(dead_code)]
    fn ensure_column(conn: &Connection, column: &str, decl: &str) -> rusqlite::Result<()> {
        let mut stmt = conn.prepare("PRAGMA table_info(workflows)")?;
        let present = stmt
            .query_map([], |r| r.get::<_, String>(1))?
            .filter_map(Result::ok)
            .any(|col| col == column);
        drop(stmt);
        if !present {
            conn.execute(decl, [])?;
        }
        Ok(())
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

    /// Build a [`WorkflowRecord`] from a row, reading every column BY NAME
    /// (the `apps.rs` positional-read lesson: two callers select different
    /// column sets, and hand-counted indices go silently wrong).
    fn row_to_record(r: &rusqlite::Row<'_>) -> rusqlite::Result<WorkflowRecord> {
        let workflow_ref = r.get::<_, Vec<u8>>("workflow_ref")?;
        let mut id = [0u8; 16];
        let n = workflow_ref.len().min(16);
        id[..n].copy_from_slice(&workflow_ref[..n]);
        let tags_json = r.get::<_, String>("tags_json")?;
        Ok(WorkflowRecord {
            workflow_ref: id,
            handle: r.get("handle")?,
            name: r.get("name")?,
            version: r.get("version")?,
            description: r.get("description")?,
            delivers: r.get("delivers")?,
            tags: serde_json::from_str(&tags_json).unwrap_or_default(),
            step_count: u32::try_from(r.get::<_, i64>("step_count")?).unwrap_or(u32::MAX),
            source_digest: r.get("source_digest")?,
            lifecycle: r.get("lifecycle")?,
        })
    }
}

impl WorkflowCatalog for WorkflowsDb {
    fn save(
        &self,
        principal: &str,
        handle: &str,
        envelope_json: &[u8],
        source_digest: Option<&[u8]>,
        lifecycle: &str,
    ) -> Result<(WorkflowRecord, bool), CoreError> {
        // Validate + canonicalize (the envelope carries NO authority, and
        // App-tagged bytes are refused here — the mutual-exclusion contract).
        let canonical = kx_app::workflow_canonical_json(envelope_json)
            .map_err(|_| CoreError::InvalidArgument("invalid workflow envelope"))?;
        let summary = kx_app::workflow_summary_of(envelope_json)
            .map_err(|_| CoreError::InvalidArgument("invalid workflow envelope"))?;
        let workflow_ref = workflow_ref_of(handle, &canonical);
        let canonical_str = String::from_utf8(canonical)
            .map_err(|_| CoreError::Internal("canonical envelope is not UTF-8".into()))?;
        let tags_json = serde_json::to_string(&summary.tags)
            .map_err(|e| CoreError::Internal(format!("workflows tags encode: {e}")))?;
        let conn = self
            .conn
            .lock()
            .map_err(|_| CoreError::Internal("workflows lock poisoned".into()))?;
        // Dedup = identical canonical bytes AND identical lifecycle. Lifecycle is
        // CALLER-STATED here (unlike apps, where the scaffold path owns it and
        // save must preserve): a lifecycle flip on identical bytes — finishing a
        // draft — is a REAL write and must not read as a no-op.
        let existing: Option<(Vec<u8>, String)> = conn
            .query_row(
                "SELECT workflow_ref, lifecycle FROM workflows WHERE principal = ?1 AND handle = ?2",
                params![principal, handle],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })
            .map_err(|e| CoreError::Internal(format!("workflows dedup probe: {e}")))?;
        let deduplicated = existing
            .as_ref()
            .is_some_and(|(r, l)| r.as_slice() == &workflow_ref[..] && l == lifecycle);
        let source_digest = source_digest.map(<[u8]>::to_vec);
        conn.execute(
            "INSERT OR REPLACE INTO workflows(principal, handle, workflow_ref, name, version, \
             description, delivers, tags_json, step_count, envelope_json, source_digest, lifecycle) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                principal,
                handle,
                workflow_ref.to_vec(),
                summary.name,
                summary.version,
                summary.description,
                summary.delivers,
                tags_json,
                i64::from(summary.step_count),
                canonical_str,
                source_digest,
                lifecycle,
            ],
        )
        .map_err(|e| CoreError::Internal(format!("workflows upsert: {e}")))?;
        Ok((
            WorkflowRecord {
                workflow_ref,
                handle: handle.to_string(),
                name: summary.name,
                version: summary.version,
                description: summary.description,
                delivers: summary.delivers,
                tags: summary.tags,
                step_count: summary.step_count,
                source_digest,
                lifecycle: lifecycle.to_string(),
            },
            deduplicated,
        ))
    }

    fn list(
        &self,
        principal: &str,
        limit: usize,
        after_handle: Option<&str>,
    ) -> Result<(Vec<WorkflowRecord>, bool), CoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| CoreError::Internal("workflows lock poisoned".into()))?;
        let cursor = after_handle.unwrap_or("");
        let mut stmt = conn
            .prepare(
                "SELECT handle, workflow_ref, name, version, description, delivers, tags_json, \
                 step_count, source_digest, lifecycle \
                 FROM workflows WHERE principal = ?1 AND handle > ?2 ORDER BY handle ASC LIMIT ?3",
            )
            .map_err(|e| CoreError::Internal(format!("workflows list prepare: {e}")))?;
        let fetch = i64::try_from(limit.saturating_add(1)).unwrap_or(i64::MAX);
        let rows = stmt
            .query_map(params![principal, cursor, fetch], Self::row_to_record)
            .map_err(|e| CoreError::Internal(format!("workflows list query: {e}")))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| CoreError::Internal(format!("workflows list row: {e}")))?);
        }
        let has_more = out.len() > limit;
        out.truncate(limit);
        Ok((out, has_more))
    }

    fn get(
        &self,
        principal: &str,
        handle: &str,
    ) -> Result<Option<(WorkflowRecord, Vec<u8>)>, CoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| CoreError::Internal("workflows lock poisoned".into()))?;
        conn.query_row(
            "SELECT handle, workflow_ref, name, version, description, delivers, tags_json, \
             step_count, envelope_json, source_digest, lifecycle \
             FROM workflows WHERE principal = ?1 AND handle = ?2",
            params![principal, handle],
            |r| {
                let record = Self::row_to_record(r)?;
                let envelope_json = r.get::<_, String>("envelope_json")?.into_bytes();
                Ok((record, envelope_json))
            },
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })
        .map_err(|e| CoreError::Internal(format!("workflows get: {e}")))
    }

    fn delete(&self, principal: &str, handle: &str) -> Result<bool, CoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| CoreError::Internal("workflows lock poisoned".into()))?;
        // Scoped by principal in SQL — `false` covers absent AND not-owned uniformly.
        let n = conn
            .execute(
                "DELETE FROM workflows WHERE principal = ?1 AND handle = ?2",
                params![principal, handle],
            )
            .map_err(|e| CoreError::Internal(format!("workflows delete: {e}")))?;
        Ok(n > 0)
    }

    fn set_lifecycle(
        &self,
        principal: &str,
        handle: &str,
        lifecycle: &str,
    ) -> Result<bool, CoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| CoreError::Internal("workflows lock poisoned".into()))?;
        // A plain column UPDATE — never touches the envelope/workflow_ref, so it
        // can never move identity. Caller-scoped in SQL (the `delete` posture).
        let n = conn
            .execute(
                "UPDATE workflows SET lifecycle = ?3 WHERE principal = ?1 AND handle = ?2",
                params![principal, handle, lifecycle],
            )
            .map_err(|e| CoreError::Internal(format!("workflows lifecycle: {e}")))?;
        Ok(n > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::app_ref_of;

    fn envelope(name: &str) -> Vec<u8> {
        let env = kx_app::WorkflowEnvelope::new(name, serde_json::json!({ "steps": [] }));
        env.to_canonical_json().unwrap()
    }

    fn tmp_dir() -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        let stamp = format!("kx-workflows-test-{}-{:p}", std::process::id(), &p);
        p.push(stamp);
        p
    }

    #[test]
    fn workflow_ref_is_handle_scoped_and_domain_separated() {
        use kx_gateway_core::workflow_digest_of;
        let canonical = envelope("triage");
        // Same envelope under two handles: workflow_ref DIFFERS ...
        assert_ne!(
            workflow_ref_of("team/wf/a", &canonical),
            workflow_ref_of("team/wf/b", &canonical)
        );
        // ... but workflow_digest is the SAME portable, handle-free id, and is
        // domain-separated from workflow_ref.
        let digest = workflow_digest_of(&canonical);
        assert_eq!(digest.len(), 32);
        assert_ne!(&digest[..16], &workflow_ref_of("team/wf/a", &canonical)[..]);
        // Cross-ENTITY domain separation: the same handle + the same bytes can
        // never yield the same id in the App and Workflow catalogs.
        assert_ne!(
            workflow_ref_of("team/x/y", &canonical),
            app_ref_of("team/x/y", &canonical)
        );
    }

    #[test]
    fn save_get_list_round_trip_with_lifecycle_dedup() {
        let dir = tmp_dir();
        let db = WorkflowsDb::open(&dir).unwrap();
        let (rec, dedup) = db
            .save("alice", "team/wf/triage", &envelope("triage"), None, "draft")
            .unwrap();
        assert!(!dedup);
        assert_eq!(rec.name, "triage");
        assert_eq!(rec.lifecycle, "draft");
        // Identical bytes + identical lifecycle ⇒ dedup.
        let (_, dedup2) = db
            .save("alice", "team/wf/triage", &envelope("triage"), None, "draft")
            .unwrap();
        assert!(dedup2);
        // Identical bytes + a lifecycle FLIP (finishing the draft) is a REAL
        // write, never a dedup no-op.
        let (rec3, dedup3) = db
            .save("alice", "team/wf/triage", &envelope("triage"), None, "")
            .unwrap();
        assert!(!dedup3, "a draft being finished must not read as a no-op");
        assert_eq!(rec3.lifecycle, "");
        let (got, bytes) = db.get("alice", "team/wf/triage").unwrap().unwrap();
        assert_eq!(got.workflow_ref, rec.workflow_ref);
        assert_eq!(got.lifecycle, "");
        assert_eq!(bytes, envelope("triage"));
        let (wfs, has_more) = db.list("alice", 100, None).unwrap();
        assert_eq!(wfs.len(), 1);
        assert!(!has_more);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cross_party_isolation_is_uniform_not_found() {
        let dir = tmp_dir();
        let db = WorkflowsDb::open(&dir).unwrap();
        db.save("alice", "team/wf/secret", &envelope("secret"), None, "")
            .unwrap();
        assert!(db.get("bob", "team/wf/secret").unwrap().is_none());
        let (wfs, _) = db.list("bob", 100, None).unwrap();
        assert!(wfs.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bad_and_app_tagged_envelopes_are_invalid_argument() {
        let dir = tmp_dir();
        let db = WorkflowsDb::open(&dir).unwrap();
        let err = db
            .save("alice", "team/wf/bad", b"{not json", None, "")
            .unwrap_err();
        assert!(matches!(err, CoreError::InvalidArgument(_)));
        // The mutual-exclusion contract: an APP envelope's bytes are refused by
        // the WORKFLOW catalog (and vice versa at the App seam), so neither
        // catalog can swallow the other's entities.
        let app = kx_app::AppEnvelope::new("app", serde_json::json!({ "steps": [] }));
        let err = db
            .save(
                "alice",
                "team/wf/app",
                &app.to_canonical_json().unwrap(),
                None,
                "",
            )
            .unwrap_err();
        assert!(matches!(err, CoreError::InvalidArgument(_)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn source_digest_round_trips_and_is_off_identity() {
        let dir = tmp_dir();
        let db = WorkflowsDb::open(&dir).unwrap();
        let sd = vec![0x22u8; 32];
        let (with_hint, _) = db
            .save("alice", "team/wf/x", &envelope("x"), Some(&sd), "")
            .unwrap();
        assert_eq!(with_hint.source_digest.as_deref(), Some(&sd[..]));
        let (got, _) = db.get("alice", "team/wf/x").unwrap().unwrap();
        assert_eq!(got.source_digest.as_deref(), Some(&sd[..]));
        // Off-identity: the SAME envelope re-saved WITHOUT a hint yields the
        // SAME workflow_ref and dedups (lifecycle unchanged).
        let (no_hint, dedup) = db
            .save("alice", "team/wf/x", &envelope("x"), None, "")
            .unwrap();
        assert_eq!(with_hint.workflow_ref, no_hint.workflow_ref);
        assert!(dedup);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn delete_removes_only_the_callers_own_row() {
        let dir = tmp_dir();
        let db = WorkflowsDb::open(&dir).unwrap();
        db.save("alice", "team/wf/x", &envelope("x"), None, "")
            .unwrap();
        db.save("alice", "team/wf/keep", &envelope("keep"), None, "")
            .unwrap();
        db.save("bob", "team/wf/x", &envelope("x"), None, "")
            .unwrap();
        assert!(db.delete("alice", "team/wf/x").unwrap());
        assert!(db.get("alice", "team/wf/x").unwrap().is_none());
        assert!(db.get("alice", "team/wf/keep").unwrap().is_some());
        assert!(db.get("bob", "team/wf/x").unwrap().is_some());
        // Uniform `false` — absent and not-owned are indistinguishable.
        assert!(!db.delete("alice", "team/wf/x").unwrap());
        assert!(!db.delete("mallory", "team/wf/keep").unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The REAL store's `set_lifecycle` — required (not defaulted) on this
    /// trait, and pinned here against the real impl anyway: the resync path
    /// (branch restore) depends on it carrying the row's lifecycle through.
    #[test]
    fn set_lifecycle_round_trips_on_the_real_store() {
        let dir = tmp_dir();
        let db = WorkflowsDb::open(&dir).unwrap();
        db.save("alice", "team/wf/a", &envelope("a"), None, "")
            .unwrap();
        assert!(db.set_lifecycle("alice", "team/wf/a", "draft").unwrap());
        let (rec, _) = db.get("alice", "team/wf/a").unwrap().unwrap();
        assert_eq!(rec.lifecycle, "draft");
        assert!(db.set_lifecycle("alice", "team/wf/a", "").unwrap());
        // Absent / not-owned: uniform false, nothing written.
        assert!(!db.set_lifecycle("alice", "team/wf/missing", "draft").unwrap());
        assert!(!db.set_lifecycle("bob", "team/wf/a", "draft").unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn schema_drift_rebuilds_empty() {
        let dir = tmp_dir();
        {
            let db = WorkflowsDb::open(&dir).unwrap();
            db.save("alice", "team/wf/x", &envelope("x"), None, "")
                .unwrap();
        }
        {
            let conn = Connection::open(dir.join("workflows.db")).unwrap();
            conn.execute(
                "UPDATE meta SET value = 999 WHERE key = 'schema_version'",
                [],
            )
            .unwrap();
        }
        let db = WorkflowsDb::open(&dir).unwrap();
        let (wfs, _) = db.list("alice", 100, None).unwrap();
        assert!(wfs.is_empty(), "schema drift must rebuild empty");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The pre-installed additive-column discipline: `ensure_column` is
    /// idempotent and adds without a version bump (rows survive).
    #[test]
    fn ensure_column_is_idempotent_and_preserves_rows() {
        let dir = tmp_dir();
        let db = WorkflowsDb::open(&dir).unwrap();
        db.save("alice", "team/wf/x", &envelope("x"), None, "")
            .unwrap();
        {
            let conn = db.conn.lock().unwrap();
            WorkflowsDb::ensure_column(
                &conn,
                "future_col",
                "ALTER TABLE workflows ADD COLUMN future_col TEXT NOT NULL DEFAULT ''",
            )
            .unwrap();
            // Second call is a no-op, not an error.
            WorkflowsDb::ensure_column(
                &conn,
                "future_col",
                "ALTER TABLE workflows ADD COLUMN future_col TEXT NOT NULL DEFAULT ''",
            )
            .unwrap();
        }
        let (wfs, _) = db.list("alice", 100, None).unwrap();
        assert_eq!(wfs.len(), 1, "rows survive an additive column");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
