//! The RC-SW1 skill-catalog sidecar: `skills.db` under `--catalog-dir`, backing
//! the [`SkillCatalog`] seam — `ListSkills` / `GetSkillForm` / `AddSkill` /
//! `RemoveSkill`.
//!
//! ## Rebuildable-to-EMPTY (the `apps.db` posture)
//! A skill references a content-store blob (`instructions_ref`) + registry ids;
//! it is NOT derivable from the journal. Truth (the blob + the registries) lives
//! elsewhere, so on corruption or a schema-version drift this ledger recreates
//! EMPTY — the only loss is the catalog index, and re-adding the same pack
//! restores the SAME `skill_ref` (content-addressed). Never journaled, never a
//! `MoteId` input, never a digest input.
//!
//! ## Server-derived id (SN-8)
//! `skill_ref = blake3("kx-skill\0" ‖ name ‖ 0 ‖ canonical(manifest))[..16]` via
//! [`kx_content::ContentRef::of`] (the `app_ref_of` precedent). The host
//! RE-CANONICALIZES the received bytes via `kx-skill` so client byte-ordering
//! never affects identity, and validates the manifest fail-closed — it carries
//! NO authority (deny-keys) and NO code.
//!
//! ## Two add forms, one unambiguous identity
//! - PACK form: the handler stored `instructions.md` and passes
//!   [`AddedInstructions`] — the manifest must NOT name `instructions_ref`
//!   (the server splices the derived ref before canonicalizing).
//! - STORED form: no body — the manifest MUST name a 64-hex `instructions_ref`.
//!
//! Both a body AND a manifest-carried ref is refused (an ambiguous identity).
//!
//! ## Caller-scoped
//! The primary key is `(principal, name)` — a skill is visible only to the
//! SERVER-RESOLVED party that added it (uniform not-found; no existence oracle).

use std::path::Path;
use std::sync::Mutex;

use kx_gateway_core::GatewayError as CoreError;
use kx_gateway_core::{AddedInstructions, SkillCatalog, SkillRecord};
use rusqlite::{params, Connection};

use crate::error::GatewayError;

/// Bump on any table-shape change. Unknown/missing version ⇒ recreate EMPTY
/// (skills are not journal-derivable, so there is no rebuild — re-add).
const SCHEMA_VERSION: i64 = 1;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS skills (
    principal            TEXT NOT NULL,   -- server-resolved caller party (scope)
    name                 TEXT NOT NULL,   -- the manifest name (upsert key within principal)
    skill_ref            BLOB NOT NULL,   -- 16B server-derived canonical-manifest hash
    version              TEXT NOT NULL,   -- manifest version (integer string)
    description          TEXT NOT NULL,   -- advisory, never parsed for enforcement
    tags_json            TEXT NOT NULL,   -- JSON [string] (denormalized)
    tools_json           TEXT NOT NULL,   -- JSON {id: version} — the WISH set (denormalized)
    instructions_ref     TEXT NOT NULL,   -- 64-hex content-store ref
    instructions_preview TEXT NOT NULL,   -- capped display excerpt ('' on a ref-only add)
    preview_truncated    INTEGER NOT NULL,
    manifest_json        TEXT NOT NULL,   -- the CANONICAL kortecx.skill/v1 bytes (STORED form)
    PRIMARY KEY (principal, name)
);
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value INTEGER NOT NULL);
";

/// `skill_ref = blake3("kx-skill\0" ‖ name ‖ 0 ‖ canonical_manifest)[..16]` (SN-8).
fn skill_ref_of(name: &str, canonical: &[u8]) -> [u8; 16] {
    let mut keyed = Vec::with_capacity(16 + name.len() + canonical.len());
    keyed.extend_from_slice(b"kx-skill\0");
    keyed.extend_from_slice(name.as_bytes());
    keyed.push(0);
    keyed.extend_from_slice(canonical);
    let mut id = [0u8; 16];
    id.copy_from_slice(&kx_content::ContentRef::of(&keyed).0[..16]);
    id
}

fn hex32(bytes: &[u8; 32]) -> String {
    let mut s = String::with_capacity(64);
    for b in bytes {
        use std::fmt::Write as _;
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// The durable skill catalog over `skills.db`. A single mutex'd connection:
/// skill authoring is interactive-rate (a CLI/SDK add / a catalog list).
pub(crate) struct SkillsDb {
    conn: Mutex<Connection>,
}

impl SkillsDb {
    /// Open (or create) `skills.db` under `dir`. A corrupt/foreign file or a
    /// `schema_version` drift recreates the catalog EMPTY (module doc).
    ///
    /// # Errors
    /// [`GatewayError::Catalog`] on an unrecoverable open/pragma failure.
    pub(crate) fn open(dir: &Path) -> Result<Self, GatewayError> {
        std::fs::create_dir_all(dir)
            .map_err(|e| GatewayError::Catalog(format!("skills dir: {e}")))?;
        let db_path = dir.join("skills.db");
        let conn = if let Ok(c) = Self::open_with_pragma(&db_path) {
            c
        } else {
            let _ = std::fs::remove_file(&db_path);
            let _ = std::fs::remove_file(dir.join("skills.db-wal"));
            let _ = std::fs::remove_file(dir.join("skills.db-shm"));
            Self::open_with_pragma(&db_path)
                .map_err(|e| GatewayError::Catalog(format!("skills reopen: {e}")))?
        };
        let fresh_or_stale = match Self::read_schema_version(&conn) {
            Ok(Some(v)) => v != SCHEMA_VERSION,
            Ok(None) | Err(_) => true,
        };
        if fresh_or_stale {
            conn.execute_batch("DROP TABLE IF EXISTS skills; DROP TABLE IF EXISTS meta;")
                .map_err(|e| GatewayError::Catalog(format!("skills rebuild: {e}")))?;
        }
        conn.execute_batch(SCHEMA)
            .map_err(|e| GatewayError::Catalog(format!("skills schema: {e}")))?;
        conn.execute(
            "INSERT OR IGNORE INTO meta(key, value) VALUES ('schema_version', ?1)",
            params![SCHEMA_VERSION],
        )
        .map_err(|e| GatewayError::Catalog(format!("skills meta init: {e}")))?;
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

    /// Resolve the STORED-form manifest for `(principal, manifest_json,
    /// instructions)` — the two-form rule in the module doc. Returns the typed
    /// manifest + its preview columns.
    fn resolve_stored_form(
        manifest_json: &[u8],
        instructions: Option<AddedInstructions>,
    ) -> Result<(kx_skill::SkillManifest, String, bool), CoreError> {
        if let Some(added) = instructions {
            // PACK form: the manifest must NOT carry a ref; splice the
            // server-derived one, then re-validate in stored form.
            let mut m =
                kx_skill::SkillManifest::from_json_slice_pack(manifest_json).map_err(|_| {
                    CoreError::InvalidArgument(
                        "invalid skill manifest (pack form: kortecx.skill/v1, no authority \
                         keys, no instructions_ref when a body is supplied)",
                    )
                })?;
            m.instructions_ref = hex32(&added.content_ref);
            m.validate_stored()
                .map_err(|_| CoreError::InvalidArgument("invalid skill manifest"))?;
            Ok((m, added.preview, added.truncated))
        } else {
            // STORED form: the manifest must already carry the 64-hex ref.
            let m =
                kx_skill::SkillManifest::from_json_slice_stored(manifest_json).map_err(|_| {
                    CoreError::InvalidArgument(
                        "invalid skill manifest (stored form: kortecx.skill/v1, no authority \
                         keys, a 64-hex instructions_ref when no body is supplied)",
                    )
                })?;
            Ok((m, String::new(), false))
        }
    }

    fn row_to_record(r: &rusqlite::Row<'_>) -> rusqlite::Result<SkillRecord> {
        let skill_ref_bytes = r.get::<_, Vec<u8>>(1)?;
        let mut skill_ref = [0u8; 16];
        let n = skill_ref_bytes.len().min(16);
        skill_ref[..n].copy_from_slice(&skill_ref_bytes[..n]);
        let tags_json = r.get::<_, String>(4)?;
        let tools_json = r.get::<_, String>(5)?;
        Ok(SkillRecord {
            name: r.get::<_, String>(0)?,
            skill_ref,
            version: r.get::<_, String>(2)?,
            description: r.get::<_, String>(3)?,
            tags: serde_json::from_str(&tags_json).unwrap_or_default(),
            tools: serde_json::from_str(&tools_json).unwrap_or_default(),
            instructions_ref: r.get::<_, String>(6)?,
            instructions_preview: r.get::<_, String>(7)?,
            preview_truncated: r.get::<_, i64>(8)? != 0,
        })
    }
}

const RECORD_COLUMNS: &str = "name, skill_ref, version, description, tags_json, tools_json, \
                              instructions_ref, instructions_preview, preview_truncated";

impl SkillCatalog for SkillsDb {
    fn add(
        &self,
        principal: &str,
        manifest_json: &[u8],
        instructions: Option<AddedInstructions>,
    ) -> Result<(SkillRecord, bool), CoreError> {
        let (manifest, preview, truncated) =
            Self::resolve_stored_form(manifest_json, instructions)?;
        let canonical = manifest
            .to_canonical_json()
            .map_err(|e| CoreError::Internal(format!("skill canonicalize: {e}")))?;
        let skill_ref = skill_ref_of(&manifest.name, &canonical);
        let canonical_str = String::from_utf8(canonical)
            .map_err(|_| CoreError::Internal("canonical manifest is not UTF-8".into()))?;
        let tags_json = serde_json::to_string(&manifest.tags)
            .map_err(|e| CoreError::Internal(format!("skill tags encode: {e}")))?;
        let tools_json = serde_json::to_string(&manifest.tools)
            .map_err(|e| CoreError::Internal(format!("skill tools encode: {e}")))?;
        let conn = self
            .conn
            .lock()
            .map_err(|_| CoreError::Internal("skills lock poisoned".into()))?;
        // Dedup signal: an identical canonical manifest already bound to (principal, name).
        let existing: Option<Vec<u8>> = conn
            .query_row(
                "SELECT skill_ref FROM skills WHERE principal = ?1 AND name = ?2",
                params![principal, manifest.name],
                |r| r.get(0),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })
            .map_err(|e| CoreError::Internal(format!("skills dedup probe: {e}")))?;
        let deduplicated = existing.as_deref() == Some(&skill_ref[..]);
        conn.execute(
            "INSERT OR REPLACE INTO skills(principal, name, skill_ref, version, description, \
             tags_json, tools_json, instructions_ref, instructions_preview, preview_truncated, \
             manifest_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                principal,
                manifest.name,
                skill_ref.to_vec(),
                manifest.version,
                manifest.description,
                tags_json,
                tools_json,
                manifest.instructions_ref,
                preview,
                i64::from(truncated),
                canonical_str,
            ],
        )
        .map_err(|e| CoreError::Internal(format!("skills upsert: {e}")))?;
        Ok((
            SkillRecord {
                skill_ref,
                name: manifest.name,
                version: manifest.version,
                description: manifest.description,
                tags: manifest.tags,
                instructions_ref: manifest.instructions_ref,
                tools: manifest.tools,
                instructions_preview: preview,
                preview_truncated: truncated,
            },
            deduplicated,
        ))
    }

    fn list(
        &self,
        principal: &str,
        limit: usize,
        after_name: Option<&str>,
    ) -> Result<(Vec<SkillRecord>, bool), CoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| CoreError::Internal("skills lock poisoned".into()))?;
        let cursor = after_name.unwrap_or("");
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {RECORD_COLUMNS} FROM skills \
                 WHERE principal = ?1 AND name > ?2 ORDER BY name ASC LIMIT ?3"
            ))
            .map_err(|e| CoreError::Internal(format!("skills list prepare: {e}")))?;
        let fetch = i64::try_from(limit.saturating_add(1)).unwrap_or(i64::MAX);
        let rows = stmt
            .query_map(params![principal, cursor, fetch], |r| {
                Self::row_to_record(r)
            })
            .map_err(|e| CoreError::Internal(format!("skills list query: {e}")))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| CoreError::Internal(format!("skills list row: {e}")))?);
        }
        let has_more = out.len() > limit;
        out.truncate(limit);
        Ok((out, has_more))
    }

    fn get(&self, principal: &str, name: &str) -> Result<Option<SkillRecord>, CoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| CoreError::Internal("skills lock poisoned".into()))?;
        conn.query_row(
            &format!("SELECT {RECORD_COLUMNS} FROM skills WHERE principal = ?1 AND name = ?2"),
            params![principal, name],
            Self::row_to_record,
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })
        .map_err(|e| CoreError::Internal(format!("skills get: {e}")))
    }

    fn remove(&self, principal: &str, name: &str) -> Result<bool, CoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| CoreError::Internal("skills lock poisoned".into()))?;
        let n = conn
            .execute(
                "DELETE FROM skills WHERE principal = ?1 AND name = ?2",
                params![principal, name],
            )
            .map_err(|e| CoreError::Internal(format!("skills remove: {e}")))?;
        Ok(n > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pack_manifest(name: &str) -> Vec<u8> {
        format!(
            r#"{{"schema":"kortecx.skill/v1","name":"{name}","version":"1","description":"d","tags":["t"],"tools":{{"gmail/search":"1"}}}}"#
        )
        .into_bytes()
    }

    fn added(body: &[u8]) -> AddedInstructions {
        AddedInstructions {
            content_ref: kx_content::ContentRef::of(body).0,
            preview: String::from_utf8_lossy(body).into_owned(),
            truncated: false,
        }
    }

    fn tmp_dir() -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        let stamp = format!("kx-skills-test-{}-{:p}", std::process::id(), &p);
        p.push(stamp);
        p
    }

    #[test]
    fn add_get_list_remove_round_trip_and_dedup() {
        let dir = tmp_dir();
        let db = SkillsDb::open(&dir).unwrap();
        let (rec, dedup) = db
            .add("alice", &pack_manifest("triage"), Some(added(b"# Triage")))
            .unwrap();
        assert!(!dedup);
        assert_eq!(rec.name, "triage");
        assert_eq!(rec.instructions_ref.len(), 64);
        assert_eq!(rec.tools["gmail/search"], "1");
        assert_eq!(rec.instructions_preview, "# Triage");
        // identical re-add dedups to the SAME server-derived ref.
        let (rec2, dedup2) = db
            .add("alice", &pack_manifest("triage"), Some(added(b"# Triage")))
            .unwrap();
        assert!(dedup2);
        assert_eq!(rec2.skill_ref, rec.skill_ref);
        // a DIFFERENT body moves the identity (not a dedup).
        let (rec3, dedup3) = db
            .add("alice", &pack_manifest("triage"), Some(added(b"# Other")))
            .unwrap();
        assert!(!dedup3);
        assert_ne!(rec3.skill_ref, rec.skill_ref);
        // get + list + remove.
        let got = db.get("alice", "triage").unwrap().unwrap();
        assert_eq!(got.skill_ref, rec3.skill_ref);
        let (rows, has_more) = db.list("alice", 100, None).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(!has_more);
        assert!(db.remove("alice", "triage").unwrap());
        assert!(!db.remove("alice", "triage").unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stored_form_requires_the_ref_and_pack_form_refuses_it() {
        let dir = tmp_dir();
        let db = SkillsDb::open(&dir).unwrap();
        // STORED form without a ref ⇒ InvalidArgument.
        let err = db.add("alice", &pack_manifest("x"), None).unwrap_err();
        assert!(matches!(err, CoreError::InvalidArgument(_)));
        // PACK form carrying a ref while a body is supplied ⇒ ambiguous ⇒ refused.
        let with_ref = format!(
            r#"{{"schema":"kortecx.skill/v1","name":"x","instructions_ref":"{}"}}"#,
            "a".repeat(64)
        );
        let err = db
            .add("alice", with_ref.as_bytes(), Some(added(b"body")))
            .unwrap_err();
        assert!(matches!(err, CoreError::InvalidArgument(_)));
        // STORED form with a ref works without a body.
        let (rec, _) = db.add("alice", with_ref.as_bytes(), None).unwrap();
        assert_eq!(rec.instructions_ref, "a".repeat(64));
        assert!(rec.instructions_preview.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn authority_deny_keys_are_refused() {
        let dir = tmp_dir();
        let db = SkillsDb::open(&dir).unwrap();
        let bad = br#"{"schema":"kortecx.skill/v1","name":"x","warrant":{"tool_grants":[]}}"#;
        let err = db.add("alice", bad, Some(added(b"body"))).unwrap_err();
        assert!(matches!(err, CoreError::InvalidArgument(_)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cross_party_isolation_is_uniform_not_found() {
        let dir = tmp_dir();
        let db = SkillsDb::open(&dir).unwrap();
        db.add("alice", &pack_manifest("private"), Some(added(b"b")))
            .unwrap();
        assert!(db.get("bob", "private").unwrap().is_none());
        let (rows, _) = db.list("bob", 100, None).unwrap();
        assert!(rows.is_empty());
        assert!(!db.remove("bob", "private").unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn schema_drift_rebuilds_empty() {
        let dir = tmp_dir();
        {
            let db = SkillsDb::open(&dir).unwrap();
            db.add("alice", &pack_manifest("x"), Some(added(b"b")))
                .unwrap();
        }
        {
            let conn = Connection::open(dir.join("skills.db")).unwrap();
            conn.execute(
                "UPDATE meta SET value = 999 WHERE key = 'schema_version'",
                [],
            )
            .unwrap();
        }
        let db = SkillsDb::open(&dir).unwrap();
        let (rows, _) = db.list("alice", 100, None).unwrap();
        assert!(rows.is_empty(), "schema drift must rebuild empty");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_seam_caps_mirror_the_kx_skill_parse_caps() {
        // The two constants live on opposite sides of the no-kx-skill-in-
        // gateway-core wall; this pins their equality so the handler's
        // fail-closed cap can never silently diverge from the parser's.
        assert_eq!(
            kx_gateway_core::MAX_SKILL_MANIFEST_BYTES,
            kx_skill::MAX_SKILL_MANIFEST_BYTES
        );
        assert_eq!(
            kx_gateway_core::MAX_SKILL_INSTRUCTIONS_BODY_BYTES,
            kx_skill::MAX_SKILL_INSTRUCTIONS_BYTES
        );
    }
}
