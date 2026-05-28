//! The worker registry — the coordinator's view of registered workers, **behind a
//! trait** (P2.2 DoD item 2) so the in-memory default can be swapped for a
//! distributed / persistent store later without touching the service.
//!
//! The registry is liveness bookkeeping plus the **admission oracle** for
//! `ReportCommit` / `LeaseWork` (an unregistered worker cannot propose or pull),
//! and — from P3.1 — the **worker-death detector**: it owns the coordinator's
//! [`Clock`], stamps each heartbeat's *receipt* time, and derives liveness as a
//! pure function of (last-seen, now, timeout). A worker that stops heartbeating
//! ages out of [`live_snapshot`](WorkerRegistry::live_snapshot), so placement (D56)
//! routes no new work to it. Detection only — rescheduling a dead worker's
//! in-flight Motes is P3.2 (safe only under the journal's idempotency dedupe).

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use kx_scheduler::WorkerId;
use kx_warrant::ExecutorClass;

use crate::clock::{Clock, SystemClock};

/// Default liveness window: a worker unheard-from for longer than this is `Dead`.
///
/// Chosen conservatively at ≥ 3× the recommended worker heartbeat cadence (2 s),
/// so two dropped/late heartbeats do not trip a false death (the P0.9 stuck-vs-dead
/// policy: never declare a slow-but-alive worker dead — the only consequence here
/// is placement eviction, which self-heals on the next heartbeat).
pub const DEFAULT_LIVENESS_TIMEOUT: Duration = Duration::from_secs(6);

/// A registered worker's liveness as of a given instant — a derived view, not
/// stored state (so it can never drift from the timestamps it summarizes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerStatus {
    /// Heard from within the liveness window.
    Live,
    /// Silent past the liveness window (heartbeat-timeout death, P3.1).
    Dead,
}

/// Whether a worker last seen at `last_seen_ms` is still live at `now_ms` under
/// `timeout_ms`. Pure + total: a `last_seen` in the future (clock non-monotonicity)
/// counts as live via the saturating subtraction — the conservative direction.
#[must_use]
pub fn is_live(last_seen_ms: u64, now_ms: u64, timeout_ms: u64) -> bool {
    now_ms.saturating_sub(last_seen_ms) <= timeout_ms
}

/// One worker's registration plus its last-known liveness.
#[derive(Debug, Clone)]
pub struct WorkerRecord {
    /// Coordinator-assigned identity.
    pub id: WorkerId,
    /// The sandbox backend this worker can run (D41 executor classes).
    pub executor_class: ExecutorClass,
    /// How the coordinator reaches the worker (host:port / URL / socket path).
    pub endpoint: String,
    /// **Coordinator-stamped** ms at which the most recent heartbeat (or the
    /// registration) was received — the single-clock basis for liveness (P3.1),
    /// immune to worker/coordinator clock skew. Liveness only; never hashed.
    pub last_seen_ms: u64,
    /// The worker-supplied wall-clock from its last heartbeat — advisory /
    /// diagnostic (the worker's own clock); NOT used for liveness. Never hashed.
    pub last_heartbeat_ms: u64,
    /// Motes the worker reported executing at its last heartbeat.
    pub in_flight: u32,
}

/// Errors from the worker registry.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RegistryError {
    /// An operation named a worker that never registered.
    #[error("unknown worker {0:?}")]
    UnknownWorker(WorkerId),
}

/// The worker-registry seam.
///
/// All methods take `&self`: the registry lives behind an `Arc` shared across the
/// async RPC handlers, so it owns its interior mutability.
pub trait WorkerRegistry: Send + Sync {
    /// Register a worker, assigning a fresh monotonic [`WorkerId`].
    fn register(&self, executor_class: ExecutorClass, endpoint: String) -> WorkerId;

    /// Record a heartbeat. Returns [`RegistryError::UnknownWorker`] if the id was
    /// never registered.
    fn heartbeat(&self, worker: WorkerId, now_ms: u64, in_flight: u32)
        -> Result<(), RegistryError>;

    /// Look up a worker's current record.
    fn get(&self, worker: WorkerId) -> Option<WorkerRecord>;

    /// Number of registered workers.
    fn len(&self) -> usize;

    /// Whether no worker has registered yet.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// A point-in-time copy of every registered worker's record — the input a
    /// placement policy ranks over (P2.5, D56). Default returns empty so existing
    /// implementors stay valid; the in-memory registry overrides it. Implementors
    /// MUST clone-and-release (never hold an internal lock across the returned data).
    fn snapshot(&self) -> Vec<WorkerRecord> {
        Vec::new()
    }

    /// A snapshot of the workers that are **currently live** — the input placement
    /// must rank over so a dead worker (heartbeat-timeout, P3.1) gets no new work.
    /// The implementor reads its own [`Clock`] + timeout to decide; the default
    /// (for registries that don't track liveness) treats every worker as live, so
    /// existing implementors keep their pre-P3.1 behavior.
    fn live_snapshot(&self) -> Vec<WorkerRecord> {
        self.snapshot()
    }

    /// The liveness [`WorkerStatus`] of one worker, or `None` if never registered —
    /// the operator/diagnostic query for worker-death detection. The default
    /// reports any registered worker `Live`; a liveness-tracking registry overrides
    /// it. This is also the seam P3.2 reads to trigger reschedule-on-death.
    fn status(&self, worker: WorkerId) -> Option<WorkerStatus> {
        self.get(worker).map(|_| WorkerStatus::Live)
    }
}

/// In-memory [`WorkerRegistry`] — the OSS default.
///
/// `BTreeMap` for deterministic iteration order; an [`AtomicU64`] hands out
/// monotonic ids starting at `0`. A poisoned lock is recovered (not panicked):
/// the registry's invariants survive a panicking critic elsewhere. It owns the
/// coordinator's [`Clock`] (the single liveness time source) and the liveness
/// timeout, so death detection (P3.1) is encapsulated here — callers ask for
/// [`live_snapshot`](WorkerRegistry::live_snapshot) / [`status`](WorkerRegistry::status)
/// without threading `now`/`timeout` around.
#[derive(Debug)]
pub struct InMemoryWorkerRegistry {
    next_id: AtomicU64,
    workers: Mutex<BTreeMap<WorkerId, WorkerRecord>>,
    clock: Arc<dyn Clock>,
    liveness_timeout: Duration,
}

impl Default for InMemoryWorkerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemoryWorkerRegistry {
    /// Construct an empty registry over the host wall clock ([`SystemClock`]) and
    /// the [`DEFAULT_LIVENESS_TIMEOUT`].
    #[must_use]
    pub fn new() -> Self {
        Self::with_clock(Arc::new(SystemClock))
    }

    /// Construct an empty registry over a caller-supplied [`Clock`] (tests inject a
    /// deterministic clock to advance time without sleeping) and the
    /// [`DEFAULT_LIVENESS_TIMEOUT`].
    #[must_use]
    pub fn with_clock(clock: Arc<dyn Clock>) -> Self {
        Self::with_clock_and_timeout(clock, DEFAULT_LIVENESS_TIMEOUT)
    }

    /// Construct an empty registry over a caller-supplied [`Clock`] and liveness
    /// timeout (tests that exercise the timeout boundary).
    #[must_use]
    pub fn with_clock_and_timeout(clock: Arc<dyn Clock>, liveness_timeout: Duration) -> Self {
        Self {
            next_id: AtomicU64::new(0),
            workers: Mutex::new(BTreeMap::new()),
            clock,
            liveness_timeout,
        }
    }

    /// The configured liveness window.
    #[must_use]
    pub fn liveness_timeout(&self) -> Duration {
        self.liveness_timeout
    }

    /// Liveness timeout in ms (saturating), the unit `is_live` compares in.
    fn timeout_ms(&self) -> u64 {
        u64::try_from(self.liveness_timeout.as_millis()).unwrap_or(u64::MAX)
    }
}

impl WorkerRegistry for InMemoryWorkerRegistry {
    fn register(&self, executor_class: ExecutorClass, endpoint: String) -> WorkerId {
        let id = WorkerId(self.next_id.fetch_add(1, Ordering::Relaxed));
        let record = WorkerRecord {
            id,
            executor_class,
            endpoint,
            // A just-registered worker is live: stamp receipt time so it is not
            // instantly aged out before its first heartbeat.
            last_seen_ms: self.clock.now_ms(),
            last_heartbeat_ms: 0,
            in_flight: 0,
        };
        self.workers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id, record);
        id
    }

    fn heartbeat(
        &self,
        worker: WorkerId,
        now_ms: u64,
        in_flight: u32,
    ) -> Result<(), RegistryError> {
        // Stamp the coordinator's own receipt time for liveness (skew-immune);
        // keep the worker-supplied `now_ms` only as an advisory diagnostic.
        let seen = self.clock.now_ms();
        let mut guard = self.workers.lock().unwrap_or_else(PoisonError::into_inner);
        let record = guard
            .get_mut(&worker)
            .ok_or(RegistryError::UnknownWorker(worker))?;
        record.last_seen_ms = seen;
        record.last_heartbeat_ms = now_ms;
        record.in_flight = in_flight;
        Ok(())
    }

    fn get(&self, worker: WorkerId) -> Option<WorkerRecord> {
        self.workers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&worker)
            .cloned()
    }

    fn len(&self) -> usize {
        self.workers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    fn snapshot(&self) -> Vec<WorkerRecord> {
        self.workers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .cloned()
            .collect()
    }

    fn live_snapshot(&self) -> Vec<WorkerRecord> {
        let now = self.clock.now_ms();
        let timeout = self.timeout_ms();
        self.workers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .filter(|w| is_live(w.last_seen_ms, now, timeout))
            .cloned()
            .collect()
    }

    fn status(&self, worker: WorkerId) -> Option<WorkerStatus> {
        let now = self.clock.now_ms();
        let timeout = self.timeout_ms();
        self.workers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&worker)
            .map(|w| {
                if is_live(w.last_seen_ms, now, timeout) {
                    WorkerStatus::Live
                } else {
                    WorkerStatus::Dead
                }
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLASS: ExecutorClass = ExecutorClass::MacOsSandbox;
    const TIMEOUT: Duration = Duration::from_secs(6);

    /// A deterministic clock the test advances by hand — no real sleeps.
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

    fn registry(clock: Arc<FakeClock>) -> InMemoryWorkerRegistry {
        InMemoryWorkerRegistry::with_clock_and_timeout(clock, TIMEOUT)
    }

    #[test]
    fn is_live_is_total_and_conservative() {
        // Exactly at the window edge is still live; one ms past is dead.
        assert!(is_live(0, 6_000, 6_000));
        assert!(!is_live(0, 6_001, 6_000));
        // A future last-seen (clock non-monotonicity) is treated as live.
        assert!(is_live(100, 50, 6_000));
    }

    #[test]
    fn a_freshly_registered_worker_is_live_before_any_heartbeat() {
        let clock = FakeClock::new(1_000);
        let reg = registry(clock);
        let w = reg.register(CLASS, "w".into());
        assert_eq!(reg.status(w), Some(WorkerStatus::Live));
        assert_eq!(reg.live_snapshot().len(), 1);
    }

    #[test]
    fn a_worker_goes_dead_after_the_timeout_and_revives_on_heartbeat() {
        let clock = FakeClock::new(1_000);
        let reg = registry(clock.clone());
        let w = reg.register(CLASS, "w".into());

        // Within the window: live.
        clock.set(1_000 + 6_000);
        assert_eq!(reg.status(w), Some(WorkerStatus::Live));

        // One ms past the window with no heartbeat: dead, and evicted from the
        // live snapshot (placement will route it no new work).
        clock.set(1_000 + 6_001);
        assert_eq!(reg.status(w), Some(WorkerStatus::Dead));
        assert!(reg.live_snapshot().is_empty());

        // A heartbeat re-stamps receipt time → live again (self-healing).
        reg.heartbeat(w, 999_999, 0).unwrap();
        assert_eq!(reg.status(w), Some(WorkerStatus::Live));
        assert_eq!(reg.live_snapshot().len(), 1);
    }

    #[test]
    fn heartbeat_stamps_coordinator_clock_not_worker_timestamp() {
        let clock = FakeClock::new(5_000);
        let reg = registry(clock);
        let w = reg.register(CLASS, "w".into());
        // Worker reports a wildly skewed timestamp; liveness uses the coordinator
        // clock (last_seen_ms = 5_000), the worker value is advisory only.
        reg.heartbeat(w, 42, 3).unwrap();
        let rec = reg.get(w).unwrap();
        assert_eq!(rec.last_seen_ms, 5_000, "coordinator-stamped receipt time");
        assert_eq!(
            rec.last_heartbeat_ms, 42,
            "worker timestamp kept as advisory"
        );
        assert_eq!(rec.in_flight, 3);
    }

    #[test]
    fn status_of_unknown_worker_is_none() {
        let reg = registry(FakeClock::new(1));
        assert_eq!(reg.status(WorkerId(999)), None);
    }
}
