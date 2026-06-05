//! Clean-run integration: the canonical workflow runs end-to-end through the
//! on-disk journal + content store, is deterministic across runs/processes, and
//! replaying a completed journal is a no-op. (Crash scenarios abort the process
//! and live in the `kx-p1-demo` subprocess harness, not here.)

#![allow(clippy::unwrap_used, clippy::expect_used)]

use kx_runtime::config::Mode;
use kx_runtime::{engine, RuntimeConfig};

fn cfg(dir: &std::path::Path, mode: Mode) -> RuntimeConfig {
    RuntimeConfig {
        journal_path: dir.join("journal.sqlite"),
        content_root: dir.join("content"),
        mode,
        crash_at: None,
        // Cadence 2 over the 8-Mote demo: the clean run writes real checkpoint
        // sidecars, and the "replay is a no-op" case then exercises the
        // seeded-recovery (happy) path — recovery stays bit-identical.
        checkpoint_every: Some(2),
        audit_log: None,
    }
}

#[test]
fn clean_run_commits_every_mote() {
    let dir = tempfile::tempdir().unwrap();
    let outcome = engine::run(&cfg(dir.path(), Mode::Run)).unwrap();
    assert!(outcome.is_complete(), "every workflow Mote must commit");
    // 6 declared Motes (M1, shaper, M2, Wstc, M3, M3c) + 2 materialized workers.
    assert_eq!(outcome.committed, 8);
    assert_eq!(outcome.total, 8);
}

#[test]
fn run_is_deterministic_across_independent_runs() {
    let d1 = tempfile::tempdir().unwrap();
    let d2 = tempfile::tempdir().unwrap();
    let o1 = engine::run(&cfg(d1.path(), Mode::Run)).unwrap();
    let o2 = engine::run(&cfg(d2.path(), Mode::Run)).unwrap();
    assert_eq!(
        o1.digest, o2.digest,
        "two independent clean runs must produce a bit-identical committed-result set"
    );
}

#[test]
fn fresh_process_digest_matches_live_projection() {
    // Assertion (c) surface: a fresh handle that has only the on-disk journal
    // reconstructs the same digest the live run reported.
    let dir = tempfile::tempdir().unwrap();
    let config = cfg(dir.path(), Mode::Run);
    let live = engine::run(&config).unwrap();
    let reread = engine::digest_only(&config).unwrap();
    assert_eq!(live.digest, reread);
}

#[test]
fn replay_of_completed_run_is_a_noop() {
    let dir = tempfile::tempdir().unwrap();
    let run = engine::run(&cfg(dir.path(), Mode::Run)).unwrap();
    // Replaying the already-complete journal must not commit anything new and
    // must reproduce the identical digest (every Mote re-read, none re-run).
    let replay = engine::run(&cfg(dir.path(), Mode::Replay)).unwrap();
    assert!(replay.is_complete());
    assert_eq!(run.digest, replay.digest);
}

#[test]
fn workflow_mote_ids_are_stable() {
    // The kill-and-replay harness depends on stable ids to target the crash.
    let a: Vec<_> = engine::canonical_mote_ids();
    let b: Vec<_> = engine::canonical_mote_ids();
    assert_eq!(a, b);
    assert_eq!(a.len(), 6);
    let (stc, vtc) = engine::canonical_targets();
    assert!(a.contains(&stc) && a.contains(&vtc));
}
