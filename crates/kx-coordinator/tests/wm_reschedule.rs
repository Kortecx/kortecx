//! P3.6c — R-13 under distribution: the coordinator gates a crash-failed *re-dispatch* of a
//! non-PURE Mote on the recovery oracle, so a WORLD-MUTATING effect that may have fired is
//! never re-leased without a durable `EffectStaged` hint.
//!
//! Single-node, `pick_next` / the executor's R-13 refuse to re-dispatch a WM Mote whose
//! `EffectStaged` was never recorded (the effect might already have fired — re-dispatch would
//! double it, which is unrecoverable). Distributed, reschedule (D57) re-leases dead workers'
//! in-flight Motes; without this gate it would re-offer a fired-but-unstaged VTC / IBC producer
//! (D58 lets those patterns dispatch WITHOUT staging) and a second worker would re-fire the
//! effect. The gate (`redispatch_admissible`) closes that window: PURE is always recomputable;
//! a non-PURE crash-failed Mote is re-offered ONLY with the `EffectStaged` hint
//! (`can_redispatch_world_effect`); otherwise it is left stuck (operator-recoverable).
//!
//! Time is driven by an injected [`kx_coordinator::Clock`] — death is deterministic, no sleeps.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use kx_coordinator::proto::ExecutorClass;
use kx_coordinator::{
    Clock, CoordinatorService, InMemoryWorkerRegistry, MoteState, WorkerRegistry,
};
use kx_journal::InMemoryJournal;
use kx_mote::{EffectPattern, Mote, NdClass};

const TIMEOUT: Duration = Duration::from_secs(6);
const MAC: ExecutorClass = ExecutorClass::MacosSandbox;

/// A deterministic clock the test advances by hand (mirrors `tests/reschedule.rs`).
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

/// R-13: a non-PURE Mote that crashed WITHOUT a durable `EffectStaged` hint (the
/// `ValidateThenCommit` / `IdempotentByConstruction` fire-then-crash window — those patterns
/// never stage) is NOT re-offered. Re-dispatch would risk doubling a real-world effect.
#[tokio::test]
async fn crash_failed_world_mutating_without_effect_staged_is_not_re_offered() {
    for pattern in [
        EffectPattern::ValidateThenCommit,
        EffectPattern::IdempotentByConstruction,
    ] {
        let clock = FakeClock::new(1_000);
        let svc = coordinator(clock.clone());
        let warrant = common::sample_warrant();

        let dying = common::register(&svc, "dying").await;
        let m = common::wm_mote(7, pattern);
        common::submit(&svc, &m, &warrant).await;

        // First dispatch is allowed (fresh ready Mote); the worker fires the effect then dies
        // WITHOUT staging — for VTC/IBC there is no EffectStaged step (D58 §4).
        let leased = common::lease_work(&svc, dying, MAC, 16).await;
        assert_eq!(leased.len(), 1, "{pattern:?}: first dispatch is offered");

        // Time advances past the timeout; a live worker polls → reap re-classifies the dead
        // lease as crash-failed, but the oracle gate refuses to re-offer it.
        clock.set(1_000 + 6_001);
        let live = common::register(&svc, "live").await;
        let offered = common::lease_work(&svc, live, MAC, 16).await;
        assert!(
            offered.is_empty(),
            "{pattern:?}: a crash-failed non-PURE Mote with no EffectStaged is NOT re-offered \
             (R-13 — re-dispatch could double the effect)"
        );
        assert_eq!(
            svc.state_of(m.id).await.unwrap(),
            MoteState::Failed,
            "{pattern:?}: it is left stuck (operator-recoverable via repudiation), not re-leased"
        );
    }
}

/// R-13: a `StageThenCommit` Mote that DID record `EffectStaged` before crashing IS re-offered —
/// the durable hint makes re-dispatch safe (the broker's tool-boundary idempotency dedupes the
/// re-fire). This is the safe path P3.6b's W-3 exercises end-to-end.
#[tokio::test]
async fn crash_failed_world_mutating_with_effect_staged_is_re_offered() {
    let clock = FakeClock::new(1_000);
    let svc = coordinator(clock.clone());
    let warrant = common::sample_warrant();

    let dying = common::register(&svc, "dying").await;
    let m = common::wm_mote(9, EffectPattern::StageThenCommit);
    common::submit(&svc, &m, &warrant).await;

    let leased = common::lease_work(&svc, dying, MAC, 16).await;
    assert_eq!(leased.len(), 1, "first dispatch is offered");

    // The worker stages its intent (EffectStaged recorded) then crashes before committing.
    common::report_effect_staged(&svc, &m, dying).await;

    clock.set(1_000 + 6_001);
    let live = common::register(&svc, "live").await;
    let offered = common::lease_work(&svc, live, MAC, 16).await;
    assert_eq!(
        offered.len(),
        1,
        "with the EffectStaged hint, re-dispatch is safe → the Mote is re-offered"
    );
    let offered_mote: Mote = offered[0].mote.clone().unwrap().try_into().unwrap();
    assert_eq!(offered_mote.id, m.id);
}

/// Regression guard: a crash-failed PURE Mote is always re-offered (recomputable — no
/// world-effect hazard). The gate must not over-reach into the PURE reschedule path (D57).
#[tokio::test]
async fn crash_failed_pure_mote_is_still_re_offered() {
    let clock = FakeClock::new(1_000);
    let svc = coordinator(clock.clone());
    let warrant = common::sample_warrant();

    let dying = common::register(&svc, "dying").await;
    let m = common::mote(3, NdClass::Pure, &[]);
    common::submit(&svc, &m, &warrant).await;

    let leased = common::lease_work(&svc, dying, MAC, 16).await;
    assert_eq!(leased.len(), 1);

    clock.set(1_000 + 6_001);
    let live = common::register(&svc, "live").await;
    let offered = common::lease_work(&svc, live, MAC, 16).await;
    assert_eq!(
        offered.len(),
        1,
        "PURE is recomputable — a crash-failed PURE Mote is still re-leased (D57 unchanged)"
    );
}
