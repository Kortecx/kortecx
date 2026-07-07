//! Parallel-local-exec mandatory gates — the bounded worker pool + the
//! `is_leased` admission gate MUST partition work across concurrent leasers AND leave
//! the committed truth byte-identical to a single worker.
//!
//! - **lease-exclusivity**: two workers polling the same ready set never get the same
//!   Mote (`LeaseTracker::is_leased_by_other` in `lease_ready`) — this is what turns
//!   pool>1 into real work-PARTITIONING rather than duplicated leases.
//! - **digest-invariance(pool ∈ {1,2,4})**: driving the identical PURE workflow through
//!   N leasing workers yields the identical set of committed `(mote_id, result_ref,
//!   nd_class)` facts. Sorting by `mote_id` and comparing IS the canonical projection
//!   digest by construction (`kx_runtime::digest::digest_projection` hashes those tuples
//!   sorted by `MoteId`, with worker/execution ORDER and seq excluded). So this proves
//!   the pool cannot move the digest `7d22d4bd…`, at the lease→commit layer, without a
//!   circular dev-dep on kx-runtime.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use kx_coordinator::proto;
use kx_coordinator::{Clock, CoordinatorService, InMemoryWorkerRegistry, WorkerRegistry};
use kx_journal::{InMemoryJournal, SqliteJournal};
use kx_mote::{Mote, MoteId, NdClass};
use tempfile::tempdir;

/// Liveness window for the worker-death gate (matches the `reschedule` harness).
const LIVENESS_TIMEOUT: Duration = Duration::from_secs(6);

/// A deterministic clock the test advances by hand (no sleeps) — a worker is
/// declared dead purely as a function of `(last_seen, now, timeout)`.
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

/// A coordinator whose worker-liveness is driven by `clock` (so a death is
/// declared deterministically).
fn coordinator_with_clock(clock: Arc<FakeClock>) -> CoordinatorService {
    let registry: Arc<dyn WorkerRegistry> = Arc::new(
        InMemoryWorkerRegistry::with_clock_and_timeout(clock, LIVENESS_TIMEOUT),
    );
    CoordinatorService::with_registry(InMemoryJournal::new(), registry)
}

/// The `MoteId` bytes of a leased `WorkItem` (via the same wire conversion the worker uses).
fn leased_id(item: &proto::WorkItem) -> Vec<u8> {
    let mote: Mote = item.mote.clone().unwrap().try_into().unwrap();
    mote.id.as_bytes().to_vec()
}

#[tokio::test]
async fn two_workers_never_co_lease_the_same_mote() {
    let svc = CoordinatorService::new(InMemoryJournal::new());
    let warrant = common::sample_warrant();

    // 8 parentless PURE Motes, all ready at once.
    let motes: Vec<Mote> = (0..8)
        .map(|i| common::mote(i, NdClass::Pure, &[]))
        .collect();
    for m in &motes {
        common::submit(&svc, m, &warrant).await;
    }

    let a = common::register(&svc, "wa").await;
    let b = common::register(&svc, "wb").await;

    // Worker A leases up to 4; worker B then polls the SAME ready set.
    let a_items = common::lease_work(&svc, a, common::WORKER_CLASS.into(), 4).await;
    let b_items = common::lease_work(&svc, b, common::WORKER_CLASS.into(), 4).await;

    let a_ids: BTreeSet<Vec<u8>> = a_items.iter().map(leased_id).collect();
    let b_ids: BTreeSet<Vec<u8>> = b_items.iter().map(leased_id).collect();

    assert!(!a_ids.is_empty(), "worker A leased some ready work");
    assert!(
        !b_ids.is_empty(),
        "worker B leased the REMAINING ready work (the gate partitions, not starves)"
    );
    assert!(
        a_ids.is_disjoint(&b_ids),
        "the is_leased admission gate must never hand the same Mote to two live workers: \
         A={a_ids:?} B={b_ids:?}"
    );
}

/// Drive N leasing workers round-robin until every submitted Mote commits, then return
/// the committed `(mote_id, result_ref)` facts sorted by `mote_id` (= the digest inputs).
async fn commit_facts_with_pool(n_workers: usize, motes: &[Mote]) -> Vec<(Vec<u8>, Vec<u8>)> {
    let svc = CoordinatorService::new(InMemoryJournal::new());
    let warrant = common::sample_warrant();
    for m in motes {
        common::submit(&svc, m, &warrant).await;
    }
    let by_id: HashMap<Vec<u8>, &Mote> = motes
        .iter()
        .map(|m| (m.id.as_bytes().to_vec(), m))
        .collect();

    let mut workers = Vec::with_capacity(n_workers);
    for w in 0..n_workers {
        workers.push(common::register(&svc, &format!("w{w}")).await);
    }

    // Each round: every worker leases (the gate partitions the ready set) then commits
    // its leased Motes. Bounded by a generous iteration ceiling (parentless ready work
    // drains fast); assert real progress so a regression can't pass by silently stalling.
    let target = motes.len();
    let mut rounds = 0;
    loop {
        let mut progressed = false;
        for &wid in &workers {
            let items = common::lease_work(&svc, wid, common::WORKER_CLASS.into(), 4).await;
            for item in items {
                let id = leased_id(&item);
                let mote = by_id.get(&id).expect("leased a submitted Mote");
                common::commit(&svc, mote, wid).await;
                progressed = true;
            }
        }
        if svc.committed_count().await.unwrap() >= target {
            break;
        }
        rounds += 1;
        assert!(
            progressed && rounds < 1_000,
            "pool of {n_workers} drained {}/{target} in {rounds} rounds without progress",
            svc.committed_count().await.unwrap()
        );
    }

    let entries = common::read_entries(&svc, 0, 100_000).await;
    let mut facts: Vec<(Vec<u8>, Vec<u8>)> = entries
        .entries
        .iter()
        .map(|e| {
            let (mote_id, result_ref, _seq) = common::committed_view(e);
            (mote_id, result_ref)
        })
        .collect();
    // Sort by mote_id exactly as `digest_projection` does (order-independence is the
    // structural reason a pool cannot move the digest).
    facts.sort();
    facts
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn digest_is_invariant_across_pool_sizes() {
    // A deterministic PURE workflow: 24 parentless Motes (distinct ids ⇒ distinct
    // committed facts). Each Mote's committed fact is a pure function of the Mote, so
    // the committed SET is identical no matter which worker commits which Mote — the
    // pool only changes ordering (seq), which the digest excludes.
    let motes: Vec<Mote> = (0..24)
        .map(|i| common::mote(i, NdClass::Pure, &[]))
        .collect();

    let one = commit_facts_with_pool(1, &motes).await;
    let two = commit_facts_with_pool(2, &motes).await;
    let four = commit_facts_with_pool(4, &motes).await;

    assert_eq!(
        one.len(),
        motes.len(),
        "pool=1 committed every Mote exactly once"
    );
    assert_eq!(
        one, two,
        "digest inputs must be identical at pool=1 and pool=2 (worker order is not a digest input)"
    );
    assert_eq!(
        one, four,
        "digest inputs must be identical at pool=1 and pool=4"
    );

    // Distinctness sanity: 24 unique mote_ids (the workflow really did fan out).
    let unique: BTreeSet<&Vec<u8>> = four.iter().map(|(m, _)| m).collect();
    assert_eq!(
        unique.len(),
        motes.len(),
        "every Mote committed exactly once, no dupes"
    );
    let _ = MoteId::from_bytes([0u8; 32]); // keep the MoteId import honest across refactors
}

/// Read the committed `(mote_id, result_ref)` facts of a service, sorted by mote_id.
async fn committed_facts(svc: &CoordinatorService) -> Vec<(Vec<u8>, Vec<u8>)> {
    let entries = common::read_entries(svc, 0, 100_000).await;
    let mut facts: Vec<(Vec<u8>, Vec<u8>)> = entries
        .entries
        .iter()
        .map(|e| {
            let (mote_id, result_ref, _seq) = common::committed_view(e);
            (mote_id, result_ref)
        })
        .collect();
    facts.sort();
    facts
}

/// Drive a `pool` of workers over `svc` for up to `max_commits` commits (or until the
/// ready set drains), returning how many commits landed. Used to run a multi-worker
/// swarm PARTWAY before simulating a crash.
async fn drive_pool(
    svc: &CoordinatorService,
    motes: &[Mote],
    pool: usize,
    max_commits: usize,
) -> usize {
    let by_id: HashMap<Vec<u8>, &Mote> = motes
        .iter()
        .map(|m| (m.id.as_bytes().to_vec(), m))
        .collect();
    let mut workers = Vec::with_capacity(pool);
    for w in 0..pool {
        workers.push(common::register(svc, &format!("w{w}")).await);
    }
    let mut done = 0usize;
    'outer: loop {
        let mut progressed = false;
        for &wid in &workers {
            let items = common::lease_work(svc, wid, common::WORKER_CLASS.into(), 2).await;
            for item in items {
                let id = leased_id(&item);
                let mote = by_id.get(&id).expect("leased a submitted Mote");
                common::commit(svc, mote, wid).await;
                done += 1;
                progressed = true;
                if done >= max_commits {
                    break 'outer;
                }
            }
        }
        if !progressed {
            break;
        }
    }
    done
}

/// **crash-mid-swarm replay-determinism (mandatory gate).** A multi-worker
/// (pool>1) run over a durable journal, hard-dropped MID-FLIGHT (the coordinator process
/// dies with in-flight leases outstanding), then recovered over the SAME journal by a
/// fresh coordinator + a fresh worker pool that finishes the run, produces the IDENTICAL
/// committed-fact set (⇒ identical digest) as an uninterrupted single-worker run — and
/// commits every Mote exactly once (no double-commit across the crash boundary). This is
/// the pool analogue of the single-worker `kill_and_replay` guarantee: parallelism +
/// crash + recovery must not move the truth.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn crash_mid_swarm_recovers_identical_facts() {
    let motes: Vec<Mote> = (0..16)
        .map(|i| common::mote(i, NdClass::Pure, &[]))
        .collect();
    let warrant = common::sample_warrant();

    // Reference: a clean, uninterrupted single-worker run (order-independent facts).
    let reference = commit_facts_with_pool(1, &motes).await;

    let dir = tempdir().unwrap();
    let path = dir.path().join("journal.db");

    // Phase 1: a pool of 2 workers commits PART of the run over a durable journal, then
    // the coordinator is dropped mid-flight (a crash with leases still outstanding).
    let partial;
    {
        let svc = CoordinatorService::new(SqliteJournal::open(&path).unwrap());
        for m in &motes {
            common::submit(&svc, m, &warrant).await;
        }
        partial = drive_pool(&svc, &motes, 2, 6).await;
        assert!(
            partial >= 1 && partial < motes.len(),
            "the crash must land mid-run: committed {partial}/{}",
            motes.len()
        );
    } // drop = crash: in-memory dispatch defs + lease tracker are LOST; the journal survives.

    // Phase 2: a fresh coordinator recovers the committed facts from the journal; a fresh
    // worker pool RE-submits the workflow (the gateway re-derives uncommitted work — the
    // committed Motes dedup) and drives it to completion.
    let svc2 = CoordinatorService::new(SqliteJournal::open(&path).unwrap());
    let recovered_before = svc2.committed_count().await.unwrap();
    assert_eq!(
        recovered_before, partial,
        "the fresh coordinator recovers exactly the pre-crash committed count from the journal"
    );
    for m in &motes {
        common::submit(&svc2, m, &warrant).await; // committed ones dedup; rest become leasable
    }
    drive_pool(&svc2, &motes, 2, usize::MAX).await;

    assert_eq!(
        svc2.committed_count().await.unwrap(),
        motes.len(),
        "the recovered run commits every Mote exactly once (no loss, no double-commit)"
    );
    let recovered = committed_facts(&svc2).await;
    assert_eq!(
        recovered, reference,
        "crash-mid-swarm recovery must reproduce the identical committed-fact set (⇒ digest) \
         as an uninterrupted run"
    );
}

/// **worker-death-mid-swarm digest-invariance (gate).** The COORDINATOR survives
/// but ONE pool worker dies mid-swarm holding a leased batch (the "SIGKILL a worker" case,
/// complementing `crash_mid_swarm` which drops the whole coordinator). After the liveness
/// window the coordinator reaps the dead worker (folding a `Failed{WorkerCrashed}` fact +
/// clearing its holds via `take_leases`), the surviving pool re-leases the re-offered Motes
/// — the `is_leased_by_other` gate excludes only a LIVE other holder, so a reaped hold is
/// leasable again — and drains the run. The committed-fact set (⇒ digest) must be IDENTICAL
/// to an uninterrupted single-worker run: a worker death + reap must not move the truth.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn worker_death_mid_swarm_is_digest_invariant() {
    // 20 parentless PURE Motes.
    let motes: Vec<Mote> = (0..20)
        .map(|i| common::mote(i, NdClass::Pure, &[]))
        .collect();
    let warrant = common::sample_warrant();

    // Reference: the uninterrupted committed-fact set (order-independent).
    let reference = commit_facts_with_pool(1, &motes).await;

    // A pool of 4 over a deterministic clock; the doomed worker leases a batch then dies.
    let clock = FakeClock::new(1_000);
    let svc = coordinator_with_clock(clock.clone());
    for m in &motes {
        common::submit(&svc, m, &warrant).await;
    }
    let by_id: HashMap<Vec<u8>, &Mote> = motes
        .iter()
        .map(|m| (m.id.as_bytes().to_vec(), m))
        .collect();

    let mut fleet = Vec::new();
    for tag in ["a", "b", "c", "d"] {
        fleet.push(common::register(&svc, tag).await);
    }
    let doomed = fleet[0];
    let stranded = common::lease_work(&svc, doomed, common::WORKER_CLASS.into(), 6).await;
    assert!(
        !stranded.is_empty(),
        "the doomed worker stranded a leased batch mid-swarm"
    );

    // Advance past the liveness window; the SURVIVORS heartbeat, reap the dead worker's
    // holds, and drain everything.
    clock.set(1_000 + LIVENESS_TIMEOUT.as_millis() as u64 + 1);
    let survivors: Vec<u64> = fleet[1..].to_vec();
    for _round in 0..256 {
        for &w in &survivors {
            common::heartbeat(&svc, w, clock.now_ms(), 0).await;
            let batch = common::lease_work(&svc, w, common::WORKER_CLASS.into(), 4).await;
            for item in batch {
                let id = leased_id(&item);
                let mote = by_id.get(&id).expect("leased a submitted Mote");
                common::commit(&svc, mote, w).await;
            }
        }
        if svc.committed_count().await.unwrap() >= motes.len() {
            break;
        }
    }

    assert_eq!(
        svc.committed_count().await.unwrap(),
        motes.len(),
        "the survivors drained every Mote after the worker death (no loss, no double-commit)"
    );
    let recovered = committed_facts(&svc).await;
    assert_eq!(
        recovered, reference,
        "a worker death + reap + re-offer mid-swarm must not move the committed-fact set (⇒ \
         digest); the is_leased_by_other gate excludes only LIVE others, so reaped holds re-lease"
    );
}
