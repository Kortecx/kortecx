// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! `kx migrate` — bring a journal written by an older kortecx up to this binary's
//! schema version, verifying that the run's product identity did not move.
//!
//! ## Why this verb exists
//!
//! [`kx_runtime::migrate_and_verify`] has been the supported upgrade path since M2.x-E,
//! and until now **no user could reach it**: it was a library function with no CLI verb
//! and no RPC. What a user actually experienced on upgrade was `kx serve` refusing to
//! start with a `SchemaVersionMismatch` and no remedy named — the fix existed, shipped,
//! and was unreachable. A guard nobody can invoke is not a guard.
//!
//! ## What it does
//!
//! Rewrites `--journal` into a fresh current-version journal, then folds BOTH sides and
//! refuses unless their committed-facts digests are byte-identical. The source is never
//! modified. By default the migrated journal is written beside the source and swapped in
//! only after verification passes, with the original preserved as `<name>.v<N>.bak`;
//! `--out` writes to an explicit destination and leaves the source in place instead.

use std::path::{Path, PathBuf};

use crate::error::CliError;

/// Parsed `migrate` arguments.
#[derive(Debug)]
pub struct MigrateArgs {
    /// The journal to migrate. Required — this verb never guesses at a data dir,
    /// because writing the wrong journal is not recoverable by re-running.
    pub journal: PathBuf,
    /// Explicit destination. `None` ⇒ migrate in place (source preserved as a `.bak`).
    pub out: Option<PathBuf>,
    /// Report what would happen without writing anything.
    pub dry_run: bool,
    /// Emit the report as JSON.
    pub json: bool,
}

/// Parse `migrate` args (the verb already consumed).
pub fn parse(mut args: impl Iterator<Item = String>) -> Result<MigrateArgs, CliError> {
    let mut journal: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut dry_run = false;
    let mut json = false;

    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--journal" => {
                journal = Some(PathBuf::from(next_arg(&mut args, "--journal")?));
            }
            "--out" => out = Some(PathBuf::from(next_arg(&mut args, "--out")?)),
            "--dry-run" => dry_run = true,
            "--json" => json = true,
            other => {
                return Err(CliError::Usage(format!(
                    "migrate: unknown flag `{other}` (see `kx help migrate`)"
                )))
            }
        }
    }

    Ok(MigrateArgs {
        journal: journal.ok_or_else(|| {
            CliError::Usage(
                "migrate requires --journal <path> (the journal to migrate; it is never \
                 inferred, because migrating the wrong file is not undone by re-running)"
                    .into(),
            )
        })?,
        out,
        dry_run,
        json,
    })
}

fn next_arg(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, CliError> {
    args.next()
        .ok_or_else(|| CliError::Usage(format!("migrate: {flag} requires a value")))
}

/// Decide whether there is anything to do, reporting the cases where there is not.
///
/// `Ok(Some(found))` ⇒ migrate FROM `found`. `Ok(None)` ⇒ nothing to do and the reason
/// has already been printed. `Err` ⇒ refuse (unreadable, or written by a newer binary).
fn triage(a: &MigrateArgs) -> Result<Option<u16>, CliError> {
    if !a.journal.exists() {
        return Err(CliError::Config(format!(
            "migrate: no journal at {}",
            a.journal.display()
        )));
    }
    let found = kx_runtime::journal_schema_version(&a.journal).map_err(|e| {
        CliError::Config(format!("migrate: cannot read {}: {e}", a.journal.display()))
    })?;
    let current = kx_runtime::JOURNAL_SCHEMA_VERSION;

    if found == current {
        if a.json {
            println!("{{\"status\":\"already-current\",\"schema_version\":{current}}}");
        } else {
            println!(
                "{} is already at schema v{current} — nothing to migrate",
                a.journal.display()
            );
        }
        return Ok(None);
    }
    if found > current {
        return Err(CliError::Config(format!(
            "migrate: {} was written at schema v{found}, but this binary speaks v{current}. \
             A newer journal cannot be migrated DOWN — an older binary cannot know what the \
             newer schema meant. Run the newer kortecx instead.",
            a.journal.display()
        )));
    }
    if a.dry_run {
        if a.json {
            println!(
                "{{\"status\":\"dry-run\",\"from_version\":{found},\"to_version\":{current}}}"
            );
        } else {
            println!(
                "would migrate {} from schema v{found} to v{current} \
                 (source preserved; product digest verified before the swap)",
                a.journal.display()
            );
        }
        return Ok(None);
    }
    Ok(Some(found))
}

/// Run the migration.
pub fn execute(a: &MigrateArgs) -> Result<(), CliError> {
    let Some(found) = triage(a)? else {
        return Ok(()); // already current, or a dry run — both already reported.
    };

    // Write to a sibling first in BOTH modes: `migrate_and_verify` refuses on a digest
    // mismatch, and the source must still be intact when it does.
    let staged = match &a.out {
        Some(dst) => dst.clone(),
        None => sibling(&a.journal, ".migrating"),
    };
    if staged.exists() {
        return Err(CliError::Config(format!(
            "migrate: {} already exists — move it aside first",
            staged.display()
        )));
    }

    let report = kx_runtime::migrate_and_verify(&a.journal, &staged).map_err(|e| {
        let _ = std::fs::remove_file(&staged);
        CliError::Config(format!(
            "migrate: {e}\nThe source journal at {} is UNCHANGED.",
            a.journal.display()
        ))
    })?;

    let backup = if a.out.is_none() {
        // In-place: preserve the original, then swap the verified journal in.
        let backup = sibling(&a.journal, &format!(".v{found}.bak"));
        std::fs::rename(&a.journal, &backup).map_err(|e| {
            CliError::Config(format!(
                "migrate: verified the migration but could not preserve the original as {}: {e}",
                backup.display()
            ))
        })?;
        std::fs::rename(&staged, &a.journal).map_err(|e| {
            CliError::Config(format!(
                "migrate: could not move the migrated journal into place: {e}\n\
                 The original is at {} — rename it back to recover.",
                backup.display()
            ))
        })?;
        Some(backup)
    } else {
        None
    };

    if a.json {
        println!(
            "{{\"status\":\"migrated\",\"from_version\":{},\"to_version\":{},\
             \"entries_migrated\":{},\"entries_upconverted\":{}}}",
            report.from_version,
            report.to_version,
            report.entries_migrated,
            report.entries_upconverted
        );
    } else {
        println!(
            "migrated {} : schema v{} -> v{} ({} entries, {} up-converted)",
            a.journal.display(),
            report.from_version,
            report.to_version,
            report.entries_migrated,
            report.entries_upconverted
        );
        println!("product digest verified identical across the rewrite");
        match (&backup, &a.out) {
            (Some(b), _) => println!("original preserved at {}", b.display()),
            (None, Some(dst)) => println!(
                "written to {} (source unchanged at {})",
                dst.display(),
                a.journal.display()
            ),
            (None, None) => {}
        }
    }
    Ok(())
}

/// `<path>` with `suffix` appended to the file name.
fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(suffix);
    PathBuf::from(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_ok(argv: &[&str]) -> MigrateArgs {
        parse(argv.iter().map(std::string::ToString::to_string)).unwrap()
    }

    #[test]
    fn journal_is_required_and_never_inferred() {
        let err = parse(std::iter::empty()).unwrap_err();
        assert!(
            err.to_string().contains("--journal"),
            "the usage error names the missing flag: {err}"
        );
    }

    #[test]
    fn flags_round_trip() {
        let a = parse_ok(&["--journal", "/tmp/a.db", "--out", "/tmp/b.db", "--json"]);
        assert_eq!(a.journal, PathBuf::from("/tmp/a.db"));
        assert_eq!(a.out, Some(PathBuf::from("/tmp/b.db")));
        assert!(a.json);
        assert!(!a.dry_run);
        assert!(parse_ok(&["--journal", "/tmp/a.db", "--dry-run"]).dry_run);
    }

    #[test]
    fn an_unknown_flag_is_a_usage_error() {
        let err = parse(["--nope".to_string()].into_iter()).unwrap_err();
        assert!(err.to_string().contains("unknown flag"));
    }

    #[test]
    fn a_missing_journal_file_is_refused_before_any_write() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope.db");
        let err = execute(&MigrateArgs {
            journal: missing.clone(),
            out: None,
            dry_run: false,
            json: false,
        })
        .unwrap_err();
        assert!(err.to_string().contains("no journal at"));
        assert!(!dir.path().join("nope.db.migrating").exists());
    }

    #[test]
    fn sibling_appends_to_the_file_name() {
        assert_eq!(
            sibling(Path::new("/a/b/kx.db"), ".v16.bak"),
            PathBuf::from("/a/b/kx.db.v16.bak")
        );
    }
}
