//! Concurrency + exactly-once proofs. The orchestration core serializes all
//! writes (single owner thread), so these assert the product's central promise:
//! under concurrent workers, an effect commits exactly once.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::BTreeSet;

use kx_coordinator::proto::{CommitOutcome, SubmitStatus};
use kx_coordinator::CoordinatorService;
use kx_journal::InMemoryJournal;
use kx_mote::NdClass;

fn coordinator() -> CoordinatorService {
    CoordinatorService::new(InMemoryJournal::new())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_registration_assigns_distinct_ids() {
    let svc = coordinator();
    let mut handles = Vec::new();
    for i in 0..32u32 {
        let s = svc.clone();
        handles.push(tokio::spawn(async move {
            common::register(&s, &format!("w{i}")).await
        }));
    }
    let mut ids = Vec::new();
    for h in handles {
        ids.push(h.await.unwrap());
    }
    let unique: BTreeSet<u64> = ids.iter().copied().collect();
    assert_eq!(unique.len(), 32, "every worker got a distinct id");
    assert_eq!(svc.registry().len(), 32);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_commits_of_distinct_motes_all_land() {
    let svc = coordinator();
    let warrant = common::sample_warrant();
    let worker = common::register(&svc, "w").await;

    let motes: Vec<_> = (1..=32u8)
        .map(|seed| common::mote(seed, NdClass::Pure, &[]))
        .collect();
    for m in &motes {
        common::submit(&svc, m, &warrant).await;
    }

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
    assert_eq!(svc.committed_count().await.unwrap(), 32);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_duplicate_commit_of_same_mote_is_exactly_once() {
    let svc = coordinator();
    let warrant = common::sample_warrant();
    let worker = common::register(&svc, "w").await;

    let mote = common::pure_root_mote();
    common::submit(&svc, &mote, &warrant).await;

    let mut handles = Vec::new();
    for _ in 0..32 {
        let s = svc.clone();
        let m = mote.clone();
        handles.push(tokio::spawn(
            async move { common::commit(&s, &m, worker).await },
        ));
    }

    let mut newly_committed = 0;
    let mut already = 0;
    let mut seqs = BTreeSet::new();
    for h in handles {
        let resp = h.await.unwrap();
        seqs.insert(resp.committed_seq);
        if resp.outcome == CommitOutcome::Committed as i32 {
            newly_committed += 1;
        } else if resp.outcome == CommitOutcome::AlreadyCommitted as i32 {
            already += 1;
        }
    }
    assert_eq!(newly_committed, 1, "exactly one report newly committed");
    assert_eq!(already, 31, "the rest were dedup-by-key hits");
    assert_eq!(seqs.len(), 1, "all reports observe the same committed seq");
    assert_eq!(svc.committed_count().await.unwrap(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_duplicate_submit_is_idempotent() {
    let svc = coordinator();
    let warrant = common::sample_warrant();
    let mote = common::pure_root_mote();

    let mut handles = Vec::new();
    for _ in 0..32 {
        let s = svc.clone();
        let m = mote.clone();
        let w = warrant.clone();
        handles.push(tokio::spawn(
            async move { common::submit(&s, &m, &w).await },
        ));
    }

    let mut accepted = 0;
    let mut duplicate = 0;
    for h in handles {
        let resp = h.await.unwrap();
        if resp.status == SubmitStatus::Accepted as i32 {
            accepted += 1;
        } else if resp.status == SubmitStatus::Duplicate as i32 {
            duplicate += 1;
        }
        // The re-derived id is identical on every concurrent submit.
        assert_eq!(resp.mote_id, mote.id.as_bytes().to_vec());
    }
    assert_eq!(accepted, 1, "exactly one submission accepted");
    assert_eq!(duplicate, 31);
    assert_eq!(svc.committed_count().await.unwrap(), 0);
}
