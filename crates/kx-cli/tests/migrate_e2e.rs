// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! `kx migrate` against a REAL journal written at an older schema version.
//!
//! The verb's unit tests only cover argument parsing, which would pass just as happily
//! if the verb migrated nothing at all. These drive the actual upgrade an adopter hits:
//! a journal an older binary wrote, refused by `SqliteJournal::open`, migrated, and then
//! opening cleanly — with the run's product identity proven unchanged across the rewrite.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::path::{Path, PathBuf};

use kx_journal::Journal as _;
use kx_runtime::{digest_journal, journal_schema_version, JOURNAL_SCHEMA_VERSION};

/// Write a journal, then stamp its `metadata.schema_version` back to `version` so it
/// looks exactly like a file an older binary left behind.
///
/// The entries themselves stay current-shaped. That is deliberate and is what makes this
/// a test of the VERB rather than of the entry up-converters, which
/// `kx-runtime/tests/schema_evolution.rs` already covers against curated old bytes.
fn journal_stamped_at(dir: &Path, version: u16) -> PathBuf {
    let path = dir.join("run.kxjournal");
    {
        // A minimal real run: registering one is enough to have committed facts to fold.
        let j = kx_journal::SqliteJournal::open(&path).unwrap();
        j.append(kx_journal::JournalEntry::RunRegistered {
            instance_id: [7u8; kx_journal::INSTANCE_ID_LEN],
            recipe_fingerprint: [9u8; 32],
            ts: 0,
            seq: 0,
        })
        .unwrap();
    }
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute(
        "UPDATE metadata SET value = ?1 WHERE key = 'schema_version'",
        rusqlite::params![&version.to_le_bytes()[..]],
    )
    .unwrap();
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
        .unwrap();
    path
}

fn migrate(journal: &Path, out: Option<&Path>, dry_run: bool) -> Result<(), String> {
    kx_cli::verbs::migrate::execute(&kx_cli::verbs::migrate::MigrateArgs {
        journal: journal.to_path_buf(),
        out: out.map(Path::to_path_buf),
        dry_run,
        json: false,
    })
    .map_err(|e| e.to_string())
}

/// The whole point: a journal the current binary REFUSES becomes one it accepts, and the
/// committed facts are provably the same on both sides.
#[test]
fn an_old_journal_is_refused_then_migrated_then_opens() {
    let tmp = tempfile::tempdir().unwrap();
    let old = JOURNAL_SCHEMA_VERSION - 1;
    let path = journal_stamped_at(tmp.path(), old);

    // Precondition — without this the test could pass on a journal that never needed
    // migrating, which is the failure mode that makes a green migration test worthless.
    let before = digest_journal(&kx_journal::ReplayJournal::open(&path).unwrap()).unwrap();
    assert!(
        kx_journal::SqliteJournal::open(&path).is_err(),
        "the current binary must REFUSE this journal before migration, or there is \
         nothing here to fix"
    );

    migrate(&path, None, false).expect("migrate succeeds");

    assert_eq!(
        journal_schema_version(&path).unwrap(),
        JOURNAL_SCHEMA_VERSION,
        "the journal is now at the current schema"
    );
    let reopened = kx_journal::SqliteJournal::open(&path)
        .expect("the migrated journal opens with the strict current-version open");
    assert_eq!(
        digest_journal(&reopened).unwrap().to_hex(),
        before.to_hex(),
        "THE DURABILITY LAW: migration must not move the committed-facts digest"
    );

    let backup = PathBuf::from(format!("{}.v{old}.bak", path.display()));
    assert!(
        backup.exists(),
        "the pre-migration journal is preserved, not consumed"
    );
    assert_eq!(
        journal_schema_version(&backup).unwrap(),
        old,
        "and the backup is still the OLD journal"
    );
}

/// `--out` leaves the source alone — the non-destructive mode an operator reaches for
/// when they want to keep the original in place.
#[test]
fn out_writes_a_copy_and_leaves_the_source_untouched() {
    let tmp = tempfile::tempdir().unwrap();
    let old = JOURNAL_SCHEMA_VERSION - 1;
    let path = journal_stamped_at(tmp.path(), old);
    let dst = tmp.path().join("migrated.kxjournal");

    migrate(&path, Some(&dst), false).expect("migrate --out succeeds");

    assert_eq!(
        journal_schema_version(&dst).unwrap(),
        JOURNAL_SCHEMA_VERSION
    );
    assert_eq!(
        journal_schema_version(&path).unwrap(),
        old,
        "the SOURCE is untouched when --out is given"
    );
}

/// `--dry-run` reports and writes nothing.
#[test]
fn dry_run_writes_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let old = JOURNAL_SCHEMA_VERSION - 1;
    let path = journal_stamped_at(tmp.path(), old);

    migrate(&path, None, true).expect("dry-run succeeds");

    assert_eq!(
        journal_schema_version(&path).unwrap(),
        old,
        "--dry-run must not migrate"
    );
    assert!(!PathBuf::from(format!("{}.v{old}.bak", path.display())).exists());
    assert!(!PathBuf::from(format!("{}.migrating", path.display())).exists());
}

/// Migrating an already-current journal is a no-op, not an error — an operator who runs
/// `kx migrate` defensively after every upgrade must not be punished for it.
#[test]
fn an_already_current_journal_is_a_no_op() {
    let tmp = tempfile::tempdir().unwrap();
    let path = journal_stamped_at(tmp.path(), JOURNAL_SCHEMA_VERSION);
    migrate(&path, None, false).expect("already-current is not an error");
    assert!(!PathBuf::from(format!("{}.v{JOURNAL_SCHEMA_VERSION}.bak", path.display())).exists());
}

/// The boot refusal NAMES the remedy.
///
/// This is the operator-facing half of the whole rider: `kx serve` failing on an old
/// journal is the moment someone needs to be told `kx migrate` exists. A refusal that
/// only reports the mismatch sends them looking for a workaround, and the obvious
/// workaround — delete the journal — destroys the run.
#[test]
fn the_schema_mismatch_refusal_points_at_the_migrate_verb() {
    let tmp = tempfile::tempdir().unwrap();
    let path = journal_stamped_at(tmp.path(), JOURNAL_SCHEMA_VERSION - 1);

    let Err(err) = kx_journal::SqliteJournal::open(&path) else {
        panic!("an old journal is refused");
    };
    let msg = err.to_string();
    assert!(
        msg.contains("kx migrate"),
        "the refusal must name the remedy, not just the problem: {msg}"
    );

    // ...and a NEWER journal must NOT be told to migrate, because it cannot be.
    let ndir = tmp.path().join("n");
    std::fs::create_dir_all(&ndir).unwrap();
    let newer = journal_stamped_at(&ndir, JOURNAL_SCHEMA_VERSION + 1);
    let Err(err) = kx_journal::SqliteJournal::open(&newer) else {
        panic!("a newer journal is refused");
    };
    let msg = err.to_string();
    assert!(
        !msg.contains("kx migrate"),
        "a newer journal has no migration remedy and must not be sent to one: {msg}"
    );
    assert!(msg.contains("NEWER"), "it says why instead: {msg}");
}

/// A journal from a NEWER binary is refused, and the file is not touched.
#[test]
fn a_newer_journal_is_refused_not_downgraded() {
    let tmp = tempfile::tempdir().unwrap();
    let newer = JOURNAL_SCHEMA_VERSION + 1;
    let path = journal_stamped_at(tmp.path(), newer);

    let err = migrate(&path, None, false).expect_err("a newer journal cannot be migrated down");
    assert!(
        err.contains("newer") || err.contains("NEWER") || err.contains("DOWN"),
        "the refusal explains why: {err}"
    );
    assert_eq!(
        journal_schema_version(&path).unwrap(),
        newer,
        "the refused journal is unchanged"
    );
}
