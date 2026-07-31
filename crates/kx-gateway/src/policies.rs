//! The Policy/Role sidecar: `policies.db` under `--catalog-dir`, backing the
//! [`PolicyAdmin`] seam — `PutPolicyRole` / `ListPolicyRoles` /
//! `DeletePolicyRole` / `AssignPolicyRole`.
//!
//! ## A role NARROWS; it never grants
//!
//! The allowlist a role names is intersected into the caller's effective tool
//! authority, so assigning a role can only ever take capability away. Naming a
//! tool the party could not fire anyway simply drops out of the intersection.
//!
//! This is what makes the registry safe to expose at all. Under the obvious
//! alternative — "a role GRANTS the tools it names" — anyone who could write a
//! role could write themselves a capability, and the store would become an
//! authority-minting surface sitting off the journal. Under intersection the
//! worst a malicious role can do is refuse work.
//!
//! ## Authored work — preserved across an upgrade, never wiped by one
//!
//! A role is something an operator wrote and cannot regenerate from anything the
//! runtime still has, so this store opens `UserAuthored`: a schema bump renames
//! it aside and re-imports by column intersection, and a DOWNGRADE is refused. A
//! corrupt/foreign FILE still recreates empty.
//!
//! The `apps.db` precedent is why the additive-column path is installed from day
//! one: `apps.db` froze its version at 1 with a comment saying a bump "would drop
//! saved apps", which routed around the destructive open rather than fixing it
//! and froze the schema as collateral. Every future column here goes through
//! [`PoliciesDb::ensure_column`] and never bumps [`SCHEMA_VERSION`].
//!
//! ## Caller-scoped to AUTHOR, party-scoped to ENFORCE
//!
//! Both tables are keyed by `(principal, …)`, so authoring gets the usual
//! uniform not-found for absent OR not-owned and no cross-party existence
//! oracle.
//!
//! [`PolicyAdmin::allowlist_for`] is the deliberate exception: it looks up by
//! PARTY alone. At fire time the runtime knows which party a warrant resolves
//! for and nothing about who authored the role that constrains it, so keying
//! enforcement by principal too would mean a role only ever narrowed its own
//! author — a preference, not a policy mechanism. Single-node OSS has one
//! operator so the asymmetry is invisible; multi-tenant "whose policy binds this
//! party" is a CLOUD question (D129) and is not answered here.
//!
//! ## Off the truth path
//!
//! Never journaled, never a `MoteId` input, never a digest input.

use std::path::Path;
use std::sync::Mutex;

use kx_gateway_core::{PolicyAdmin, PolicyAdminError, PolicyRoleRow, PolicyRoleToolWire};
use rusqlite::{params, Connection};

use crate::error::GatewayError;

/// Bump on any table-shape change. A corrupt/foreign FILE recreates empty; a
/// version BUMP renames the store aside and re-imports what still fits, and a
/// DOWNGRADE is refused — this store holds authored work (`crate::sidecar`).
const SCHEMA_VERSION: i64 = 1;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS policy_roles (
    principal       TEXT NOT NULL,   -- server-resolved caller party (scope)
    name            TEXT NOT NULL,   -- catalog key within principal
    description     TEXT NOT NULL,   -- advisory, never parsed for enforcement
    tools_json      TEXT NOT NULL,   -- JSON [[tool_id, tool_version], ...]; [] means 'narrows to nothing'
    created_unix_ms INTEGER NOT NULL,
    updated_unix_ms INTEGER NOT NULL,
    PRIMARY KEY (principal, name)
);
CREATE TABLE IF NOT EXISTS policy_assignments (
    principal   TEXT NOT NULL,       -- server-resolved caller party (scope)
    party       TEXT NOT NULL,       -- the PartyId the role applies to
    role_name   TEXT NOT NULL,       -- FK-by-convention into policy_roles.name
    PRIMARY KEY (principal, party)
);
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value INTEGER NOT NULL);
";

/// The durable Policy/Role registry over `policies.db`. A single mutex'd
/// connection: role authoring is interactive-rate, never contended.
pub(crate) struct PoliciesDb {
    conn: Mutex<Connection>,
}

impl PoliciesDb {
    /// Open (or create) `policies.db` under `dir`. A corrupt/foreign file
    /// recreates the store empty; a schema bump PRESERVES it — renamed aside and
    /// re-imported — and a downgrade is refused (module doc).
    ///
    /// # Errors
    /// [`GatewayError::Catalog`] on an unrecoverable open/pragma failure.
    pub(crate) fn open(dir: &Path) -> Result<Self, GatewayError> {
        let conn = crate::sidecar::open_sidecar(
            dir,
            "policies.db",
            SCHEMA_VERSION,
            SCHEMA,
            &["policy_roles", "policy_assignments", "meta"],
            crate::sidecar::Durability::UserAuthored,
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Idempotently add one additive column, given its `ALTER TABLE` declaration.
    ///
    /// Pre-installed from day one (the `apps.db` lesson): every future column
    /// addition goes through here and NEVER bumps [`SCHEMA_VERSION`]. Old
    /// binaries ignore an unknown column because every SELECT/INSERT names its
    /// columns explicitly.
    #[allow(dead_code)]
    fn ensure_column(
        conn: &Connection,
        table: &str,
        column: &str,
        decl: &str,
    ) -> rusqlite::Result<()> {
        let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
        let present = stmt
            .query_map([], |r| r.get::<_, String>(1))?
            .filter_map(Result::ok)
            .any(|col| col == column);
        drop(stmt);
        if !present {
            conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {decl};"))?;
        }
        Ok(())
    }
}

/// Encode a tool allowlist as the stored JSON.
fn tools_to_json(tools: &[PolicyRoleToolWire]) -> String {
    let pairs: Vec<[&str; 2]> = tools
        .iter()
        .map(|t| [t.tool_id.as_str(), t.tool_version.as_str()])
        .collect();
    serde_json::to_string(&pairs).unwrap_or_else(|_| "[]".to_string())
}

/// Decode a stored tool allowlist.
///
/// A row that will not parse yields an EMPTY allowlist, not a permissive one.
/// The two failure directions are not symmetric: an unreadable role that narrows
/// to nothing refuses work loudly, while an unreadable role that narrows to
/// everything silently restores authority the operator meant to remove.
fn tools_from_json(raw: &str) -> Vec<PolicyRoleToolWire> {
    serde_json::from_str::<Vec<[String; 2]>>(raw)
        .unwrap_or_default()
        .into_iter()
        .map(|[tool_id, tool_version]| PolicyRoleToolWire {
            tool_id,
            tool_version,
        })
        .collect()
}

fn storage(e: impl std::fmt::Display) -> PolicyAdminError {
    PolicyAdminError::Storage(e.to_string())
}

impl PolicyAdmin for PoliciesDb {
    fn put(&self, principal: &str, role: PolicyRoleRow) -> Result<bool, PolicyAdminError> {
        let conn = self.conn.lock().map_err(storage)?;
        // Preserve the ORIGINAL created_unix_ms on an update: an audit timestamp
        // that silently resets on every edit records the last edit, not the
        // creation, and cannot answer "how long has this narrowing been in force".
        let existing: Option<i64> = conn
            .query_row(
                "SELECT created_unix_ms FROM policy_roles WHERE principal = ?1 AND name = ?2",
                params![principal, &role.name],
                |r| r.get(0),
            )
            .ok();
        let created = existing.unwrap_or_else(|| i64::try_from(role.created_unix_ms).unwrap_or(0));
        conn.execute(
            "INSERT INTO policy_roles
                 (principal, name, description, tools_json, created_unix_ms, updated_unix_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(principal, name) DO UPDATE SET
                 description     = excluded.description,
                 tools_json      = excluded.tools_json,
                 updated_unix_ms = excluded.updated_unix_ms",
            params![
                principal,
                &role.name,
                &role.description,
                tools_to_json(&role.tools),
                created,
                i64::try_from(role.updated_unix_ms).unwrap_or(0),
            ],
        )
        .map_err(storage)?;
        Ok(existing.is_none())
    }

    fn list(&self, principal: &str, limit: usize) -> Result<Vec<PolicyRoleRow>, PolicyAdminError> {
        let conn = self.conn.lock().map_err(storage)?;
        let mut stmt = conn
            .prepare(
                "SELECT name, description, tools_json, created_unix_ms, updated_unix_ms
                 FROM policy_roles WHERE principal = ?1 ORDER BY name ASC LIMIT ?2",
            )
            .map_err(storage)?;
        let rows = stmt
            .query_map(
                params![principal, i64::try_from(limit).unwrap_or(100)],
                |r| {
                    Ok(PolicyRoleRow {
                        name: r.get(0)?,
                        description: r.get(1)?,
                        tools: tools_from_json(&r.get::<_, String>(2)?),
                        created_unix_ms: u64::try_from(r.get::<_, i64>(3)?).unwrap_or(0),
                        updated_unix_ms: u64::try_from(r.get::<_, i64>(4)?).unwrap_or(0),
                    })
                },
            )
            .map_err(storage)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage)?;
        Ok(rows)
    }

    fn delete(&self, principal: &str, name: &str) -> Result<bool, PolicyAdminError> {
        let conn = self.conn.lock().map_err(storage)?;
        let removed = conn
            .execute(
                "DELETE FROM policy_roles WHERE principal = ?1 AND name = ?2",
                params![principal, name],
            )
            .map_err(storage)?;
        // Cascade the assignments. Leaving them would make `allowlist_for` name a
        // role that no longer exists, and the honest reading of that is "no
        // narrowing" — which is a WIDENING expressed as an orphan row instead of
        // as a decision. Deleting them makes the widening explicit and keeps the
        // two tables agreeing.
        conn.execute(
            "DELETE FROM policy_assignments WHERE principal = ?1 AND role_name = ?2",
            params![principal, name],
        )
        .map_err(storage)?;
        Ok(removed > 0)
    }

    fn assign(
        &self,
        principal: &str,
        party: &str,
        name: Option<&str>,
    ) -> Result<bool, PolicyAdminError> {
        let conn = self.conn.lock().map_err(storage)?;
        let Some(name) = name else {
            conn.execute(
                "DELETE FROM policy_assignments WHERE principal = ?1 AND party = ?2",
                params![principal, party],
            )
            .map_err(storage)?;
            return Ok(false);
        };
        // Assigning a role that does not exist must NOT silently succeed: the
        // caller would believe an authority had been reduced when it had not.
        let known: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM policy_roles WHERE principal = ?1 AND name = ?2",
                params![principal, name],
                |r| r.get(0),
            )
            .map_err(storage)?;
        if known == 0 {
            return Err(PolicyAdminError::NotFound(name.to_string()));
        }
        conn.execute(
            "INSERT INTO policy_assignments (principal, party, role_name)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(principal, party) DO UPDATE SET role_name = excluded.role_name",
            params![principal, party, name],
        )
        .map_err(storage)?;
        Ok(true)
    }

    fn allowlist_for(
        &self,
        party: &str,
    ) -> Result<Option<Vec<PolicyRoleToolWire>>, PolicyAdminError> {
        let conn = self.conn.lock().map_err(storage)?;
        // By PARTY, not by (principal, party) — see the seam doc. At fire time
        // the runtime knows which party a warrant resolves for and nothing about
        // who authored the role that constrains it. `ORDER BY a.principal` makes
        // the answer DETERMINISTIC if two principals ever assign to one party
        // (impossible on single-node OSS; a cloud concern under D129) — a
        // narrowing that varied run to run would be worse than either answer.
        let raw: Option<String> = conn
            .query_row(
                "SELECT r.tools_json
                 FROM policy_assignments a
                 JOIN policy_roles r
                   ON r.principal = a.principal AND r.name = a.role_name
                 WHERE a.party = ?1
                 ORDER BY a.principal ASC
                 LIMIT 1",
                params![party],
                |r| r.get(0),
            )
            .ok();
        // `None` — no assignment — must resolve exactly as a serve with no policy
        // registry at all. That is the compatibility contract, not a convenience:
        // every existing install has no assignments, so this arm is the one that
        // decides whether upgrading changes behaviour.
        Ok(raw.map(|j| tools_from_json(&j)))
    }
}

#[cfg(test)]
mod tests {
    use super::{tools_from_json, tools_to_json, PoliciesDb};
    use kx_gateway_core::{PolicyAdmin, PolicyAdminError, PolicyRoleRow, PolicyRoleToolWire};

    fn tool(id: &str, version: &str) -> PolicyRoleToolWire {
        PolicyRoleToolWire {
            tool_id: id.into(),
            tool_version: version.into(),
        }
    }

    fn role(name: &str, tools: Vec<PolicyRoleToolWire>) -> PolicyRoleRow {
        PolicyRoleRow {
            name: name.into(),
            description: String::new(),
            tools,
            created_unix_ms: 1,
            updated_unix_ms: 1,
        }
    }

    fn db() -> (tempfile::TempDir, PoliciesDb) {
        let dir = tempfile::tempdir().unwrap();
        let db = PoliciesDb::open(dir.path()).unwrap();
        (dir, db)
    }

    #[test]
    fn a_put_then_list_round_trips_the_allowlist() {
        let (_d, db) = db();
        assert!(db
            .put("p", role("ops", vec![tool("fs.read", "1")]))
            .unwrap());
        // A second put UPDATES rather than creating.
        assert!(!db
            .put(
                "p",
                role("ops", vec![tool("fs.read", "1"), tool("http.get", "2")])
            )
            .unwrap());
        let rows = db.list("p", 100).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].tools.len(), 2);
    }

    /// The scope is real: another principal sees nothing.
    #[test]
    fn roles_are_caller_scoped() {
        let (_d, db) = db();
        db.put("p", role("ops", vec![tool("fs.read", "1")]))
            .unwrap();
        assert!(db.list("other", 100).unwrap().is_empty());
        // And cannot be assigned across the scope boundary.
        assert!(matches!(
            db.assign("other", "party-a", Some("ops")),
            Err(PolicyAdminError::NotFound(_))
        ));
    }

    /// No assignment ⇒ `None`, which is what keeps existing installs unchanged.
    #[test]
    fn an_unassigned_party_expresses_no_narrowing() {
        let (_d, db) = db();
        db.put("p", role("ops", vec![tool("fs.read", "1")]))
            .unwrap();
        assert_eq!(db.allowlist_for("party-a").unwrap(), None);
    }

    /// An EMPTY role is not the same as no role: it narrows to nothing.
    #[test]
    fn an_empty_role_is_some_empty_not_none() {
        let (_d, db) = db();
        db.put("p", role("locked", vec![])).unwrap();
        db.assign("p", "party-a", Some("locked")).unwrap();
        assert_eq!(db.allowlist_for("party-a").unwrap(), Some(vec![]));
    }

    #[test]
    fn assigning_an_unknown_role_is_not_a_silent_success() {
        let (_d, db) = db();
        let err = db.assign("p", "party-a", Some("nope")).unwrap_err();
        assert!(matches!(err, PolicyAdminError::NotFound(ref n) if n == "nope"));
        // And nothing was written — refused means nothing happened.
        assert_eq!(db.allowlist_for("party-a").unwrap(), None);
    }

    #[test]
    fn unassigning_returns_the_party_to_no_narrowing() {
        let (_d, db) = db();
        db.put("p", role("ops", vec![tool("fs.read", "1")]))
            .unwrap();
        db.assign("p", "party-a", Some("ops")).unwrap();
        assert!(db.allowlist_for("party-a").unwrap().is_some());
        assert!(!db.assign("p", "party-a", None).unwrap());
        assert_eq!(db.allowlist_for("party-a").unwrap(), None);
    }

    /// Deleting a role cascades its assignments, so the two tables cannot
    /// disagree and the widening is explicit rather than an orphan row.
    #[test]
    fn deleting_a_role_cascades_its_assignments() {
        let (_d, db) = db();
        db.put("p", role("ops", vec![tool("fs.read", "1")]))
            .unwrap();
        db.assign("p", "party-a", Some("ops")).unwrap();
        assert!(db.delete("p", "ops").unwrap());
        assert_eq!(
            db.allowlist_for("party-a").unwrap(),
            None,
            "a deleted role leaves no orphan assignment"
        );
        // Deleting again is not an error, and reports nothing removed.
        assert!(!db.delete("p", "ops").unwrap());
    }

    /// `created_unix_ms` records CREATION, not the last edit.
    #[test]
    fn an_update_preserves_the_creation_timestamp() {
        let (_d, db) = db();
        db.put(
            "p",
            PolicyRoleRow {
                created_unix_ms: 111,
                updated_unix_ms: 111,
                ..role("ops", vec![])
            },
        )
        .unwrap();
        db.put(
            "p",
            PolicyRoleRow {
                created_unix_ms: 999,
                updated_unix_ms: 999,
                ..role("ops", vec![])
            },
        )
        .unwrap();
        let rows = db.list("p", 100).unwrap();
        assert_eq!(
            rows[0].created_unix_ms, 111,
            "creation is not the last edit"
        );
        assert_eq!(rows[0].updated_unix_ms, 999);
    }

    /// An unreadable allowlist fails CLOSED (empty), never open.
    #[test]
    fn a_corrupt_allowlist_narrows_to_nothing_rather_than_everything() {
        assert!(tools_from_json("not json at all").is_empty());
        assert!(tools_from_json("").is_empty());
        // Anti-vacuity: the encoder and decoder actually agree on well-formed data,
        // so the emptiness above is a failure mode and not the only behaviour.
        let round = tools_from_json(&tools_to_json(&[tool("a", "1"), tool("b", "2")]));
        assert_eq!(round.len(), 2);
        assert_eq!(round[0].tool_id, "a");
    }
}
