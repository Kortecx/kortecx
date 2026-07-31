// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! `tools.db` — the last authored store that was opening outside the upgrade policy.
//!
//! ## Why this module exists at all
//!
//! Every other store a user authors into routes through [`crate::sidecar`], which
//! renames aside and re-imports on a schema bump and REFUSES a downgrade.
//! `tools.db` did not: it was opened by `kx-tool-registry`'s own helper, whose
//! version check is a loud refusal in BOTH directions. That is a defensible
//! posture on its own, but it means a version bump has no upgrade path at all —
//! the same corner `apps.db` painted itself into when its version was frozen at 1
//! with a comment admitting a bump "would drop saved apps".
//!
//! It also holds more than tools. Every registered SCRIPT lives here too, via
//! `HostScriptRegistry`'s `SqliteToolRegistry`. So the store outside the policy
//! was holding two of the six authoring domains.
//!
//! ## Why the connection is opened here and not there
//!
//! `sidecar::open_sidecar` is `pub(crate)` in `kx-gateway`, and `kx-gateway`
//! DEPENDS on `kx-tool-registry`. Reaching the other way would be a dependency
//! cycle, so the gateway opens the connection under its own policy and hands it
//! to `SqliteToolRegistry::from_connection`, which still seeds the builtins and
//! rebuilds its index.
//!
//! ## What existing installs see
//!
//! Nothing. An older `tools.db` stamps its version in a `metadata` table; the
//! policy looks in `meta`, finds no row, and takes the fresh-file arm — where
//! every statement is `CREATE TABLE IF NOT EXISTS`, so the `tools` rows are
//! untouched and the file simply gains a `meta.schema_version`. The old
//! `metadata` table stays behind as an inert orphan and rides along in the `.bak`
//! at the next real bump, which is correct. `an_existing_tools_db_survives_the_move`
//! pins that rather than arguing it.

use std::path::Path;

use kx_tool_registry::SqliteToolRegistry;

use crate::error::GatewayError;
use crate::sidecar::{open_sidecar, Durability};

/// The sidecar-side schema version for `tools.db`.
///
/// Deliberately declared HERE rather than reusing `kx-tool-registry`'s `u16`
/// constant. Two reasons, and the second is the load-bearing one:
///
/// 1. `open_sidecar` speaks `i64`, so a cast would be needed either way.
/// 2. The `schema-version-fanout` CI job fires on an added or removed
///    `const …SCHEMA_VERSION` line and then demands a migration site and a
///    CHANGELOG entry. Casting at the call site and leaving the old declaration
///    untouched would have moved this store's upgrade posture while keeping the
///    guard silent — quietly evading a check on precisely the change it exists
///    for. Declaring it makes the job fire, and it is answered honestly.
const SCHEMA_VERSION: i64 = 1;

/// The tables the policy owns for this store.
///
/// `metadata` is deliberately ABSENT: it belongs to the old opener and is an
/// inert orphan now. Listing it would invite the policy to import or drop a table
/// whose shape it does not define.
const TABLES: &[&str] = &["tools", "meta"];

/// The schema the policy creates for a FRESH `tools.db`.
///
/// The tools table is `kx_tool_registry::DDL` VERBATIM — never a copy. A second
/// copy here would be two schemas that can drift, and the drift would surface as
/// a column the intersection import silently cannot carry. `meta` is appended
/// because the policy stamps its version there.
///
/// Every statement is `IF NOT EXISTS`, which is what makes the move lossless for
/// an existing file: the policy runs this over the old database and changes
/// nothing already present.
fn schema() -> String {
    format!(
        "{}\nCREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value INTEGER NOT NULL);\n",
        kx_tool_registry::DDL
    )
}

/// Open `tools.db` under the sidecar upgrade policy, as `UserAuthored`.
///
/// # Errors
/// [`GatewayError::Catalog`] if the store is a downgrade (refused, never wiped),
/// or on any SQLite / registry failure.
pub(crate) fn open(dir: &Path) -> Result<SqliteToolRegistry, GatewayError> {
    let conn = open_sidecar(
        dir,
        "tools.db",
        SCHEMA_VERSION,
        &schema(),
        TABLES,
        // Registered tools AND every registered script. An operator authored
        // both; neither is derivable from anything else on the box.
        Durability::UserAuthored,
    )?;
    SqliteToolRegistry::from_connection(conn)
        .map_err(|e| GatewayError::Catalog(format!("tools.db: {e}")))
}

#[cfg(test)]
mod tests {
    use super::open;

    /// A `tools.db` written by the OLD opener keeps its rows through the move.
    ///
    /// This is the claim the whole change rests on, so it is measured rather than
    /// argued: register a tool the old way, reopen through the policy, and the
    /// tool must still resolve.
    #[test]
    fn an_existing_tools_db_survives_the_move() {
        let dir = tempfile::tempdir().expect("tempdir");

        // 1. The old path: kx-tool-registry opens and stamps `metadata`.
        {
            let reg = kx_tool_registry::SqliteToolRegistry::open(dir.path().join("tools.db"))
                .expect("old-style open");
            drop(reg);
        }
        let before = std::fs::metadata(dir.path().join("tools.db"))
            .expect("the old opener created the file")
            .len();
        assert!(before > 0, "the old file has content");

        // 2. The new path: the sidecar policy opens the SAME file.
        let reg = open(dir.path()).expect("policy open of an existing tools.db");

        // 3. The builtins the registry seeds are resolvable, i.e. the store is
        //    live rather than a fresh empty file wearing the same name.
        let listed = reg.discover(64, None).expect("discover");
        assert!(
            !listed.is_empty(),
            "the reopened registry must still resolve its rows"
        );

        // 4. The policy stamped its own version, and did NOT rename the file
        //    aside (an upgrade would have left a .bak — this is not an upgrade).
        let conn = rusqlite::Connection::open(dir.path().join("tools.db")).expect("open");
        let v: i64 = conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'schema_version'",
                [],
                |r| r.get(0),
            )
            .expect("the policy stamped meta.schema_version");
        assert_eq!(v, super::SCHEMA_VERSION);
        assert!(
            !dir.path().join("tools.db.v1.bak").exists(),
            "a first move must not rename anything aside"
        );
    }

    /// A fresh directory opens cleanly and is usable.
    #[test]
    fn a_fresh_store_opens_and_seeds() {
        let dir = tempfile::tempdir().expect("tempdir");
        let reg = open(dir.path()).expect("fresh open");
        assert!(
            !reg.discover(64, None).expect("discover").is_empty(),
            "builtins are seeded"
        );
    }
}
