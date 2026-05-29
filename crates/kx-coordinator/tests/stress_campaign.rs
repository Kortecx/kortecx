//! Clean-slate stress / load harnesses (validation campaign, 2026-05-29).
//!
//! These are **additive** and gated behind `#[ignore]` so the default
//! `cargo test` and CI gates keep their existing semantics. Run explicitly:
//!
//! ```text
//! cargo test -p kx-coordinator --test stress_campaign -- --ignored --nocapture
//! ```
//!
//! They push the in-process coordinator harder than the existing `load.rs`:
//! a multi-thousand-Mote layered DAG, concurrent distinct+duplicate commits at
//! scale (exactly-once under contention), a wide poison-cascade blast radius, and
//! a repeated drop/reopen recovery storm over a durable journal. The point is to
//! surface pathological regressions and ordering bugs, not to benchmark.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::time::Instant;

use kx_coordinator::proto::CommitOutcome;
use kx_coordinator::{CoordinatorService, MoteState, RepudiationReason};
use kx_journal::{InMemoryJournal, SqliteJournal};
use kx_mote::{EdgeMeta, GraphPosition, InputDataId, Mote, MoteId, NdClass, ParentRef};
use smallvec::SmallVec;
use tempfile::tempdir;

/// A uniquely-identified Mote at `index` (u64, beyond the 256 `common::mote`
/// supports) with explicit data-edge `parents`. Identity is derived by
/// `Mote::new` from `def + input_data_id + graph_position`.
fn dag_mote(index: u64, nd: NdClass, parents: &[MoteId]) -> Mote {
    let mut input = [0u8; 32];
    input[..8].copy_from_slice(&index.to_le_bytes());
    let prefs: SmallVec<[ParentRef; 4]> = parents
        .iter()
        .map(|id| ParentRef {
            parent_id: *id,
            edge: EdgeMeta::data(),
        })
        .collect();
    Mote::new(
        common::mote_def(nd),
        InputDataId::from_bytes(input),
        GraphPosition(index.to_le_bytes().to_vec()),
        prefs,
    )
}

/// SCALE — a ~3,000-Mote layered DAG. Each non-root node depends on two nodes in
/// the previous layer (a fan-in/diamond lattice). Submit the whole graph, then
/// commit layer by layer in dependency order, asserting the ready-set advances
/// monotonically and every distinct Mote commits exactly once.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "stress: run with --ignored"]
async fn stress_large_layered_dag_commits_exactly_once() {
    const WIDTH: u64 = 50;
    const DEPTH: u64 = 60; // 3,000 Motes
    let svc = CoordinatorService::new(InMemoryJournal::new());
    let warrant = common::sample_warrant();
    let worker = common::register(&svc, "w").await;

    // Build layers: layer 0 is parentless; layer L node j depends on layer L-1
    // nodes j and (j+1)%WIDTH.
    let mut layers: Vec<Vec<Mote>> = Vec::with_capacity(DEPTH as usize);
    for l in 0..DEPTH {
        let mut layer = Vec::with_capacity(WIDTH as usize);
        for j in 0..WIDTH {
            let index = l * WIDTH + j;
            let parents: Vec<MoteId> = if l == 0 {
                vec![]
            } else {
                let prev = &layers[(l - 1) as usize];
                vec![prev[j as usize].id, prev[((j + 1) % WIDTH) as usize].id]
            };
            layer.push(dag_mote(index, NdClass::Pure, &parents));
        }
        layers.push(layer);
    }
    let total = (WIDTH * DEPTH) as usize;

    let start = Instant::now();
    for layer in &layers {
        for m in layer {
            let r = common::submit(&svc, m, &warrant).await;
            assert_eq!(
                r.status,
                kx_coordinator::proto::SubmitStatus::Accepted as i32
            );
        }
    }
    let submit_elapsed = start.elapsed();

    // Only layer 0 is ready initially.
    assert_eq!(svc.ready_set().await.unwrap().len(), WIDTH as usize);

    let commit_start = Instant::now();
    for layer in &layers {
        for m in layer {
            assert_eq!(
                common::commit(&svc, m, worker).await.outcome,
                CommitOutcome::Committed as i32
            );
        }
    }
    let commit_elapsed = commit_start.elapsed();

    assert_eq!(svc.committed_count().await.unwrap(), total);
    assert!(
        svc.ready_set().await.unwrap().is_empty(),
        "all work drained"
    );
    eprintln!("large-dag: {total} Motes — submit {submit_elapsed:?}, commit {commit_elapsed:?}");
    // Generous machine-independent ceiling: catches an O(n^2) fold regression.
    assert!(
        commit_elapsed.as_secs() < 60,
        "committing {total} Motes layer-by-layer should stay linear; took {commit_elapsed:?}"
    );
}

/// PARALLEL SATURATION — N distinct Motes, each committed by TWO concurrent
/// tasks (a duplicate race). Exactly-once must hold: every Mote ends Committed,
/// the journal dedupes the duplicate by idempotency key, and `committed_count`
/// equals N exactly (no double-write).
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "stress: run with --ignored"]
async fn stress_concurrent_distinct_and_duplicate_is_exactly_once() {
    const N: u64 = 1_500;
    let svc = CoordinatorService::new(InMemoryJournal::new());
    let warrant = common::sample_warrant();
    let worker = common::register(&svc, "w").await;

    let motes: Vec<Mote> = (0..N).map(|i| dag_mote(i, NdClass::Pure, &[])).collect();
    for m in &motes {
        common::submit(&svc, m, &warrant).await;
    }

    // Two racing committers per Mote.
    let mut handles = Vec::with_capacity((2 * N) as usize);
    for m in &motes {
        for _ in 0..2 {
            let s = svc.clone();
            let m = m.clone();
            handles.push(tokio::spawn(async move {
                common::commit(&s, &m, worker).await.outcome
            }));
        }
    }
    for h in handles {
        let outcome = h.await.unwrap();
        assert!(
            outcome == CommitOutcome::Committed as i32
                || outcome == CommitOutcome::AlreadyCommitted as i32,
            "each racing commit is either the winner or a dedup hit"
        );
    }

    assert_eq!(
        svc.committed_count().await.unwrap(),
        N as usize,
        "every distinct Mote committed exactly once despite duplicate races"
    );
}

/// BLAST RADIUS — a root with a wide fan-out of direct children (one layer).
/// Commit the whole star, repudiate the root, and assert the poison cascade
/// invalidates exactly the children and nothing remains committed.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "stress: run with --ignored"]
async fn stress_repudiation_blast_radius() {
    const FANOUT: u64 = 2_000;
    let svc = CoordinatorService::new(InMemoryJournal::new());
    let warrant = common::sample_warrant();
    let worker = common::register(&svc, "w").await;

    let root = dag_mote(0, NdClass::Pure, &[]);
    common::submit(&svc, &root, &warrant).await;
    common::commit(&svc, &root, worker).await;

    let children: Vec<Mote> = (1..=FANOUT)
        .map(|i| dag_mote(i, NdClass::Pure, &[root.id]))
        .collect();
    for c in &children {
        common::submit(&svc, c, &warrant).await;
        common::commit(&svc, c, worker).await;
    }
    assert_eq!(svc.committed_count().await.unwrap(), (FANOUT + 1) as usize);

    let start = Instant::now();
    let outcome = svc
        .repudiate(root.id, RepudiationReason::OperatorAction, 1)
        .await
        .unwrap();
    eprintln!(
        "repudiation: cascade of {} in {:?}",
        outcome.cascade_size,
        start.elapsed()
    );
    assert_eq!(
        outcome.cascade_size, FANOUT as usize,
        "all children cascaded"
    );
    assert_eq!(
        svc.committed_count().await.unwrap(),
        0,
        "the entire poisoned lineage is repudiated"
    );
    assert_eq!(svc.state_of(root.id).await.unwrap(), MoteState::Repudiated);
}

/// RECOVERY STORM — over a durable Sqlite journal, repeatedly commit a batch,
/// drop the coordinator (simulating a crash), and reopen a fresh one over the
/// same file. The recovered `committed_count` must equal the cumulative total
/// every round — the journal, not the process, is the source of truth.
#[tokio::test]
#[ignore = "stress: run with --ignored"]
async fn stress_recovery_storm_converges() {
    const ROUNDS: u64 = 20;
    const BATCH: u64 = 100;
    let dir = tempdir().unwrap();
    let path = dir.path().join("storm.db");
    let warrant = common::sample_warrant();

    let mut committed_so_far = 0u64;
    for round in 0..ROUNDS {
        let svc = CoordinatorService::new(SqliteJournal::open(&path).unwrap());
        // Recovery is implicit in `new`: the prior rounds' commits must be present.
        assert_eq!(
            svc.committed_count().await.unwrap(),
            committed_so_far as usize,
            "round {round}: recovered the cumulative committed state"
        );
        let worker = common::register(&svc, "w").await;
        for j in 0..BATCH {
            let m = dag_mote(round * BATCH + j, NdClass::Pure, &[]);
            common::submit(&svc, &m, &warrant).await;
            assert_eq!(
                common::commit(&svc, &m, worker).await.outcome,
                CommitOutcome::Committed as i32
            );
        }
        committed_so_far += BATCH;
        assert_eq!(
            svc.committed_count().await.unwrap(),
            committed_so_far as usize
        );
        // svc dropped here → owner thread exits → Sqlite connection closed.
    }

    // Final reopen confirms durable convergence.
    let svc = CoordinatorService::new(SqliteJournal::open(&path).unwrap());
    assert_eq!(
        svc.committed_count().await.unwrap(),
        (ROUNDS * BATCH) as usize,
        "every committed Mote survived {ROUNDS} crash/recover cycles"
    );
}
