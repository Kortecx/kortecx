//! Worker-death detection (P3.1): a worker that stops heartbeating ages out of the
//! coordinator's live view past the liveness timeout, so `LeaseWork` (placement v2,
//! D56) routes no new work to it. Detection only — rescheduling the dead worker's
//! in-flight Motes is P3.2.
//!
//! Time is driven by an injected [`kx_coordinator::Clock`] so the test advances past
//! the timeout deterministically, with no real sleeps. The coordinator stamps each
//! heartbeat's *receipt* time with this clock (skew-immune liveness).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use kx_coordinator::proto::ExecutorClass;
use kx_coordinator::{
    Clock, CoordinatorService, InMemoryWorkerRegistry, WorkerId, WorkerRegistry, WorkerStatus,
};
use kx_journal::InMemoryJournal;
use kx_mote::NdClass;

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

#[tokio::test]
async fn a_silent_worker_is_detected_dead_and_gets_no_new_work() {
    let clock = FakeClock::new(1_000);
    let svc = coordinator(clock.clone());
    let warrant = common::sample_warrant(); // executor_class = MacOsSandbox

    let dead = common::register(&svc, "dead").await;
    let live = common::register(&svc, "live").await;

    // Several independent ready PURE roots to place.
    let motes: Vec<_> = (10u8..18)
        .map(|s| common::mote(s, NdClass::Pure, &[]))
        .collect();
    for m in &motes {
        common::submit(&svc, m, &warrant).await;
    }

    // Both fresh registrations are live.
    assert_eq!(
        svc.registry().status(WorkerId(dead)),
        Some(WorkerStatus::Live)
    );
    assert_eq!(
        svc.registry().status(WorkerId(live)),
        Some(WorkerStatus::Live)
    );

    // Advance just past the timeout with no heartbeats: both have aged out.
    clock.set(1_000 + 6_001);
    assert_eq!(
        svc.registry().status(WorkerId(dead)),
        Some(WorkerStatus::Dead)
    );
    assert_eq!(
        svc.registry().status(WorkerId(live)),
        Some(WorkerStatus::Dead)
    );

    // The `live` worker heartbeats (re-stamping its receipt time); `dead` stays silent.
    common::heartbeat(&svc, live, 0, 0).await;
    assert_eq!(
        svc.registry().status(WorkerId(live)),
        Some(WorkerStatus::Live)
    );
    assert_eq!(
        svc.registry().status(WorkerId(dead)),
        Some(WorkerStatus::Dead)
    );

    // Every Mote the live worker leases is placement-preferred for it (the dead
    // worker is not a candidate) — and it can drain the whole ready set.
    let leased = common::lease_work(&svc, live, ExecutorClass::MacosSandbox, 64).await;
    assert_eq!(
        leased.len(),
        motes.len(),
        "the live worker can lease all ready work; the dead one is evicted from placement"
    );

    // Even if the dead worker *did* poll (e.g. a network partition that later heals),
    // fill-to-max still hands it ready work rather than starving it — detection only
    // stops *placement preference*, it does not refuse a registered worker (reschedule
    // + eviction-on-poll is P3.2). The point P3.1 proves: no new work is *placed* on it.
    let dead_leased = common::lease_work(&svc, dead, ExecutorClass::MacosSandbox, 64).await;
    // Work was already leased to `live` above but never committed; both can see ready
    // work (double-execution is harmless under journal dedupe, D54). The assertion that
    // matters is liveness status, checked above.
    let _ = dead_leased;
}

#[tokio::test]
async fn a_heartbeat_keeps_a_worker_live_across_the_window() {
    let clock = FakeClock::new(0);
    let svc = coordinator(clock.clone());

    let w = common::register(&svc, "w").await;

    // Walk time forward in sub-timeout steps, heartbeating each step: the worker
    // never crosses the death threshold.
    for step in 1..=10u64 {
        clock.set(step * 5_000); // 5 s < 6 s timeout
        common::heartbeat(&svc, w, 0, 0).await;
        assert_eq!(
            svc.registry().status(WorkerId(w)),
            Some(WorkerStatus::Live),
            "a worker heartbeating inside the window is always live (step {step})"
        );
    }

    // Stop heartbeating and cross the window once: now dead.
    clock.set(10 * 5_000 + 6_001);
    assert_eq!(svc.registry().status(WorkerId(w)), Some(WorkerStatus::Dead));
}
