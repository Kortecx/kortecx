// SPDX-License-Identifier: Apache-2.0
//! [`SqliteToolRegistry`] — a durable [`ToolRegistry`] (PR-6a). Registrations
//! survive a restart in an off-journal `tools.db`. Mirrors the G1a / D94
//! durable-backend-behind-an-unchanged-trait shape (`kx_fleet::SqliteMembershipLedger`,
//! `kx_catalog::SqliteGrantLedger`): a SQLite `tools` table is the truth; an
//! in-memory [`InMemoryToolRegistry`] is rebuilt from it on every open + after
//! every write, so the READ half (`lookup`/`resolve`) delegates to the SAME fold
//! and **can never diverge** — and `resolve` therefore produces a byte-identical
//! `resolved_def_hash` / `ToolResolutionEvent` regardless of backend.
//!
//! ## One object, two views (the coordinator + the gateway)
//!
//! The live `kx serve` holds a single `Arc<SqliteToolRegistry>`: the coordinator
//! upcasts it to `Arc<dyn ToolRegistry>` and only ever calls the READ half
//! (`lookup`/`resolve`); the gateway keeps the concrete `Arc` and calls the
//! inherent `&self` WRITE methods ([`register_durable`](SqliteToolRegistry::register_durable),
//! [`deregister`](SqliteToolRegistry::deregister)) for the RegisterTool /
//! DeregisterTool RPCs. Same store, read-only to the coordinator, write-capable
//! to the gateway — so a freshly registered tool is immediately resolvable
//! (register-then-resolvable by construction).
//!
//! ## Off-journal, off-digest (SN-8 / GR8)
//!
//! `tools.db` is entirely off the journal and off the projection digest. The
//! server-derived `tool_id` is [`registration_token_of`]`(def, provenance)[..16]`
//! — content-addressed over the registration bytes, so the client never names or
//! forges it, and deleting + re-registering the same def re-materializes the same
//! id. No `MoteId` / journal entry / checkpoint is touched ⇒ the canonical digest
//! `7d22d4bd` is invariant by construction.

use std::path::Path;
use std::sync::{Mutex, RwLock};

use kx_mote::{canonical_config, ToolName, ToolVersion};
use kx_warrant::{ToolGrant, WarrantSpec};
use rusqlite::{params, Connection};

use crate::errors::{RegistrationError, ResolutionError};
use crate::ids::{RegistrationToken, ReviewerId};
use crate::provenance::{RegistrationStatus, ToolProvenance};
use crate::registry::{builtin_registrations, InMemoryToolRegistry, ToolRegistry};
use crate::sqlite_util::{open_db, open_db_in_memory};
use crate::token::registration_token_of;
use crate::tool_def::{ResolvedTool, ToolDef};

/// The durable tool-registry schema version. A mismatch is a **loud refusal**
/// (the operator migrates deliberately), never a silent rebuild — `tools.db` is
/// authoritative for its own registrations.
pub const TOOL_REGISTRY_SCHEMA_VERSION: u16 = 1;

const DDL: &str = "CREATE TABLE IF NOT EXISTS tools (
    tool_id        BLOB PRIMARY KEY,
    tool_name      TEXT NOT NULL,
    tool_version   TEXT NOT NULL,
    is_builtin     INTEGER NOT NULL,
    server_host    TEXT,
    def_canonical  BLOB NOT NULL,
    prov_canonical BLOB NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_tools_name_ver ON tools (tool_name, tool_version);";

/// One row of the durable registry, decoded for the governance / inventory view
/// (`DiscoverTools`). Carries the full [`ToolDef`] + [`ToolProvenance`] so the
/// caller can project whatever wire shape it needs without a second lookup.
#[derive(Debug, Clone)]
pub struct RegisteredEntry {
    /// The 16-byte server-derived id (`registration_token_of(def, provenance)[..16]`).
    pub tool_id: [u8; 16],
    /// The tool's full definition.
    pub def: ToolDef,
    /// Who/what authored the registration.
    pub provenance: ToolProvenance,
    /// `true` for server-built tools (re-seeded on open; NOT deregisterable).
    pub is_builtin: bool,
    /// The vetted egress endpoint the PR-6b MCP gateway will dial (`None` = a
    /// no-egress tool). Captured + SSRF-vetted at registration; not dialed in 6a.
    pub server_host: Option<String>,
}

impl RegisteredEntry {
    /// The registration lifecycle status implied by the provenance (HumanAuthored
    /// ⇒ Approved; SelfGenerated ⇒ PendingHumanReview), mirroring the in-memory
    /// registry's routing.
    #[must_use]
    pub fn status(&self) -> RegistrationStatus {
        match self.provenance {
            ToolProvenance::HumanAuthored { .. } => RegistrationStatus::Approved,
            ToolProvenance::SelfGenerated { .. } => RegistrationStatus::PendingHumanReview,
        }
    }
}

/// The 16-byte server-derived `tool_id` for a registration token (the first half
/// of the content-addressed `RegistrationToken`). SN-8: derived runtime-side from
/// the registration bytes; the client cannot name or forge it.
#[must_use]
pub fn tool_id_of(token: &RegistrationToken) -> [u8; 16] {
    let full = token.0.as_bytes();
    let mut id = [0u8; 16];
    id.copy_from_slice(&full[..16]);
    id
}

fn store_err<E: std::fmt::Display>(err: &E) -> RegistrationError {
    RegistrationError::Storage(err.to_string())
}

fn encode_def(def: &ToolDef) -> Result<Vec<u8>, RegistrationError> {
    bincode::serde::encode_to_vec(def, canonical_config())
        .map_err(|e| RegistrationError::Storage(format!("encode ToolDef: {e}")))
}

fn encode_prov(prov: &ToolProvenance) -> Result<Vec<u8>, RegistrationError> {
    bincode::serde::encode_to_vec(prov, canonical_config())
        .map_err(|e| RegistrationError::Storage(format!("encode ToolProvenance: {e}")))
}

fn decode_def(bytes: &[u8]) -> Result<ToolDef, RegistrationError> {
    bincode::serde::decode_from_slice(bytes, canonical_config())
        .map(|(d, _)| d)
        .map_err(|e| RegistrationError::Storage(format!("decode ToolDef: {e}")))
}

fn decode_prov(bytes: &[u8]) -> Result<ToolProvenance, RegistrationError> {
    bincode::serde::decode_from_slice(bytes, canonical_config())
        .map(|(p, _)| p)
        .map_err(|e| RegistrationError::Storage(format!("decode ToolProvenance: {e}")))
}

/// A durable, SQLite-backed [`ToolRegistry`].
pub struct SqliteToolRegistry {
    conn: Mutex<Connection>,
    inner: RwLock<InMemoryToolRegistry>,
}

impl SqliteToolRegistry {
    /// Open (creating if absent) a durable tool registry at `path`. The OSS
    /// built-in tool set is idempotently re-seeded on every open, so a fresh or
    /// re-created `tools.db` always carries the built-ins; operator registrations
    /// accrete on top and survive a restart.
    ///
    /// # Errors
    /// [`RegistrationError::Storage`] on a SQLite / schema-mismatch / corrupt-row
    /// failure.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, RegistrationError> {
        let conn = open_db(path, TOOL_REGISTRY_SCHEMA_VERSION, DDL).map_err(|e| store_err(&e))?;
        Self::from_conn(conn)
    }

    /// Open an ephemeral in-memory durable tool registry (tests + the
    /// backend-agnostic conformance harness).
    ///
    /// # Errors
    /// [`RegistrationError::Storage`] on a SQLite failure.
    pub fn open_in_memory() -> Result<Self, RegistrationError> {
        let conn =
            open_db_in_memory(TOOL_REGISTRY_SCHEMA_VERSION, DDL).map_err(|e| store_err(&e))?;
        Self::from_conn(conn)
    }

    fn from_conn(conn: Connection) -> Result<Self, RegistrationError> {
        seed_builtins(&conn)?;
        let inner = rebuild_inner(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
            inner: RwLock::new(inner),
        })
    }

    /// Durably register an operator-authored tool (always `HumanAuthored` ⇒
    /// `Approved`; SN-8: the OSS RPC path never lets a client self-assert
    /// `SelfGenerated` to launder lineage). `server_host` is the vetted egress
    /// endpoint captured for the PR-6b MCP gateway (`None` for a no-egress tool).
    /// Re-registering the same `(name, version)` replaces the row (and its
    /// `tool_id` re-derives from the new def bytes).
    ///
    /// Returns the deterministic [`RegistrationToken`] (its first 16 bytes are the
    /// wire `tool_id`, [`tool_id_of`]).
    ///
    /// # Errors
    /// [`RegistrationError::Storage`] on a durable-write failure (fail-closed: a
    /// write that cannot durably commit is surfaced, never silently dropped).
    pub fn register_durable(
        &self,
        def: ToolDef,
        provenance: ToolProvenance,
        server_host: Option<String>,
    ) -> Result<RegistrationToken, RegistrationError> {
        self.register_inner(def, provenance, false, server_host)
    }

    /// Durably register a SERVER-BUILT tool (the bundled `mcp-echo@1`, `fs-list@1`,
    /// …) as a built-in: re-seeded conceptually like the OSS built-ins and **NOT
    /// deregisterable**. Used by the serve path so `DiscoverTools` shows the real
    /// runnable set alongside the OSS built-ins. Idempotent (re-register replaces).
    ///
    /// # Errors
    /// [`RegistrationError::Storage`] on a durable-write failure.
    pub fn register_server_tool(
        &self,
        def: ToolDef,
        provenance: ToolProvenance,
        server_host: Option<String>,
    ) -> Result<RegistrationToken, RegistrationError> {
        self.register_inner(def, provenance, true, server_host)
    }

    fn register_inner(
        &self,
        def: ToolDef,
        provenance: ToolProvenance,
        is_builtin: bool,
        server_host: Option<String>,
    ) -> Result<RegistrationToken, RegistrationError> {
        let token = registration_token_of(&def, &provenance);
        let conn = self.conn.lock().expect("poisoned mutex");
        insert_row(
            &conn,
            &token,
            &def,
            &provenance,
            is_builtin,
            server_host.as_deref(),
        )?;
        let rebuilt = rebuild_inner(&conn)?;
        *self.inner.write().expect("poisoned lock") = rebuilt;
        Ok(token)
    }

    /// Durably remove an operator-registered tool by exact `(name, version)`.
    /// **Built-ins are refused** (a server-built tool cannot be deregistered).
    /// Returns `true` iff a row was removed; `false` if the tool was absent or a
    /// built-in.
    ///
    /// # Errors
    /// [`RegistrationError::Storage`] on a durable-write failure.
    pub fn deregister(
        &self,
        tool_name: &ToolName,
        tool_version: &ToolVersion,
    ) -> Result<bool, RegistrationError> {
        let conn = self.conn.lock().expect("poisoned mutex");
        let removed = conn
            .execute(
                "DELETE FROM tools WHERE tool_name = ?1 AND tool_version = ?2 AND is_builtin = 0",
                params![tool_name.0, tool_version.0],
            )
            .map_err(|e| store_err(&e))?;
        if removed > 0 {
            let rebuilt = rebuild_inner(&conn)?;
            *self.inner.write().expect("poisoned lock") = rebuilt;
        }
        Ok(removed > 0)
    }

    /// The durable-registry inventory / governance view (the `DiscoverTools` RPC
    /// source) — every registered tool in deterministic `(name, version)` order,
    /// paginated by an exclusive `(after_name, after_version)` cursor. Distinct
    /// from the advisory toolscout index: this is "what is registered, by whom,
    /// with what authority", not a ranking surface.
    ///
    /// # Errors
    /// [`RegistrationError::Storage`] on a read / decode failure.
    pub fn discover(
        &self,
        limit: usize,
        after: Option<(&str, &str)>,
    ) -> Result<Vec<RegisteredEntry>, RegistrationError> {
        let conn = self.conn.lock().expect("poisoned mutex");
        let (sql, after_name, after_version) = match after {
            Some((n, v)) => (
                "SELECT tool_id, is_builtin, server_host, def_canonical, prov_canonical \
                 FROM tools WHERE (tool_name, tool_version) > (?1, ?2) \
                 ORDER BY tool_name, tool_version LIMIT ?3",
                n.to_string(),
                v.to_string(),
            ),
            None => (
                "SELECT tool_id, is_builtin, server_host, def_canonical, prov_canonical \
                 FROM tools ORDER BY tool_name, tool_version LIMIT ?3",
                String::new(),
                String::new(),
            ),
        };
        let limit_i64 = i64::try_from(limit).unwrap_or(i64::MAX);
        let mut stmt = conn.prepare(sql).map_err(|e| store_err(&e))?;
        let rows = stmt
            .query_map(params![after_name, after_version, limit_i64], |r| {
                Ok((
                    r.get::<_, Vec<u8>>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Vec<u8>>(3)?,
                    r.get::<_, Vec<u8>>(4)?,
                ))
            })
            .map_err(|e| store_err(&e))?;

        let mut out = Vec::new();
        for row in rows {
            let (id_bytes, is_builtin, server_host, def_bytes, prov_bytes) =
                row.map_err(|e| store_err(&e))?;
            if id_bytes.len() != 16 {
                return Err(RegistrationError::Storage(
                    "tool_id is not 16 bytes".to_string(),
                ));
            }
            let mut tool_id = [0u8; 16];
            tool_id.copy_from_slice(&id_bytes);
            out.push(RegisteredEntry {
                tool_id,
                def: decode_def(&def_bytes)?,
                provenance: decode_prov(&prov_bytes)?,
                is_builtin: is_builtin != 0,
                server_host,
            });
        }
        Ok(out)
    }

    /// Every **`Approved`** tool definition, in deterministic `(tool_id,
    /// tool_version)` order — the same surface `InMemoryToolRegistry::defs`
    /// exposes, served from the durable fold. Used to build the toolscout advisory
    /// index + the planner/ReAct tool catalog from the durable registry.
    #[must_use]
    pub fn defs(&self) -> Vec<ToolDef> {
        self.inner.read().expect("poisoned lock").defs()
    }
}

fn seed_builtins(conn: &Connection) -> Result<(), RegistrationError> {
    for (def, provenance) in builtin_registrations() {
        let token = registration_token_of(&def, &provenance);
        // INSERT OR IGNORE so a re-open never disturbs an operator's accreted
        // rows; the built-ins are always present even after a manual row delete.
        let def_bytes = encode_def(&def)?;
        let prov_bytes = encode_prov(&provenance)?;
        conn.execute(
            "INSERT OR IGNORE INTO tools \
             (tool_id, tool_name, tool_version, is_builtin, server_host, def_canonical, prov_canonical) \
             VALUES (?1, ?2, ?3, 1, NULL, ?4, ?5)",
            params![
                &tool_id_of(&token)[..],
                def.tool_id.0,
                def.tool_version.0,
                &def_bytes[..],
                &prov_bytes[..],
            ],
        )
        .map_err(|e| store_err(&e))?;
    }
    Ok(())
}

fn insert_row(
    conn: &Connection,
    token: &RegistrationToken,
    def: &ToolDef,
    provenance: &ToolProvenance,
    is_builtin: bool,
    server_host: Option<&str>,
) -> Result<(), RegistrationError> {
    let def_bytes = encode_def(def)?;
    let prov_bytes = encode_prov(provenance)?;
    conn.execute(
        "INSERT OR REPLACE INTO tools \
         (tool_id, tool_name, tool_version, is_builtin, server_host, def_canonical, prov_canonical) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            &tool_id_of(token)[..],
            def.tool_id.0,
            def.tool_version.0,
            i64::from(is_builtin),
            server_host,
            &def_bytes[..],
            &prov_bytes[..],
        ],
    )
    .map_err(|e| store_err(&e))?;
    Ok(())
}

/// Rebuild a fresh [`InMemoryToolRegistry`] from the durable rows via the SAME
/// `register` fold, so `lookup`/`resolve` behave byte-identically to the
/// in-memory backend (and `resolved_def_hash` is identical).
fn rebuild_inner(conn: &Connection) -> Result<InMemoryToolRegistry, RegistrationError> {
    let mut reg = InMemoryToolRegistry::new();
    let mut stmt = conn
        .prepare("SELECT def_canonical, prov_canonical FROM tools ORDER BY tool_name, tool_version")
        .map_err(|e| store_err(&e))?;
    let rows = stmt
        .query_map([], |r| {
            Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, Vec<u8>>(1)?))
        })
        .map_err(|e| store_err(&e))?;
    for row in rows {
        let (def_bytes, prov_bytes) = row.map_err(|e| store_err(&e))?;
        let def = decode_def(&def_bytes)?;
        let prov = decode_prov(&prov_bytes)?;
        let _ = reg.register(def, prov);
    }
    Ok(reg)
}

impl ToolRegistry for SqliteToolRegistry {
    fn lookup(&self, tool_id: &ToolName, tool_version: &ToolVersion) -> Option<ToolDef> {
        self.inner
            .read()
            .expect("poisoned lock")
            .lookup(tool_id, tool_version)
    }

    fn resolve(
        &self,
        grant: &ToolGrant,
        warrant: &WarrantSpec,
    ) -> Result<ResolvedTool, ResolutionError> {
        self.inner
            .read()
            .expect("poisoned lock")
            .resolve(grant, warrant)
    }

    fn register(
        &mut self,
        def: ToolDef,
        provenance: ToolProvenance,
    ) -> Result<RegistrationToken, RegistrationError> {
        // Durable write through the inherent &self path (interior mutability).
        self.register_durable(def, provenance, None)
    }

    fn approve_registration(
        &mut self,
        token: RegistrationToken,
        _approver: ReviewerId,
    ) -> Result<(), RegistrationError> {
        // The OSS PR-6a durable registry registers only HumanAuthored (Approved)
        // tools — there is no PendingHumanReview row to approve. SelfGenerated
        // durable approval is a future, separately-D-numbered effort.
        Err(RegistrationError::UnknownToken { token })
    }
}

const _: fn() = || {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<SqliteToolRegistry>();
};

#[cfg(test)]
mod tests {
    use super::*;
    use kx_content::ContentRef;
    use kx_warrant::{
        ExecutorClass, FsScope, ModelRoute, MoteClass, NetScope, ResourceCeiling, ToolRequirement,
        WarrantSpec,
    };
    use std::collections::BTreeSet;

    fn custom_def(name: &str) -> ToolDef {
        ToolDef {
            tool_id: ToolName(name.into()),
            tool_version: ToolVersion("1".into()),
            kind: crate::ToolKind::Builtin,
            required_capability: ToolRequirement {
                net_scope_required: NetScope::None,
                fs_scope_required: FsScope::empty(),
                syscall_profile_ref: ContentRef::from_bytes([0; 32]),
                min_resource_ceiling: ResourceCeiling {
                    cpu_milli: 0,
                    mem_bytes: 0,
                    wall_clock_ms: 0,
                    fd_count: 0,
                    disk_bytes: 0,
                },
            },
            description: "a custom tool".into(),
            idempotency_class: crate::IdempotencyClass::Readback,
            input_schema: None,
        }
    }

    fn warrant_granting(name: &str) -> WarrantSpec {
        let mut grants = BTreeSet::new();
        grants.insert(ToolGrant {
            tool_id: ToolName(name.into()),
            tool_version: ToolVersion("1".into()),
        });
        WarrantSpec {
            mote_class: MoteClass::Pure,
            nd_class: MoteClass::Pure,
            fs_scope: FsScope::empty(),
            net_scope: NetScope::None,
            syscall_profile_ref: ContentRef::from_bytes([0; 32]),
            tool_grants: grants,
            model_route: ModelRoute {
                model_id: kx_mote::ModelId("m".into()),
                max_input_tokens: 100,
                max_output_tokens: 100,
                max_calls: 1,
            },
            resource_ceiling: ResourceCeiling {
                cpu_milli: 100,
                mem_bytes: 1 << 20,
                wall_clock_ms: 1000,
                fd_count: 16,
                disk_bytes: 1 << 20,
            },
            environment_ref: None,
            executor_class: ExecutorClass::Bwrap,
            ..Default::default()
        }
    }

    #[test]
    fn open_seeds_the_builtins() {
        let reg = SqliteToolRegistry::open_in_memory().unwrap();
        let names: BTreeSet<String> = reg.defs().into_iter().map(|d| d.tool_id.0).collect();
        assert!(names.contains("fs-read"));
        assert!(names.contains("fs-write"));
        assert!(names.contains("text-summarize"));
        assert_eq!(names.len(), 3);
    }

    #[test]
    fn register_then_resolvable() {
        let reg = SqliteToolRegistry::open_in_memory().unwrap();
        let prov = ToolProvenance::HumanAuthored {
            author: "ops".into(),
        };
        reg.register_durable(custom_def("my-tool"), prov, None)
            .unwrap();
        assert!(reg
            .lookup(&ToolName("my-tool".into()), &ToolVersion("1".into()))
            .is_some());
        let grant = ToolGrant {
            tool_id: ToolName("my-tool".into()),
            tool_version: ToolVersion("1".into()),
        };
        assert!(reg.resolve(&grant, &warrant_granting("my-tool")).is_ok());
    }

    #[test]
    fn tool_id_deterministic_across_reregister() {
        let reg = SqliteToolRegistry::open_in_memory().unwrap();
        let prov = ToolProvenance::HumanAuthored {
            author: "ops".into(),
        };
        let t1 = reg
            .register_durable(custom_def("dup"), prov.clone(), None)
            .unwrap();
        // Deregister + re-register the identical def reproduces the same id.
        assert!(reg
            .deregister(&ToolName("dup".into()), &ToolVersion("1".into()))
            .unwrap());
        let t2 = reg.register_durable(custom_def("dup"), prov, None).unwrap();
        assert_eq!(tool_id_of(&t1), tool_id_of(&t2));
    }

    #[test]
    fn builtins_not_deregisterable() {
        let reg = SqliteToolRegistry::open_in_memory().unwrap();
        let removed = reg
            .deregister(&ToolName("fs-read".into()), &ToolVersion("1".into()))
            .unwrap();
        assert!(!removed, "built-in must be refused");
        assert!(reg
            .lookup(&ToolName("fs-read".into()), &ToolVersion("1".into()))
            .is_some());
    }

    #[test]
    fn registrations_persist_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tools.db");
        {
            let reg = SqliteToolRegistry::open(&path).unwrap();
            reg.register_durable(
                custom_def("persisted"),
                ToolProvenance::HumanAuthored {
                    author: "ops".into(),
                },
                Some("example.com".into()),
            )
            .unwrap();
        }
        let reopened = SqliteToolRegistry::open(&path).unwrap();
        assert!(reopened
            .lookup(&ToolName("persisted".into()), &ToolVersion("1".into()))
            .is_some());
        let entry = reopened
            .discover(64, None)
            .unwrap()
            .into_iter()
            .find(|e| e.def.tool_id.0 == "persisted")
            .unwrap();
        assert_eq!(entry.server_host.as_deref(), Some("example.com"));
        assert!(!entry.is_builtin);
    }

    #[test]
    fn resolve_byte_identical_to_in_memory() {
        // The durable resolve must produce the SAME resolved_def_hash as the
        // in-memory backend — the digest-invariance keystone.
        let durable = SqliteToolRegistry::open_in_memory().unwrap();
        let memory = InMemoryToolRegistry::with_builtins();
        let grant = ToolGrant {
            tool_id: ToolName("fs-read".into()),
            tool_version: ToolVersion("1".into()),
        };
        let w = warrant_granting("fs-read");
        let a = durable.resolve(&grant, &w).unwrap();
        let b = memory.resolve(&grant, &w).unwrap();
        assert_eq!(a.event.resolved_def_hash, b.event.resolved_def_hash);
        assert_eq!(a.event_ref, b.event_ref);
    }
}
