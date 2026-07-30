// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! The ONE upgrade policy for the gateway's SQLite sidecars.
//!
//! Every sidecar under `--catalog-dir` used to carry its own copy of the same open
//! sequence, and every copy ended in the same line: on a `schema_version` mismatch,
//! `DROP TABLE` and start empty. For a derived cache that is correct and free. For a
//! store holding what a USER AUTHORED — their apps, their workflows, their triggers —
//! it means a version bump silently deletes their work on the next boot.
//!
//! That was not a hypothetical. `apps.rs` documents the consequence in a comment on the
//! additive-column path: *"a bump would drop saved apps"* — the schema version was being
//! held frozen to route around a destructive upgrade rather than fix it. Freezing a
//! version to avoid data loss is a debt that compounds: it also freezes the schema.
//!
//! ## The policy
//!
//! One decision, stated once, applied by [`open_sidecar`]:
//!
//! | found vs mine | [`Durability::UserAuthored`] | [`Durability::Cache`] |
//! |---|---|---|
//! | equal / absent | open (create if new) | open (create if new) |
//! | found < mine (upgrade) | **rename aside, recreate, import what still fits** | recreate empty |
//! | found > mine (downgrade) | **REFUSE — never wipe** | recreate empty |
//!
//! **Rename aside, never delete.** The old file is moved to `<name>.db.v<found>.bak`
//! (with its `-wal`/`-shm`), then the surviving columns are imported into the fresh
//! schema by INTERSECTION of old and new column names. A column the new schema dropped
//! is left behind in the `.bak`; a column it added takes its default. If the import
//! fails for any reason the boot still succeeds — the `.bak` is the safety net, and a
//! recoverable file on disk beats a refused boot.
//!
//! **A downgrade REFUSES.** An older binary cannot know what a newer schema meant, so
//! "migrate" is not available and "wipe" is not acceptable. It says so, names the file,
//! and stops. A cache may still be rebuilt, because by construction it holds nothing
//! that was not derived from something else.
//!
//! The `Cache` arm is byte-identical to the previous behaviour. The classification is
//! per-store and deliberate — see each caller.

use std::path::{Path, PathBuf};

use rusqlite::{params, Connection};

use crate::error::GatewayError;

/// What a sidecar holds, and therefore what may be done to it on a version change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Durability {
    /// Holds facts a user authored and cannot regenerate: apps, workflows, triggers,
    /// branch bindings, skills, secret NAMES. Never wiped by an upgrade.
    UserAuthored,
    /// Holds only values derived from something else that still exists (run exhaust,
    /// telemetry, alerts, uploads, locks). Safe to rebuild empty.
    Cache,
}

/// Open a sidecar under `dir`, applying the upgrade policy above.
///
/// `tables` is every table the store owns (including its `meta`), used both to rebuild a
/// cache and to drive the intersection import for a user-authored store.
///
/// # Errors
/// [`GatewayError::Catalog`] on an unrecoverable open/pragma failure, or on a DOWNGRADE
/// of a [`Durability::UserAuthored`] store — which is refused rather than wiped.
pub(crate) fn open_sidecar(
    dir: &Path,
    file_name: &str,
    schema_version: i64,
    schema_ddl: &str,
    tables: &[&str],
    durability: Durability,
) -> Result<Connection, GatewayError> {
    std::fs::create_dir_all(dir)
        .map_err(|e| GatewayError::Catalog(format!("{file_name} dir: {e}")))?;
    let db_path = dir.join(file_name);

    // A file we cannot even open with our pragmas is corrupt or foreign. That is not a
    // version question and the previous behaviour (drop it and start over) is kept for
    // BOTH classes: there is nothing in it we could read in order to save it.
    let mut conn = if let Ok(c) = open_with_pragma(&db_path, durability) {
        c
    } else {
        remove_db_files(&db_path);
        open_with_pragma(&db_path, durability)
            .map_err(|e| GatewayError::Catalog(format!("{file_name} reopen: {e}")))?
    };

    let found = read_schema_version(&conn);
    let mut imported_from: Option<PathBuf> = None;

    match found {
        // Current, or a fresh/unreadable file that is about to be created below.
        Some(v) if v == schema_version => {}
        None => {}
        Some(found_v) if found_v > schema_version => {
            if durability == Durability::UserAuthored {
                return Err(GatewayError::Catalog(format!(
                    "{file_name} was written by a NEWER kortecx (schema v{found_v}; this \
                     binary speaks v{schema_version}). Refusing to open it: an older \
                     binary cannot know what the newer schema meant, and this file holds \
                     authored work that must not be discarded to make the boot succeed. \
                     Run the newer version, or move {file_name} aside deliberately."
                )));
            }
            drop_tables(&conn, tables, file_name)?;
        }
        Some(found_v) => {
            // An upgrade.
            if durability == Durability::UserAuthored {
                drop(conn);
                let aside = rename_aside(&db_path, found_v, file_name)?;
                conn = open_with_pragma(&db_path, durability).map_err(|e| {
                    GatewayError::Catalog(format!("{file_name} reopen after rename: {e}"))
                })?;
                imported_from = Some(aside);
            } else {
                drop_tables(&conn, tables, file_name)?;
            }
        }
    }

    conn.execute_batch(schema_ddl)
        .map_err(|e| GatewayError::Catalog(format!("{file_name} schema: {e}")))?;
    conn.execute(
        "INSERT OR IGNORE INTO meta(key, value) VALUES ('schema_version', ?1)",
        params![schema_version],
    )
    .map_err(|e| GatewayError::Catalog(format!("{file_name} meta init: {e}")))?;

    if let Some(aside) = imported_from {
        // Best-effort BY DESIGN: the `.bak` is the real guarantee. A failed import must
        // not turn an upgrade into a refused boot, and the operator still has the file.
        if let Err(e) = import_intersecting(&conn, &aside, tables) {
            tracing::warn!(
                file = file_name,
                backup = %aside.display(),
                error = %e,
                "could not import rows from the pre-upgrade sidecar; it is preserved \
                 alongside the new one and can be recovered by hand"
            );
        }
    }

    Ok(conn)
}

/// Open with the durability posture the store's CLASS earns.
///
/// `synchronous = FULL` fsyncs on every commit; `NORMAL` batches, which can lose
/// the last transactions to a power cut while keeping the file consistent. That
/// trade is right for a cache you can rebuild and wrong for work a user authored
/// and cannot: losing the last App someone saved is not "rebuildable", it is data
/// loss, and it would be a strange thing to accept in the very module whose job
/// is to stop upgrades destroying authored work.
fn open_with_pragma(db_path: &Path, durability: Durability) -> rusqlite::Result<Connection> {
    let conn = Connection::open(db_path)?;
    let sync = match durability {
        Durability::UserAuthored => "FULL",
        Durability::Cache => "NORMAL",
    };
    conn.execute_batch(&format!(
        "PRAGMA journal_mode = WAL; PRAGMA synchronous = {sync};"
    ))?;
    Ok(conn)
}

fn remove_db_files(db_path: &Path) {
    let _ = std::fs::remove_file(db_path);
    for suffix in ["-wal", "-shm"] {
        let mut p = db_path.as_os_str().to_os_string();
        p.push(suffix);
        let _ = std::fs::remove_file(PathBuf::from(p));
    }
}

/// Move `<name>.db` (and its sidecar journals) to `<name>.db.v<found>.bak`.
///
/// A pre-existing `.bak` from an earlier upgrade is not clobbered — it gains a numeric
/// suffix — because the whole point is that nothing a user authored is destroyed.
fn rename_aside(db_path: &Path, found_v: i64, file_name: &str) -> Result<PathBuf, GatewayError> {
    let mut target = PathBuf::from(format!("{}.v{found_v}.bak", db_path.display()));
    let mut nth = 1;
    while target.exists() {
        target = PathBuf::from(format!("{}.v{found_v}.bak.{nth}", db_path.display()));
        nth += 1;
    }
    std::fs::rename(db_path, &target).map_err(|e| {
        GatewayError::Catalog(format!(
            "{file_name}: could not preserve the pre-upgrade catalog as {}: {e}",
            target.display()
        ))
    })?;
    // The journals belong to the old file; leaving them would corrupt the new one.
    remove_db_files(db_path);
    tracing::info!(
        file = file_name,
        backup = %target.display(),
        from_schema = found_v,
        "sidecar schema upgraded; the pre-upgrade catalog is preserved"
    );
    Ok(target)
}

fn drop_tables(conn: &Connection, tables: &[&str], file_name: &str) -> Result<(), GatewayError> {
    use std::fmt::Write as _;
    let mut batch = String::with_capacity(tables.len() * 40);
    for t in tables {
        // Infallible into a String; the `let _` keeps the lint quiet without hiding a
        // real error path (there isn't one).
        let _ = writeln!(batch, "DROP TABLE IF EXISTS {t};");
    }
    conn.execute_batch(&batch)
        .map_err(|e| GatewayError::Catalog(format!("{file_name} rebuild: {e}")))
}

/// Copy rows from the renamed-aside file into the fresh schema, column by column.
///
/// Only columns present in BOTH schemas move. A dropped column stays in the `.bak`; an
/// added column takes its declared default. Tables absent from either side are skipped.
fn import_intersecting(
    conn: &Connection,
    aside: &Path,
    tables: &[&str],
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "ATTACH DATABASE ?1 AS prev",
        params![aside.to_string_lossy()],
    )?;
    let result = (|| -> Result<(), rusqlite::Error> {
        for table in tables {
            // `meta` needs one exception rather than a skip. Importing it wholesale would
            // reinstate the OLD `schema_version` and the next boot would upgrade again,
            // forever. But other meta keys are real state a store depends on — branches
            // versions its history table independently under `history_schema_version`,
            // and dropping that key silently changes what the staleness check decides. So
            // every key EXCEPT `schema_version` carries across.
            if *table == "meta" {
                conn.execute_batch(
                    "INSERT OR IGNORE INTO main.meta (key, value)
                     SELECT key, value FROM prev.meta WHERE key <> 'schema_version';",
                )?;
                continue;
            }
            let old_cols = columns_of(conn, "prev", table)?;
            if old_cols.is_empty() {
                continue;
            }
            let new_cols = columns_of(conn, "main", table)?;
            let shared: Vec<String> = new_cols
                .into_iter()
                .filter(|c| old_cols.contains(c))
                .collect();
            if shared.is_empty() {
                continue;
            }
            let list = shared.join(", ");
            conn.execute_batch(&format!(
                "INSERT OR IGNORE INTO main.{table} ({list}) SELECT {list} FROM prev.{table};"
            ))?;
        }
        Ok(())
    })();
    conn.execute_batch("DETACH DATABASE prev")?;
    result
}

fn columns_of(
    conn: &Connection,
    schema: &str,
    table: &str,
) -> Result<Vec<String>, rusqlite::Error> {
    let mut stmt = conn.prepare(&format!("PRAGMA {schema}.table_info({table})"))?;
    let cols = stmt
        .query_map([], |r| r.get::<_, String>(1))?
        .filter_map(Result::ok)
        .collect();
    Ok(cols)
}

fn read_schema_version(conn: &Connection) -> Option<i64> {
    conn.query_row(
        "SELECT value FROM meta WHERE key = 'schema_version'",
        [],
        |r| r.get(0),
    )
    .ok()
}
