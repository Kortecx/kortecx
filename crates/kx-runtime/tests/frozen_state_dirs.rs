// Integration-test: state dirs a RELEASED binary actually wrote still open.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]
//! Boot the frozen state dirs in `tests/fixtures/state-dirs/` with current code.
//!
//! ## Why these fixtures and not a synthesized one
//!
//! `schema_evolution.rs` builds its old-version journal by writing a CURRENT one
//! and downgrading it with raw SQL. A fixture made that way can only contain what
//! today's WRITER knows how to produce, so it structurally cannot catch the class
//! of defect where an old binary wrote something today's READER does not expect.
//!
//! Two boot-path defects shipped past 469 green suites for exactly that reason:
//! no fixture in the tree had a pre-existing `bodies.db` or a pre-existing
//! sidecar. The suites were green because there was nothing old to be wrong
//! about.
//!
//! These directories were written by the published `v0.1.1` and `v0.2.0-rc.1`
//! binaries, sha256-verified against each release's own `checksums.txt` before
//! being executed. See `fixtures/state-dirs/SPEC.md` for provenance and the
//! recapture procedure.
//!
//! ## What is asserted, and what deliberately is not
//!
//! These tests assert that a real old state dir OPENS, MIGRATES with its product
//! identity intact, and still CARRIES the rows a user authored. They do not
//! assert the digest equals a frozen constant: the fixtures were captured once
//! and their digest is a property of that capture, not of the runtime. Pinning it
//! would turn any legitimate recapture into a mystery failure.

use std::path::{Path, PathBuf};

use kx_journal::{Journal, ReplayJournal, SqliteJournal, JOURNAL_SCHEMA_VERSION};
use kx_runtime::{digest_journal, migrate_and_verify};
use rusqlite::Connection;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/state-dirs")
}

/// Copy a fixture to a temp dir. The originals are read-only truth; a test that
/// migrated them in place would consume the thing it exists to protect.
fn scratch(tag: &str, tmp: &Path) -> PathBuf {
    let src = fixtures().join(tag);
    assert!(
        src.is_dir(),
        "{} is missing — see fixtures/state-dirs/SPEC.md",
        src.display()
    );
    let dst = tmp.join(tag);
    copy_tree(&src, &dst);
    dst
}

fn copy_tree(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let to = dst.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &to);
        } else {
            std::fs::copy(entry.path(), &to).unwrap();
        }
    }
}

/// The `schema_version` an old journal stamped in its own `metadata` table.
fn journal_version(path: &Path) -> u16 {
    let conn = Connection::open(path).unwrap();
    let raw: Vec<u8> = conn
        .query_row(
            "SELECT value FROM metadata WHERE key = 'schema_version'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    u16::from_le_bytes([raw[0], raw[1]])
}

fn table_rows(db: &Path, table: &str) -> i64 {
    let conn = Connection::open(db).unwrap();
    conn.query_row(&format!("SELECT COUNT(*) FROM \"{table}\""), [], |r| {
        r.get(0)
    })
    .unwrap()
}

/// Both fixtures are genuinely OLDER than the current schema.
///
/// Without this the migration tests below could pass by doing nothing: a fixture
/// that had drifted to the current version would "migrate" as a no-op and every
/// assertion would still hold. This is the anti-vacuity floor for the whole file.
#[test]
fn the_fixtures_are_actually_old() {
    let v011 = journal_version(&fixtures().join("v0.1.1/kx.db"));
    let v020 = journal_version(&fixtures().join("v0.2.0-rc.1/kx.db"));

    assert_eq!(v011, 8, "v0.1.1 wrote journal v8");
    assert_eq!(v020, 16, "v0.2.0-rc.1 wrote journal v16");
    assert!(
        v011 < JOURNAL_SCHEMA_VERSION && v020 < JOURNAL_SCHEMA_VERSION,
        "both fixtures must predate the current schema v{JOURNAL_SCHEMA_VERSION}; \
         got v{v011} and v{v020}. If the current version is now one of these, these \
         fixtures have stopped testing an upgrade and a NEWER tag must be captured."
    );
    // And they are not the same version as each other, or the matrix is one row.
    assert_ne!(v011, v020);
}

/// A v8 journal a released binary wrote migrates, and identity survives.
#[test]
fn a_released_v0_1_1_journal_migrates_with_its_identity_intact() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = scratch("v0.1.1", tmp.path());
    let src = dir.join("kx.db");
    let dst = dir.join("migrated.kxjournal");

    // The up-converting reader must fold the ORIGINAL before anything is
    // rewritten — that fold is what the migration is verified against.
    let before = digest_journal(&ReplayJournal::open(&src).unwrap()).unwrap();

    let report = migrate_and_verify(&src, &dst).unwrap();
    assert_eq!(report.from_version, 8);
    assert_eq!(report.to_version, JOURNAL_SCHEMA_VERSION);

    let after = digest_journal(&SqliteJournal::open(&dst).unwrap()).unwrap();
    assert_eq!(
        before, after,
        "migrating a real released journal must preserve product identity"
    );

    // The run's facts are still there — a migration that produced an EMPTY
    // journal would also have a stable digest.
    let j = SqliteJournal::open(&dst).unwrap();
    assert_eq!(
        j.count_entries().unwrap(),
        3,
        "the captured run's three entries survive the migration"
    );
}

/// A v16 journal a released binary wrote migrates, and identity survives.
#[test]
fn a_released_v0_2_0_rc_1_journal_migrates_with_its_identity_intact() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = scratch("v0.2.0-rc.1", tmp.path());
    let src = dir.join("kx.db");
    let dst = dir.join("migrated.kxjournal");

    let before = digest_journal(&ReplayJournal::open(&src).unwrap()).unwrap();
    let report = migrate_and_verify(&src, &dst).unwrap();
    assert_eq!(report.from_version, 16);
    assert_eq!(report.to_version, JOURNAL_SCHEMA_VERSION);

    let after = digest_journal(&SqliteJournal::open(&dst).unwrap()).unwrap();
    assert_eq!(before, after);
    assert_eq!(
        SqliteJournal::open(&dst).unwrap().count_entries().unwrap(),
        3
    );
}

/// The v16 fixture's AUTHORED CATALOG is real, and this is what the sidecar
/// upgrade policy has to preserve.
///
/// A migration that kept the journal and lost the App would satisfy every
/// assertion above. These rows are the things a user made and cannot regenerate.
#[test]
fn the_released_catalog_carries_the_entities_a_user_authored() {
    let catalog = fixtures().join("v0.2.0-rc.1/catalog");

    assert_eq!(table_rows(&catalog.join("apps.db"), "apps"), 1);
    assert_eq!(table_rows(&catalog.join("triggers.db"), "triggers"), 1);
    assert_eq!(table_rows(&catalog.join("branches.db"), "branches"), 1);
    assert_eq!(table_rows(&catalog.join("bundles.db"), "bundles"), 1);

    // The App is the one the SPEC names, not just "some row".
    let conn = Connection::open(catalog.join("apps.db")).unwrap();
    let handle: String = conn
        .query_row("SELECT handle FROM apps", [], |r| r.get(0))
        .unwrap();
    assert_eq!(handle, "fixtures/frozen/reporter");
}

/// **The claim this fixture exists to settle.**
///
/// The previous PR moved `tools.db` under the sidecar upgrade policy and argued
/// in its CHANGELOG that existing databases would be unaffected *because* the old
/// opener stamped its version in a table called `metadata` while the policy looks
/// in `meta` — so the policy finds nothing, takes the fresh-file arm, and every
/// statement is `CREATE TABLE IF NOT EXISTS`.
///
/// That was an argument about a file nobody had. This is the file.
#[test]
fn a_released_tools_db_really_does_use_the_metadata_table() {
    let tools = fixtures().join("v0.2.0-rc.1/catalog/tools.db");
    let conn = Connection::open(&tools).unwrap();
    let tables: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();

    assert!(
        tables.iter().any(|t| t == "metadata"),
        "the released tools.db stamps its version in `metadata`; found {tables:?}"
    );
    assert!(
        !tables.iter().any(|t| t == "meta"),
        "the released tools.db has NO `meta` table — that absence is precisely why \
         the upgrade policy takes the fresh-file arm and leaves the rows alone. \
         Found {tables:?}"
    );
    // And it has rows to lose, so the claim is about something.
    assert!(table_rows(&tools, "tools") > 0);
}

/// A fixture with a host path baked in works only on the machine that made it.
///
/// Scanning here rather than trusting the capture-time scan: a recapture on a
/// different machine, or a hand-edit, would reintroduce it silently, and the
/// symptom would be a Linux-only CI failure long after the cause.
#[test]
fn no_fixture_carries_an_absolute_host_path() {
    let mut offenders: Vec<String> = Vec::new();
    for tag in ["v0.1.1", "v0.2.0-rc.1"] {
        walk(&fixtures().join(tag), &mut |p| {
            let bytes = std::fs::read(p).unwrap();
            for needle in [b"/Users/".as_slice(), b"/private/tmp/".as_slice()] {
                if bytes.windows(needle.len()).any(|w| w == needle) {
                    offenders.push(p.display().to_string());
                    return;
                }
            }
        });
    }
    assert!(
        offenders.is_empty(),
        "these fixtures carry an absolute host path and will not boot elsewhere: \
         {offenders:?}"
    );
}

fn walk(dir: &Path, f: &mut impl FnMut(&Path)) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let p = entry.path();
        if entry.file_type().unwrap().is_dir() {
            walk(&p, f);
        } else if p.extension().is_some_and(|e| e == "db") {
            f(&p);
        }
    }
}
