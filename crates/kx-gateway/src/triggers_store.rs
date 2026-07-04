//! D113 (trigger seam): the `triggers.db` off-journal sidecar.
//!
//! A **trigger** binds an inbound event source (webhook / cron / grpc) to a recipe
//! handle: when the event fires, the gateway starts a FRESH registered run via the
//! existing Invoke path (the coordinator stays the sole journal writer; the frozen
//! trio is untouched). This module is the durable store behind that seam:
//!
//! - **`triggers`** — the registered configs (source of truth; re-registerable).
//! - **`trigger_fires`** — the idempotency-key dedup + run-origin record: the
//!   "this event already started THIS run" fact. `INSERT OR IGNORE` on the key makes
//!   a replayed inbound event a no-op (returns the prior run, fires nothing).
//!
//! ## Off-journal / off-digest (the connections.db posture)
//! The OS keychain holds no run state here; everything is off the journal + the
//! canonical projection digest. On a corrupt file / schema-version drift the sidecar
//! recreates EMPTY — the only loss is the registered triggers (re-register to
//! restore) + the dedup window (a replayed event within the lost window could
//! re-fire). A FUTURE schema bump that must preserve registered triggers has to add
//! a forward-migration here (v1 has no prior version to migrate from).
//!
//! SN-8: `trigger_id` is server-derived (`blake3("kx-trigger\0" ‖ name)[..16]`), so
//! re-registering the same name is idempotent; the client never forges it. The auth
//! secret is referenced by NAME only (the value lives in the keychain, never here).

use std::path::Path;
use std::sync::Mutex;

use kx_content::ContentRef;
use rusqlite::{params, Connection};

use crate::error::GatewayError;

/// Bump on any table-shape change. Unknown/missing ⇒ recreate EMPTY (module doc).
/// v2 (T-APP-TRIGGER-TARGET): + `app_handle` (App target), `timezone` (5-field cron
/// tz), `require_approval` (per-trigger HITL). Off-journal ⇒ the bump rebuilds the
/// sidecar EMPTY (re-register after upgrade); the canonical projection digest is
/// untouched (no journal/checkpoint change).
const SCHEMA_VERSION: i64 = 2;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS triggers (
    trigger_id         BLOB PRIMARY KEY,   -- 16B server-derived id
    name               TEXT NOT NULL UNIQUE,
    kind               TEXT NOT NULL,      -- 'webhook' | 'cron' | 'grpc'
    recipe_handle      TEXT NOT NULL,      -- the kx/recipes/... handle to Invoke ('' ⇒ App target)
    app_handle         TEXT NOT NULL DEFAULT '',   -- the saved App handle to RunApp ('' ⇒ recipe target)
    args_template_json TEXT NOT NULL,      -- reserved (passthrough today); '' default
    auth               TEXT NOT NULL,      -- 'none' | 'hmac_sha256' | 'bearer'
    auth_secret_ref    TEXT NOT NULL,      -- SecretRef NAME ('' ⇒ none); never the value
    schedule_spec      TEXT NOT NULL,      -- cron: interval seconds OR a 5-field crontab expr; '' otherwise
    timezone           TEXT NOT NULL DEFAULT 'UTC', -- IANA zone for a 5-field cron expr ('' ⇒ UTC)
    owner_party        TEXT NOT NULL,      -- the registrant party the run fires under (D102.2, SN-8)
    require_approval   INTEGER NOT NULL DEFAULT 0,  -- per-trigger HITL: withhold irreversible actions (D114)
    enabled            INTEGER NOT NULL,
    next_fire_unix_ms  INTEGER NOT NULL,   -- cron watermark (0 ⇒ n/a)
    created_unix_ms    INTEGER NOT NULL,
    last_fire_unix_ms  INTEGER NOT NULL    -- 0 ⇒ never fired
);
CREATE TABLE IF NOT EXISTS trigger_fires (
    idempotency_key  TEXT PRIMARY KEY,     -- event-level dedup key
    trigger_id       BLOB NOT NULL,        -- 16B
    instance_id      BLOB NOT NULL,        -- 16B the run this event started
    received_unix_ms INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS trigger_fires_by_trigger ON trigger_fires(trigger_id);
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value INTEGER NOT NULL);
";

/// A registered trigger config, in host string vocabulary (the gateway-core seam +
/// the proto convert at the boundary). `kind`/`auth` are validated strings, not enums,
/// so the store stays decoupled from the proto (the `McpServerRegistration` posture).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TriggerRow {
    pub trigger_id: [u8; 16],
    pub name: String,
    pub kind: String,
    /// The `kx/recipes/…` handle to Invoke (`""` ⇒ this is an App target). Exactly one
    /// of `recipe_handle` / `app_handle` is non-empty (validated at register).
    pub recipe_handle: String,
    /// The saved App handle to RunApp (`""` ⇒ this is a recipe target).
    pub app_handle: String,
    pub args_template_json: String,
    pub auth: String,
    pub auth_secret_ref: String,
    /// Cron: legacy interval-seconds OR a 5-field crontab expr; empty otherwise.
    pub schedule_spec: String,
    /// IANA timezone for a 5-field cron expr (`""` ⇒ UTC); ignored for interval/other.
    pub timezone: String,
    /// The registrant party the fired run binds authority under (D102.2; server-derived).
    pub owner_party: String,
    /// Per-trigger HITL (D114): withhold irreversible actions until an operator grant.
    pub require_approval: bool,
    pub enabled: bool,
    pub next_fire_unix_ms: u64,
    pub created_unix_ms: u64,
    pub last_fire_unix_ms: u64,
}

/// Server-derived 16-byte trigger id from the operator name (idempotent re-register).
pub(crate) fn trigger_id_of(name: &str) -> [u8; 16] {
    let mut keyed = Vec::with_capacity(11 + name.len());
    keyed.extend_from_slice(b"kx-trigger\0");
    keyed.extend_from_slice(name.as_bytes());
    let mut id = [0u8; 16];
    id.copy_from_slice(&ContentRef::of(&keyed).0[..16]);
    id
}

/// The durable trigger sidecar over `triggers.db`. A single mutex'd connection:
/// registrations are interactive-rate; fires are bounded by the listener rate-limit.
pub(crate) struct TriggersDb {
    conn: Mutex<Connection>,
}

impl TriggersDb {
    /// Open (or create) `triggers.db` under `dir`. A corrupt file / schema drift
    /// recreates EMPTY (module doc).
    ///
    /// # Errors
    /// [`GatewayError::Catalog`] on an unrecoverable open/pragma failure.
    pub(crate) fn open(dir: &Path) -> Result<Self, GatewayError> {
        std::fs::create_dir_all(dir)
            .map_err(|e| GatewayError::Catalog(format!("triggers dir: {e}")))?;
        let db_path = dir.join("triggers.db");
        let conn = if let Ok(c) = Self::open_with_pragma(&db_path) {
            c
        } else {
            let _ = std::fs::remove_file(&db_path);
            let _ = std::fs::remove_file(dir.join("triggers.db-wal"));
            let _ = std::fs::remove_file(dir.join("triggers.db-shm"));
            Self::open_with_pragma(&db_path)
                .map_err(|e| GatewayError::Catalog(format!("triggers reopen: {e}")))?
        };
        let fresh_or_stale = match Self::read_schema_version(&conn) {
            Ok(Some(v)) => v != SCHEMA_VERSION,
            Ok(None) | Err(_) => true,
        };
        if fresh_or_stale {
            conn.execute_batch(
                "DROP TABLE IF EXISTS triggers;
                 DROP TABLE IF EXISTS trigger_fires;
                 DROP TABLE IF EXISTS meta;",
            )
            .map_err(|e| GatewayError::Catalog(format!("triggers rebuild: {e}")))?;
        }
        conn.execute_batch(SCHEMA)
            .map_err(|e| GatewayError::Catalog(format!("triggers schema: {e}")))?;
        conn.execute(
            "INSERT OR IGNORE INTO meta(key, value) VALUES ('schema_version', ?1)",
            params![SCHEMA_VERSION],
        )
        .map_err(|e| GatewayError::Catalog(format!("triggers meta init: {e}")))?;
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

    /// Register (or replace) a trigger by name. `created_unix_ms` is preserved across
    /// a re-register (the row's first-seen time); the rest of the config is updated.
    pub(crate) fn upsert(&self, row: &TriggerRow) -> Result<(), GatewayError> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO triggers(trigger_id, name, kind, recipe_handle, args_template_json, \
             auth, auth_secret_ref, schedule_spec, owner_party, enabled, next_fire_unix_ms, \
             created_unix_ms, last_fire_unix_ms, app_handle, timezone, require_approval) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16) \
             ON CONFLICT(name) DO UPDATE SET kind=?3, recipe_handle=?4, args_template_json=?5, \
             auth=?6, auth_secret_ref=?7, schedule_spec=?8, owner_party=?9, enabled=?10, \
             next_fire_unix_ms=?11, app_handle=?14, timezone=?15, require_approval=?16",
            params![
                row.trigger_id.to_vec(),
                row.name,
                row.kind,
                row.recipe_handle,
                row.args_template_json,
                row.auth,
                row.auth_secret_ref,
                row.schedule_spec,
                row.owner_party,
                i64::from(row.enabled),
                ms(row.next_fire_unix_ms),
                ms(row.created_unix_ms),
                ms(row.last_fire_unix_ms),
                row.app_handle,
                row.timezone,
                i64::from(row.require_approval),
            ],
        )
        .map_err(|e| GatewayError::Catalog(format!("triggers upsert: {e}")))?;
        Ok(())
    }

    /// Fetch a trigger by name.
    pub(crate) fn get(&self, name: &str) -> Result<Option<TriggerRow>, GatewayError> {
        let conn = self.lock()?;
        conn.query_row(
            "SELECT trigger_id, name, kind, recipe_handle, args_template_json, auth, \
             auth_secret_ref, schedule_spec, owner_party, enabled, next_fire_unix_ms, created_unix_ms, \
             last_fire_unix_ms, app_handle, timezone, require_approval FROM triggers WHERE name = ?1",
            params![name],
            row_from,
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(GatewayError::Catalog(format!("triggers get: {other}"))),
        })
    }

    /// List triggers keyset-paged after `after_name` (exclusive), up to `limit`.
    /// Returns `(rows, has_more)`.
    pub(crate) fn list(
        &self,
        limit: u32,
        after_name: &str,
    ) -> Result<(Vec<TriggerRow>, bool), GatewayError> {
        let lim = if limit == 0 { 200 } else { limit.min(1000) };
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT trigger_id, name, kind, recipe_handle, args_template_json, auth, \
                 auth_secret_ref, schedule_spec, owner_party, enabled, next_fire_unix_ms, created_unix_ms, \
                 last_fire_unix_ms, app_handle, timezone, require_approval FROM triggers WHERE name > ?1 ORDER BY name ASC LIMIT ?2",
            )
            .map_err(|e| GatewayError::Catalog(format!("triggers list prepare: {e}")))?;
        let mut rows = stmt
            .query_map(params![after_name, i64::from(lim) + 1], row_from)
            .map_err(|e| GatewayError::Catalog(format!("triggers list: {e}")))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| GatewayError::Catalog(format!("triggers list row: {e}")))?;
        let has_more = rows.len() > lim as usize;
        rows.truncate(lim as usize);
        Ok((rows, has_more))
    }

    /// Remove a trigger by name. Returns `true` iff a row was removed. The
    /// `trigger_fires` history is retained (audit) — orphaned rows are inert.
    pub(crate) fn remove(&self, name: &str) -> Result<bool, GatewayError> {
        let conn = self.lock()?;
        let n = conn
            .execute("DELETE FROM triggers WHERE name = ?1", params![name])
            .map_err(|e| GatewayError::Catalog(format!("triggers remove: {e}")))?;
        Ok(n > 0)
    }

    /// Pre-check dedup: the `instance_id` already recorded for `idempotency_key`, or
    /// `None` if the key is fresh. The common replay path returns the prior run WITHOUT
    /// registering a new one.
    pub(crate) fn fired(&self, idempotency_key: &str) -> Result<Option<[u8; 16]>, GatewayError> {
        let conn = self.lock()?;
        conn.query_row(
            "SELECT instance_id FROM trigger_fires WHERE idempotency_key = ?1",
            params![idempotency_key],
            |r| r.get::<_, Vec<u8>>(0),
        )
        .map(|blob| {
            let mut id = [0u8; 16];
            id.copy_from_slice(&blob.get(..16).unwrap_or(&[0u8; 16])[..16]);
            Some(id)
        })
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(GatewayError::Catalog(format!(
                "trigger_fires fired: {other}"
            ))),
        })
    }

    /// Record an event fire under `idempotency_key`. Returns the `instance_id` that
    /// IS recorded for the key — either the freshly-supplied `instance_id` (this was
    /// the first time the key was seen, `deduped = false`) or the PRIOR run's id (the
    /// key was already recorded, `deduped = true`). The caller fires a run only when
    /// `deduped == false`.
    pub(crate) fn record_fire(
        &self,
        idempotency_key: &str,
        trigger_id: &[u8; 16],
        instance_id: &[u8; 16],
        received_unix_ms: u64,
    ) -> Result<([u8; 16], bool), GatewayError> {
        let conn = self.lock()?;
        let inserted = conn
            .execute(
                "INSERT OR IGNORE INTO trigger_fires(idempotency_key, trigger_id, instance_id, \
                 received_unix_ms) VALUES (?1, ?2, ?3, ?4)",
                params![
                    idempotency_key,
                    trigger_id.to_vec(),
                    instance_id.to_vec(),
                    ms(received_unix_ms)
                ],
            )
            .map_err(|e| GatewayError::Catalog(format!("trigger_fires record: {e}")))?;
        if inserted > 0 {
            return Ok((*instance_id, false));
        }
        // Duplicate key ⇒ return the run already recorded for it.
        let prior: Vec<u8> = conn
            .query_row(
                "SELECT instance_id FROM trigger_fires WHERE idempotency_key = ?1",
                params![idempotency_key],
                |r| r.get(0),
            )
            .map_err(|e| GatewayError::Catalog(format!("trigger_fires prior: {e}")))?;
        let mut id = [0u8; 16];
        id.copy_from_slice(&prior.get(..16).unwrap_or(&[0u8; 16])[..16]);
        Ok((id, true))
    }

    /// Mark a trigger fired (advisory `last_fire_unix_ms`).
    pub(crate) fn set_last_fire(&self, name: &str, now_ms: u64) -> Result<(), GatewayError> {
        let conn = self.lock()?;
        conn.execute(
            "UPDATE triggers SET last_fire_unix_ms = ?2 WHERE name = ?1",
            params![name, ms(now_ms)],
        )
        .map_err(|e| GatewayError::Catalog(format!("triggers set_last_fire: {e}")))?;
        Ok(())
    }

    /// Advance a cron trigger's next-fire watermark.
    pub(crate) fn set_next_fire(&self, name: &str, next_ms: u64) -> Result<(), GatewayError> {
        let conn = self.lock()?;
        conn.execute(
            "UPDATE triggers SET next_fire_unix_ms = ?2 WHERE name = ?1",
            params![name, ms(next_ms)],
        )
        .map_err(|e| GatewayError::Catalog(format!("triggers set_next_fire: {e}")))?;
        Ok(())
    }

    /// Enabled `cron` triggers whose `next_fire_unix_ms` is at/under `now_ms` (due).
    pub(crate) fn due_cron(&self, now_ms: u64) -> Result<Vec<TriggerRow>, GatewayError> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT trigger_id, name, kind, recipe_handle, args_template_json, auth, \
                 auth_secret_ref, schedule_spec, owner_party, enabled, next_fire_unix_ms, created_unix_ms, \
                 last_fire_unix_ms, app_handle, timezone, require_approval FROM triggers \
                 WHERE kind = 'cron' AND enabled = 1 AND next_fire_unix_ms <= ?1 \
                 ORDER BY name ASC",
            )
            .map_err(|e| GatewayError::Catalog(format!("triggers due prepare: {e}")))?;
        let rows = stmt
            .query_map(params![ms(now_ms)], row_from)
            .map_err(|e| GatewayError::Catalog(format!("triggers due: {e}")))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| GatewayError::Catalog(format!("triggers due row: {e}")))?;
        Ok(rows)
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>, GatewayError> {
        self.conn
            .lock()
            .map_err(|_| GatewayError::Catalog("triggers lock poisoned".into()))
    }
}

/// Clamp a `u64` ms timestamp into sqlite's `i64` column (saturating).
fn ms(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

/// Map a `triggers` row to a [`TriggerRow`].
fn row_from(r: &rusqlite::Row<'_>) -> rusqlite::Result<TriggerRow> {
    let id_blob: Vec<u8> = r.get(0)?;
    let mut trigger_id = [0u8; 16];
    trigger_id.copy_from_slice(&id_blob.get(..16).unwrap_or(&[0u8; 16])[..16]);
    Ok(TriggerRow {
        trigger_id,
        name: r.get(1)?,
        kind: r.get(2)?,
        recipe_handle: r.get(3)?,
        args_template_json: r.get(4)?,
        auth: r.get(5)?,
        auth_secret_ref: r.get(6)?,
        schedule_spec: r.get(7)?,
        owner_party: r.get(8)?,
        enabled: r.get::<_, i64>(9)? != 0,
        next_fire_unix_ms: u64::try_from(r.get::<_, i64>(10)?).unwrap_or(0),
        created_unix_ms: u64::try_from(r.get::<_, i64>(11)?).unwrap_or(0),
        last_fire_unix_ms: u64::try_from(r.get::<_, i64>(12)?).unwrap_or(0),
        app_handle: r.get(13)?,
        timezone: r.get(14)?,
        require_approval: r.get::<_, i64>(15)? != 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str, kind: &str) -> TriggerRow {
        TriggerRow {
            trigger_id: trigger_id_of(name),
            name: name.to_string(),
            kind: kind.to_string(),
            recipe_handle: "kx/recipes/chat".to_string(),
            app_handle: String::new(),
            args_template_json: String::new(),
            auth: "hmac_sha256".to_string(),
            auth_secret_ref: "WEBHOOK_SECRET".to_string(),
            schedule_spec: if kind == "cron" {
                "60".to_string()
            } else {
                String::new()
            },
            timezone: "UTC".to_string(),
            owner_party: "local-dev".to_string(),
            require_approval: false,
            enabled: true,
            next_fire_unix_ms: if kind == "cron" { 1_000 } else { 0 },
            created_unix_ms: 500,
            last_fire_unix_ms: 0,
        }
    }

    #[test]
    fn app_target_and_cron_tz_and_hitl_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let db = TriggersDb::open(dir.path()).unwrap();
        // An App-target cron trigger with a 5-field expr, a timezone, and HITL on.
        let mut r = row("nightly", "cron");
        r.recipe_handle = String::new();
        r.app_handle = "support-triage".to_string();
        r.schedule_spec = "0 9 * * 1-5".to_string();
        r.timezone = "America/New_York".to_string();
        r.require_approval = true;
        db.upsert(&r).unwrap();
        let got = db.get("nightly").unwrap().unwrap();
        assert_eq!(got, r);
        assert_eq!(got.app_handle, "support-triage");
        assert_eq!(got.timezone, "America/New_York");
        assert!(got.require_approval);
        assert!(got.recipe_handle.is_empty());
    }

    #[test]
    fn upsert_get_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let db = TriggersDb::open(dir.path()).unwrap();
        assert!(db.get("hook").unwrap().is_none());
        let r = row("hook", "webhook");
        db.upsert(&r).unwrap();
        assert_eq!(db.get("hook").unwrap().unwrap(), r);
    }

    #[test]
    fn idempotency_key_replay_is_deduped() {
        let dir = tempfile::tempdir().unwrap();
        let db = TriggersDb::open(dir.path()).unwrap();
        let tid = trigger_id_of("hook");
        let run_a = [0xAA; 16];
        let (id1, dup1) = db.record_fire("evt-1", &tid, &run_a, 10).unwrap();
        assert_eq!(id1, run_a);
        assert!(!dup1, "first sight of the key is NOT a duplicate");
        // A replay with a DIFFERENT run id must still resolve to the FIRST run + dedup.
        let run_b = [0xBB; 16];
        let (id2, dup2) = db.record_fire("evt-1", &tid, &run_b, 11).unwrap();
        assert_eq!(id2, run_a, "the key keeps its FIRST run id");
        assert!(dup2, "the replay is deduped (fires no second run)");
    }

    #[test]
    fn list_pages_and_remove() {
        let dir = tempfile::tempdir().unwrap();
        let db = TriggersDb::open(dir.path()).unwrap();
        for n in ["a", "b", "c"] {
            db.upsert(&row(n, "webhook")).unwrap();
        }
        let (p1, more) = db.list(2, "").unwrap();
        assert_eq!(
            p1.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
            ["a", "b"]
        );
        assert!(more);
        assert!(db.remove("b").unwrap());
        assert!(!db.remove("b").unwrap(), "second remove is a no-op");
        let (all, _) = db.list(0, "").unwrap();
        assert_eq!(
            all.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
            ["a", "c"]
        );
    }

    #[test]
    fn due_cron_selects_only_enabled_due_cron() {
        let dir = tempfile::tempdir().unwrap();
        let db = TriggersDb::open(dir.path()).unwrap();
        db.upsert(&row("c-due", "cron")).unwrap(); // next_fire=1000
        let mut future = row("c-future", "cron");
        future.next_fire_unix_ms = 9_999_999;
        db.upsert(&future).unwrap();
        db.upsert(&row("w-hook", "webhook")).unwrap(); // not cron
        let due = db.due_cron(5_000).unwrap();
        assert_eq!(
            due.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
            ["c-due"]
        );
    }

    #[test]
    fn reopen_preserves_then_corrupt_recreates_empty() {
        let dir = tempfile::tempdir().unwrap();
        {
            let db = TriggersDb::open(dir.path()).unwrap();
            db.upsert(&row("hook", "webhook")).unwrap();
        }
        // survives a restart
        {
            let db = TriggersDb::open(dir.path()).unwrap();
            assert!(db.get("hook").unwrap().is_some());
        }
        // corrupt file recreates empty
        std::fs::write(dir.path().join("triggers.db"), b"not sqlite").unwrap();
        let db = TriggersDb::open(dir.path()).unwrap();
        assert!(db.get("hook").unwrap().is_none());
        db.upsert(&row("hook", "webhook")).unwrap();
        assert!(db.get("hook").unwrap().is_some());
    }
}
