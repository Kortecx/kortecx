// Integration-test file: compiled as a separate crate from the host lib; tests
// legitimately use `.unwrap()` for fixture construction.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]
//! IMP-4 (D116) — single-writer **realistic end-to-end** commit ceiling (number iii).
//!
//! `kx-journal/tests/ceiling_throughput.rs` measures the raw `Journal` write floor/ceiling
//! in isolation. This file measures the number the roadmap actually cares about: the
//! coordinator's sole-writer path under maximum simultaneous fan-out — many workers
//! reporting commits at once, all funneling through the single owner thread. That path
//! already does **group commit** (the corpus contingency "design group-commit if too low"
//! is already built): `core_loop` drains up to `MAX_DRAIN=256` `Command`s per wake from the
//! bounded `mpsc` channel (`COMMAND_BUFFER=1024`) and `flush_commits`/`apply_batch` coalesce
//! consecutive `Commit`s into ONE atomic `journal.append_batch()` + fold. So the on-disk
//! number here is the group-commit ceiling end-to-end, including the channel + `oneshot`
//! reply + fold overhead a real worker pays.
//!
//! **Non-gating** (testing doctrine §Load/throughput): `#[ignore]`, prints commits/s, asserts
//! only a loose catastrophic-regression floor + the exactly-once correctness count
//! (`committed_count == n`) — never an absolute-time threshold.
//!
//! Run via `just bench-ceiling` (or directly):
//! `cargo test -p kx-coordinator --release --test ceiling_e2e -- --ignored --nocapture --test-threads=1`
//! `KX_CEILING_HUGE=1` adds the 10^6 in-memory tier (local only).
//!
//! Caveats for the published number: it includes tokio multi-thread scheduling + the channel
//! hops, so present it *beside* the raw `append_batch` ceiling (ii) to see how much of the gap
//! is fsync-amortization vs runtime overhead. On-disk numbers are platform-sensitive (macOS
//! `fsync` is weaker than Linux) — label them with their environment.

mod common;

use std::time::Instant;

use kx_coordinator::proto;
use kx_coordinator::proto::coordinator_server::Coordinator;
use kx_coordinator::CoordinatorService;
use kx_journal::SqliteJournal;
use kx_mote::{Mote, NdClass};
use kx_warrant::WarrantSpec;
use tempfile::tempdir;
use tonic::Request;

/// Register the run once, then submit all `motes` (untimed setup). Avoids the
/// per-call re-registration `common::submit` does so setup of 10^5 Motes stays cheap.
async fn submit_all(svc: &CoordinatorService, motes: &[Mote], warrant: &WarrantSpec) {
    common::register_run(svc, common::TEST_RECIPE_FINGERPRINT).await;
    for m in motes {
        svc.submit_mote(Request::new(proto::SubmitMoteRequest {
            mote: Some(m.clone().into()),
            warrant: Some(warrant.clone().into()),
            accept_at_least_once: false,
        }))
        .await
        .unwrap();
    }
}

/// Fan out a `report_commit` per Mote across concurrent tasks (the owner thread
/// coalesces them into group commits), join all, and return the timed wall clock
/// of just the commit phase.
async fn time_concurrent_commits(svc: &CoordinatorService, motes: &[Mote], worker: u64) -> f64 {
    let start = Instant::now();
    let mut handles = Vec::with_capacity(motes.len());
    for m in motes {
        let s = svc.clone();
        let req = common::report_commit_request(m, worker);
        handles.push(tokio::spawn(async move {
            s.report_commit(Request::new(req)).await
        }));
    }
    for h in handles {
        h.await.unwrap().unwrap();
    }
    start.elapsed().as_secs_f64()
}

fn sizes(include_huge: bool) -> Vec<u64> {
    if include_huge && std::env::var_os("KX_CEILING_HUGE").is_some() {
        vec![10_000, 100_000, 1_000_000]
    } else {
        vec![10_000, 100_000]
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "scale: just bench-ceiling (cargo test -p kx-coordinator --release --test ceiling_e2e -- --ignored --nocapture)"]
async fn coordinator_concurrent_ceiling_on_disk() {
    eprintln!(
        "=== (iii) coordinator concurrent commit ceiling — on-disk (group-commit, fsync/batch) ==="
    );
    let warrant = common::sample_warrant();
    // On-disk capped at 10^5 (a 10^6 on-disk journal is hundreds of MB on disk + WAL).
    for &n in &sizes(false) {
        let dir = tempdir().unwrap();
        let svc = CoordinatorService::new(SqliteJournal::open(dir.path().join("e2e.db")).unwrap());
        let worker = common::register(&svc, "w").await;
        let motes: Vec<Mote> = (0..n)
            .map(|i| common::mote_indexed(i, NdClass::Pure))
            .collect();
        submit_all(&svc, &motes, &warrant).await;

        let secs = time_concurrent_commits(&svc, &motes, worker).await;

        assert_eq!(
            svc.committed_count().await.unwrap(),
            n as usize,
            "every distinct Mote committed exactly once"
        );
        let rate = n as f64 / secs;
        eprintln!("  n={n:>9}  {secs:>10.3}s  {rate:>12.0} commits/s");
        assert!(
            rate > 100.0,
            "e2e on-disk below 100 commits/s ({rate:.0}) — catastrophic regression"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "scale: just bench-ceiling (cargo test -p kx-coordinator --release --test ceiling_e2e -- --ignored --nocapture)"]
async fn coordinator_concurrent_ceiling_in_memory() {
    eprintln!("=== (iii) coordinator concurrent commit ceiling — in-memory (no fsync; channel + fold only) ===");
    let warrant = common::sample_warrant();
    for &n in &sizes(true) {
        let svc = CoordinatorService::new(SqliteJournal::open_in_memory().unwrap());
        let worker = common::register(&svc, "w").await;
        let motes: Vec<Mote> = (0..n)
            .map(|i| common::mote_indexed(i, NdClass::Pure))
            .collect();
        submit_all(&svc, &motes, &warrant).await;

        let secs = time_concurrent_commits(&svc, &motes, worker).await;

        assert_eq!(
            svc.committed_count().await.unwrap(),
            n as usize,
            "exactly-once"
        );
        let rate = n as f64 / secs;
        eprintln!("  n={n:>9}  {secs:>10.3}s  {rate:>12.0} commits/s");
        assert!(
            rate > 100.0,
            "e2e in-memory below 100 commits/s ({rate:.0}) — catastrophic regression"
        );
    }
}
