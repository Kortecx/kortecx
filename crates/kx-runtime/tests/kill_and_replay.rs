//! **The P1 exit-gate proof — kill-and-replay.**
//!
//! Spawns the real `kx-runtime` binary as a subprocess, kills it with a hard
//! `process::abort` (SIGABRT) at a precise window over an on-disk SQLite
//! journal, then restarts a **fresh process** that recovers by replaying the
//! journal. Two scenarios prove exactly-once + recover-by-re-read:
//!
//! - **Scenario 1 — `pre-commit-stc`:** crash mid `StageThenCommit` (after
//!   `EffectStaged` + broker stage, before `Committed`). Recovery re-dispatches;
//!   idempotency-key dedup makes the external effect exactly-once.
//! - **Scenario 2 — `post-commit-vtc`:** crash the instant the
//!   `ValidateThenCommit` Mote's `Committed` is durable. Recovery RE-READS the
//!   committed result, never re-running the effect — the headline novel claim.
//!
//! Each scenario asserts the three exit-gate properties:
//! - **(a)** the recovered committed-result set is bit-identical to a clean run;
//! - **(b)** no Mote has more than one `Committed` entry;
//! - **(c)** a fresh process folding only the journal reconstructs a
//!   bit-identical projection digest ("different machine" replay).
//!
//! The harness lives in the binary's own crate so the binary path resolves via
//! `CARGO_BIN_EXE_kx-runtime` (robust across debug/release + target dirs) — the
//! idiomatic Rust way to drive a binary under test.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::doc_markdown)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use kx_journal::{Journal, JournalEntry, SqliteJournal};
use kx_mote::MoteId;

const BIN: &str = env!("CARGO_BIN_EXE_kx-runtime");

struct Paths {
    _dir: tempfile::TempDir,
    journal: PathBuf,
    content: PathBuf,
}

fn paths() -> Paths {
    let dir = tempfile::tempdir().unwrap();
    let journal = dir.path().join("journal.sqlite");
    let content = dir.path().join("content");
    Paths {
        _dir: dir,
        journal,
        content,
    }
}

fn invoke(mode: &str, p: &Paths, crash_at: Option<&str>) -> Output {
    let mut cmd = Command::new(BIN);
    cmd.arg(mode)
        .arg("--journal")
        .arg(&p.journal)
        .arg("--content")
        .arg(&p.content);
    if let Some(c) = crash_at {
        cmd.arg("--crash-at").arg(c);
    }
    cmd.output().expect("spawn kx-runtime")
}

/// The leading whitespace-delimited token of stdout is the hex digest (both
/// `run`/`replay` — "<hex> (n/m committed)" — and `digest` — "<hex>").
fn digest_of(out: &Output) -> String {
    assert!(
        out.status.success(),
        "expected success, got {:?}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    stdout
        .split_whitespace()
        .next()
        .expect("digest token")
        .to_string()
}

/// Clean reference digest from an independent run in its own temp dir.
fn clean_reference_digest() -> String {
    let p = paths();
    digest_of(&invoke("run", &p, None))
}

/// Count `Committed` entries per Mote in the on-disk journal — assertion (b).
///
/// The range end is `current_seq + 1`, NOT `u64::MAX`: SQLite binds `u64::MAX` to `-1`
/// (i64), which silently returns **zero rows** (the bind quirk recorded in HANDOFF
/// §2.55). Reading the real watermark makes every caller's count meaningful — without
/// this, `assert_exactly_once` would pass vacuously over an empty map.
fn committed_counts(journal_path: &Path) -> BTreeMap<MoteId, usize> {
    let journal = SqliteJournal::open(journal_path).unwrap();
    let end = journal.current_seq().unwrap().saturating_add(1);
    let mut counts: BTreeMap<MoteId, usize> = BTreeMap::new();
    for entry in journal.read_entries_by_seq(0..end).unwrap() {
        if let JournalEntry::Committed { mote_id, .. } = entry {
            *counts.entry(mote_id).or_insert(0) += 1;
        }
    }
    counts
}

fn assert_exactly_once(journal_path: &Path) {
    for (mote_id, count) in committed_counts(journal_path) {
        assert_eq!(
            count, 1,
            "Mote {mote_id:?} has {count} Committed entries; exactly-once requires exactly 1"
        );
    }
}

/// On Unix a `process::abort` death reports via signal (no clean exit code).
fn assert_aborted(out: &Output) {
    assert!(
        !out.status.success(),
        "crash run must NOT exit cleanly; got {:?}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        assert!(
            out.status.signal().is_some(),
            "crash run must die by signal (SIGABRT), got code {:?}",
            out.status.code()
        );
    }
}

#[test]
fn scenario_1_stage_then_commit_pre_commit_crash_recovers_exactly_once() {
    let reference = clean_reference_digest();
    let p = paths();

    // Kill mid StageThenCommit (after EffectStaged + stage, before Committed).
    let crashed = invoke("run", &p, Some("pre-commit-stc"));
    assert_aborted(&crashed);

    // The journal carries the in-flight prefix but no Committed for the target.
    let mid = committed_counts(&p.journal);
    assert!(
        mid.values().all(|&c| c == 1),
        "no duplicate Committed even mid-crash"
    );

    // Restart: a fresh process recovers by replaying the journal.
    let replay = invoke("replay", &p, None);
    let replay_digest = digest_of(&replay);

    // (a) recovered committed-result set is bit-identical to a clean run.
    assert_eq!(replay_digest, reference, "(a) bit-identical committed set");
    // (b) no Mote committed more than once.
    assert_exactly_once(&p.journal);
    // (c) a fresh process folding only the journal reconstructs the same digest.
    let fresh = digest_of(&invoke("digest", &p, None));
    assert_eq!(fresh, reference, "(c) cross-process replay digest");
}

#[test]
fn scenario_2_validate_then_commit_post_commit_crash_rereads_not_reruns() {
    let reference = clean_reference_digest();
    let p = paths();

    // Kill the instant the VTC Mote's Committed is durable.
    let crashed = invoke("run", &p, Some("post-commit-vtc"));
    assert_aborted(&crashed);

    // The VTC Mote is already Committed exactly once at crash time.
    let mid = committed_counts(&p.journal);
    assert!(
        mid.values().all(|&c| c == 1),
        "the committed WM Mote appears exactly once even at crash time"
    );

    // Restart: recovery RE-READS the committed Mote (never re-runs its effect).
    let replay = invoke("replay", &p, None);
    let replay_digest = digest_of(&replay);

    // (a) (b) (c)
    assert_eq!(replay_digest, reference, "(a) bit-identical committed set");
    assert_exactly_once(&p.journal);
    let fresh = digest_of(&invoke("digest", &p, None));
    assert_eq!(fresh, reference, "(c) cross-process replay digest");
}

/// Scenario 3 — `shaper-children-pending` (P0.6 / P3.4, the hardest path): crash the
/// instant the topology shaper has committed its `TopologyDecision` and every *declared*
/// Mote is committed, but the **materialized children** have not yet run. Recovery must
/// REPLAY the committed decision — re-materialize the SAME children (D49, identity from
/// journal facts) and run them — NEVER re-run the shaper to re-decide (which would orphan
/// or duplicate children). Without re-deriving the committed shaper's children before the
/// drive loop can stop, a fresh process would break with the children orphaned.
#[test]
fn scenario_3_shaper_commits_then_crash_replays_decision_not_re_decided() {
    let reference = clean_reference_digest();
    let p = paths();

    // Crash after the shaper + every declared Mote committed, before the children run.
    let crashed = invoke("run", &p, Some("shaper-children-pending"));
    assert_aborted(&crashed);

    // At crash time: only the declared Motes are committed (exactly once each); the two
    // materialized children have NOT committed yet.
    let mid = committed_counts(&p.journal);
    assert!(
        mid.values().all(|&c| c == 1),
        "every committed Mote appears exactly once at crash time"
    );
    let committed_at_crash = mid.len();
    assert_eq!(
        committed_at_crash, 6,
        "the 6 declared Motes (incl. the shaper) committed; the 2 children are pending"
    );

    // Restart: recovery re-folds the committed shaper, re-materializes the SAME children,
    // and runs them — the shaper is never re-run (its committed decision is a fact).
    let replay = invoke("replay", &p, None);
    let replay_digest = digest_of(&replay);

    // (a) bit-identical committed set: the replayed decision yields the same children +
    //     the same final committed set as a clean run.
    assert_eq!(
        replay_digest, reference,
        "(a) recovery replays the committed decision: bit-identical committed set"
    );
    // (b) no Mote (shaper or child) committed more than once — the shaper did not
    //     re-decide, the children were not duplicated.
    assert_exactly_once(&p.journal);
    // (c) cross-process replay digest.
    let fresh = digest_of(&invoke("digest", &p, None));
    assert_eq!(fresh, reference, "(c) cross-process replay digest");

    // The previously-pending children DID materialize + commit on recovery (8 total now,
    // up from 6 at crash) — they were not orphaned.
    let after = committed_counts(&p.journal);
    assert_eq!(
        after.len(),
        8,
        "recovery ran the 2 previously-pending materialized children (6 → 8 committed)"
    );
}

#[test]
fn clean_run_via_binary_completes() {
    let p = paths();
    let out = invoke("run", &p, None);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success());
    assert!(
        stdout.contains("(8/8 committed)"),
        "clean run must commit all 8 Motes (6 declared + 2 materialized workers); got: {stdout}"
    );
}
