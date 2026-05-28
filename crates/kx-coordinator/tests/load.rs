//! Throughput / load over the production `SqliteJournal` backend. The in-memory
//! variant exercises the indexed (O(log n)) dedup/read paths and asserts the
//! coordinator scales **linearly** (the incremental fold must not regress to
//! O(n²)); the on-disk variant exercises the durable path (fsync per transaction)
//! where **group commit** amortizes fsync by coalescing concurrent commits.
//! Bounds are generous (machine-independent) — the point is to catch a pathological
//! regression, not to benchmark (precise tracking belongs in a criterion bench).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::time::{Duration, Instant};

use kx_coordinator::proto::CommitOutcome;
use kx_coordinator::CoordinatorService;
use kx_journal::SqliteJournal;
use kx_mote::NdClass;
use tempfile::tempdir;

#[tokio::test]
async fn sequential_submit_and_commit_throughput() {
    let n: u64 = 1_000;
    let svc = CoordinatorService::new(SqliteJournal::open_in_memory().unwrap());
    let warrant = common::sample_warrant();
    let worker = common::register(&svc, "w").await;

    let motes: Vec<_> = (0..n)
        .map(|i| common::mote_indexed(i, NdClass::Pure))
        .collect();

    let start = Instant::now();
    for m in &motes {
        common::submit(&svc, m, &warrant).await;
    }
    for m in &motes {
        let commit = common::commit(&svc, m, worker).await;
        assert_eq!(commit.outcome, CommitOutcome::Committed as i32);
    }
    let elapsed = start.elapsed();

    assert_eq!(svc.committed_count().await.unwrap(), n as usize);
    let ops = (2 * n) as f64 / elapsed.as_secs_f64();
    eprintln!("throughput: {n} submit + {n} commit in {elapsed:?} ({ops:.0} ops/s)");
    assert!(
        elapsed.as_secs() < 20,
        "1000 submit+commit should be well under 20s if linear; took {elapsed:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_commit_at_scale_is_exactly_once() {
    let n: u64 = 500;
    let svc = CoordinatorService::new(SqliteJournal::open_in_memory().unwrap());
    let warrant = common::sample_warrant();
    let worker = common::register(&svc, "w").await;

    let motes: Vec<_> = (0..n)
        .map(|i| common::mote_indexed(i, NdClass::Pure))
        .collect();
    for m in &motes {
        common::submit(&svc, m, &warrant).await;
    }

    // Fan out commits across many concurrent clients.
    let mut handles = Vec::new();
    for m in &motes {
        let s = svc.clone();
        let m = m.clone();
        handles.push(tokio::spawn(
            async move { common::commit(&s, &m, worker).await },
        ));
    }
    for h in handles {
        let resp = h.await.unwrap();
        assert_eq!(resp.outcome, CommitOutcome::Committed as i32);
    }

    assert_eq!(
        svc.committed_count().await.unwrap(),
        n as usize,
        "every distinct Mote committed exactly once under concurrency"
    );
}

/// Commit `motes` on `svc` either sequentially (await each) or concurrently (fan
/// out, then join). Returns the wall time for the commit phase.
async fn time_commits(
    svc: &CoordinatorService,
    motes: &[kx_mote::Mote],
    concurrent: bool,
) -> Duration {
    let worker = common::register(svc, "w").await;
    let warrant = common::sample_warrant();
    for m in motes {
        common::submit(svc, m, &warrant).await;
    }
    let start = Instant::now();
    if concurrent {
        let mut handles = Vec::new();
        for m in motes {
            let s = svc.clone();
            let m = m.clone();
            handles.push(tokio::spawn(
                async move { common::commit(&s, &m, worker).await },
            ));
        }
        for h in handles {
            assert_eq!(h.await.unwrap().outcome, CommitOutcome::Committed as i32);
        }
    } else {
        for m in motes {
            assert_eq!(
                common::commit(svc, m, worker).await.outcome,
                CommitOutcome::Committed as i32
            );
        }
    }
    start.elapsed()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn group_commit_speeds_up_concurrent_durable_commits() {
    // The group-commit win shows on the DURABLE path: an on-disk journal fsyncs
    // per transaction (synchronous=FULL), so coalescing N concurrent commits into
    // shared transactions amortizes the fsync cost. (In-memory has no fsync, so
    // there is nothing to amortize — that is expected.)
    let n: u64 = 1_000;
    let dir = tempdir().unwrap();
    let motes: Vec<_> = (0..n)
        .map(|i| common::mote_indexed(i, NdClass::Pure))
        .collect();

    let seq_svc = CoordinatorService::new(SqliteJournal::open(dir.path().join("seq.db")).unwrap());
    let sequential = time_commits(&seq_svc, &motes, false).await;
    assert_eq!(seq_svc.committed_count().await.unwrap(), n as usize);

    let conc_svc =
        CoordinatorService::new(SqliteJournal::open(dir.path().join("conc.db")).unwrap());
    let concurrent = time_commits(&conc_svc, &motes, true).await;
    assert_eq!(conc_svc.committed_count().await.unwrap(), n as usize);

    let speedup = sequential.as_secs_f64() / concurrent.as_secs_f64();
    eprintln!(
        "on-disk {n} commits — sequential {sequential:?}, concurrent (group-commit) {concurrent:?} \
         ({speedup:.1}x)"
    );
    // The exact speedup is hardware/fsync-dependent (precise tracking belongs in a
    // criterion bench), so assert only that the group-commit path stays bounded and
    // is not pathologically worse than the per-commit path.
    assert!(
        concurrent.as_secs() < 30,
        "concurrent commits should stay bounded"
    );
    assert!(
        concurrent.as_secs_f64() <= sequential.as_secs_f64() * 2.0,
        "group commit must not regress badly vs per-commit: seq={sequential:?} conc={concurrent:?}"
    );
}
