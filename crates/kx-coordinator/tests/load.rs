//! Throughput / load. These run over the production `SqliteJournal` backend (the
//! in-memory variant) so the journal's dedup + read paths are indexed (O(log n)),
//! and assert the coordinator scales **linearly** — the incremental projection
//! fold must not regress to O(n²). Bounds are generous (machine-independent); the
//! point is to catch a pathological regression, not to benchmark.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::time::Instant;

use kx_coordinator::proto::CommitOutcome;
use kx_coordinator::CoordinatorService;
use kx_journal::SqliteJournal;
use kx_mote::NdClass;

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
