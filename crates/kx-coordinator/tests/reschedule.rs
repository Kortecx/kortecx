//! Reschedule-on-failure (P3.2 / D57): when a worker dies (P3.1), its in-flight PURE
//! Motes are re-leased to a live worker, the death is recorded as a journal fact, and
//! exactly-once holds (the dead worker's late commit dedupes — first-wins).
//!
//! Time is driven by an injected [`kx_coordinator::Clock`] so a worker is declared dead
//! deterministically, with no sleeps. Death is asserted *via the service*: a reaped Mote
//! shows `state_of == Failed` (the projection is a pure fold of the log, so a `Failed`
//! state proves a `Failed{WorkerCrashed}` journal entry exists — D21 §11, no off-journal
//! facts) before its replacement commits.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use kx_coordinator::proto::{CommitOutcome, ExecutorClass};
use kx_coordinator::{
    Clock, CoordinatorService, InMemoryWorkerRegistry, MoteState, WorkerRegistry,
};
use kx_journal::InMemoryJournal;
use kx_mote::{Mote, NdClass};

const TIMEOUT: Duration = Duration::from_secs(6);

/// A deterministic clock the test advances by hand.
#[derive(Debug)]
struct FakeClock(AtomicU64);
impl FakeClock {
    fn new(ms: u64) -> Arc<Self> {
        Arc::new(Self(AtomicU64::new(ms)))
    }
    fn set(&self, ms: u64) {
        self.0.store(ms, Ordering::Relaxed);
    }
}
impl Clock for FakeClock {
    fn now_ms(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
}

fn coordinator(clock: Arc<FakeClock>) -> CoordinatorService {
    let registry: Arc<dyn WorkerRegistry> = Arc::new(
        InMemoryWorkerRegistry::with_clock_and_timeout(clock, TIMEOUT),
    );
    CoordinatorService::with_registry(InMemoryJournal::new(), registry)
}

const MAC: ExecutorClass = ExecutorClass::MacosSandbox;

/// R-1 (reschedule) + R-3 (death is a journal fact) + R-2 (dedupe on late commit).
#[tokio::test]
async fn dead_workers_in_flight_mote_is_rescheduled_and_commits_exactly_once() {
    let clock = FakeClock::new(1_000);
    let svc = coordinator(clock.clone());
    let warrant = common::sample_warrant();

    let dying = common::register(&svc, "dying").await;
    let m = common::mote(7, NdClass::Pure, &[]);
    common::submit(&svc, &m, &warrant).await;

    // The dying worker leases the Mote (now tracked as its in-flight lease) but never
    // commits it.
    let leased = common::lease_work(&svc, dying, MAC, 16).await;
    assert_eq!(leased.len(), 1, "dying worker leased the ready Mote");

    // Time advances past the liveness timeout; a fresh live worker registers and polls.
    clock.set(1_000 + 6_001);
    let live = common::register(&svc, "live").await;
    let released = common::lease_work(&svc, live, MAC, 16).await;
    let released_mote: Mote = released[0].mote.clone().unwrap().try_into().unwrap();
    assert_eq!(
        released_mote.id, m.id,
        "the dead worker's Mote was re-leased to the live worker"
    );

    // R-3: the death is a journal fact — the reap folded a Failed{WorkerCrashed}, so the
    // projection now reports the Mote `Failed` (until the replacement commits).
    assert_eq!(
        svc.state_of(m.id).await.unwrap(),
        MoteState::Failed,
        "worker-crash death is recorded as a journal fact (Failed state)"
    );

    // R-1: the live worker completes the replacement → exactly one commit.
    let out = common::commit(&svc, &m, live).await;
    assert_eq!(out.outcome, CommitOutcome::Committed as i32);
    assert_eq!(svc.committed_count().await.unwrap(), 1);
    assert_eq!(svc.state_of(m.id).await.unwrap(), MoteState::Committed);

    // R-2: the dead worker was not actually dead and commits late → dedupes (first-wins),
    // committed count unchanged.
    let late = common::commit(&svc, &m, dying).await;
    assert_eq!(
        late.outcome,
        CommitOutcome::AlreadyCommitted as i32,
        "the dead worker's late commit dedupes to the first"
    );
    assert_eq!(svc.committed_count().await.unwrap(), 1, "no double commit");
}

/// R-6 (command ordering: commit-then-reap). A worker that committed *before* being
/// declared dead is reaped as a no-op — no spurious Failed, no double commit.
#[tokio::test]
async fn reap_after_commit_is_a_noop() {
    let clock = FakeClock::new(1_000);
    let svc = coordinator(clock.clone());
    let warrant = common::sample_warrant();

    let worker = common::register(&svc, "w").await;
    let m = common::mote(3, NdClass::Pure, &[]);
    common::submit(&svc, &m, &warrant).await;

    // Lease then commit — the Mote is done before any death is declared.
    let _ = common::lease_work(&svc, worker, MAC, 16).await;
    common::commit(&svc, &m, worker).await;
    assert_eq!(svc.committed_count().await.unwrap(), 1);

    // Now the worker times out; a fresh worker polls, triggering the reap. The committed
    // Mote is resolved (not crash-failed) — it stays Committed, nothing is re-leased.
    clock.set(1_000 + 6_001);
    let other = common::register(&svc, "other").await;
    let leased = common::lease_work(&svc, other, MAC, 16).await;
    assert!(
        leased.is_empty(),
        "a committed Mote is not re-leased after reap"
    );
    assert_eq!(svc.state_of(m.id).await.unwrap(), MoteState::Committed);
    assert_eq!(
        svc.committed_count().await.unwrap(),
        1,
        "no spurious second commit"
    );
}

/// R-5 (no spurious reschedule). A *live* worker's lease is never crash-failed when a
/// peer polls and the reap runs.
#[tokio::test]
async fn a_live_workers_lease_is_not_reaped() {
    let clock = FakeClock::new(1_000);
    let svc = coordinator(clock.clone());
    let warrant = common::sample_warrant();

    let holder = common::register(&svc, "holder").await;
    let m = common::mote(5, NdClass::Pure, &[]);
    common::submit(&svc, &m, &warrant).await;
    let _ = common::lease_work(&svc, holder, MAC, 16).await;

    // Time advances, but the holder heartbeats (stays live); a peer polls → reap runs.
    clock.set(1_000 + 5_000);
    common::heartbeat(&svc, holder, 0, 0).await;
    let peer = common::register(&svc, "peer").await;
    let _ = common::lease_work(&svc, peer, MAC, 16).await;

    // The holder's Mote was NOT crash-failed (no Failed entry) — it is still leasable
    // (Pending), not Failed.
    assert_eq!(
        svc.state_of(m.id).await.unwrap(),
        MoteState::Pending,
        "a live worker's in-flight Mote is never reaped"
    );
}

/// Scalability / fault-tolerance under fan-out: a fleet runs many Motes distributed;
/// one worker dies mid-run holding a batch; every Mote still commits **exactly once**.
#[tokio::test]
async fn fleet_survives_one_worker_death_exactly_once() {
    let clock = FakeClock::new(1_000);
    let svc = coordinator(clock.clone());
    let warrant = common::sample_warrant();

    // A fleet of 4 workers and 24 independent ready PURE Motes.
    let fleet: Vec<u64> = {
        let mut ids = Vec::new();
        for tag in ["a", "b", "c", "d"] {
            ids.push(common::register(&svc, tag).await);
        }
        ids
    };
    let motes: Vec<Mote> = (0u8..24)
        .map(|s| common::mote(s, NdClass::Pure, &[]))
        .collect();
    for m in &motes {
        common::submit(&svc, m, &warrant).await;
    }

    // The doomed worker leases a batch and then "dies" (never commits, never heartbeats).
    let doomed = fleet[0];
    let stranded = common::lease_work(&svc, doomed, MAC, 6).await;
    assert!(!stranded.is_empty(), "doomed worker stranded a batch");

    // Time passes the timeout; the survivors keep heartbeating and drain everything.
    clock.set(1_000 + 6_001);
    let survivors = &fleet[1..];
    let mut committed_by: std::collections::BTreeMap<u64, usize> =
        std::collections::BTreeMap::new();
    for _round in 0..64 {
        for &w in survivors {
            common::heartbeat(&svc, w, clock.now_ms(), 0).await;
            let batch = common::lease_work(&svc, w, MAC, 4).await;
            for item in batch {
                let mote: Mote = item.mote.clone().unwrap().try_into().unwrap();
                let out = common::commit(&svc, &mote, w).await;
                if out.outcome == CommitOutcome::Committed as i32 {
                    *committed_by.entry(w).or_default() += 1;
                }
            }
        }
        if svc.committed_count().await.unwrap() >= motes.len() {
            break;
        }
    }

    // Every Mote committed exactly once — including the doomed worker's stranded batch,
    // which the survivors reaped + re-ran. No Mote was double-counted (dedupe holds).
    assert_eq!(
        svc.committed_count().await.unwrap(),
        motes.len(),
        "every Mote committed despite the worker death"
    );
    let total_new: usize = committed_by.values().sum();
    assert_eq!(
        total_new,
        motes.len(),
        "exactly-once: new commits sum to the Mote count (no double commit)"
    );
    for m in &motes {
        assert_eq!(svc.state_of(m.id).await.unwrap(), MoteState::Committed);
    }
}
