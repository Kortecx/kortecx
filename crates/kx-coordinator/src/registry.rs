//! The worker registry — the coordinator's view of registered workers, **behind a
//! trait** (P2.2 DoD item 2) so the in-memory default can be swapped for a
//! distributed / persistent store later without touching the service.
//!
//! In P2.2 the coordinator never dispatches work (that is P2.3): the registry is
//! liveness bookkeeping plus the **admission oracle** for `ReportCommit` — an
//! unregistered worker cannot propose a commit.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

use kx_scheduler::WorkerId;
use kx_warrant::ExecutorClass;

/// One worker's registration plus its last-known liveness.
#[derive(Debug, Clone)]
pub struct WorkerRecord {
    /// Coordinator-assigned identity.
    pub id: WorkerId,
    /// The sandbox backend this worker can run (D41 executor classes).
    pub executor_class: ExecutorClass,
    /// How the coordinator reaches the worker (host:port / URL / socket path).
    pub endpoint: String,
    /// Wall-clock ms of the most recent heartbeat (liveness only; never hashed).
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
}

/// In-memory [`WorkerRegistry`] — the OSS default.
///
/// `BTreeMap` for deterministic iteration order; an [`AtomicU64`] hands out
/// monotonic ids starting at `0`. A poisoned lock is recovered (not panicked):
/// the registry's invariants survive a panicking critic elsewhere.
#[derive(Debug, Default)]
pub struct InMemoryWorkerRegistry {
    next_id: AtomicU64,
    workers: Mutex<BTreeMap<WorkerId, WorkerRecord>>,
}

impl InMemoryWorkerRegistry {
    /// Construct an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl WorkerRegistry for InMemoryWorkerRegistry {
    fn register(&self, executor_class: ExecutorClass, endpoint: String) -> WorkerId {
        let id = WorkerId(self.next_id.fetch_add(1, Ordering::Relaxed));
        let record = WorkerRecord {
            id,
            executor_class,
            endpoint,
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
        let mut guard = self.workers.lock().unwrap_or_else(PoisonError::into_inner);
        let record = guard
            .get_mut(&worker)
            .ok_or(RegistryError::UnknownWorker(worker))?;
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
}
