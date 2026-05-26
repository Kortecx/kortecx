//! `ResourceManager` — the runtime's self-management seam (D25). Distinct
//! from the capability broker (D24) per `resource-manager.md` §3 / `capability-
//! broker.md` §3: broker = workflow-declared effects; resource manager =
//! runtime self-management. The executor invokes both; never fuses them.
//!
//! **PR 9a skeleton.** `LocalResourceManager` does in-memory accounting of
//! acquired slots — `acquire` checks the requested ceiling fits within the
//! manager's configured caps + returns an opaque `Slot`; `release` decrements
//! the in-memory counters. The PR 9a-hardening follow-up adds: cgroup v2 file
//! I/O on Linux (`/sys/fs/cgroup/...`), `getrlimit`/`setrlimit` + a timer
//! thread for `wall_clock_ms` on macOS, NVML for GPU. **NEVER
//! `std::process::Command` shell-outs** — compile-time refused via
//! `crates/kx-executor/clippy.toml`.

use std::sync::{Arc, Mutex};

use kx_warrant::ResourceCeiling;
use thiserror::Error;

/// Self-management seam. P1 OSS impl is `LocalResourceManager`; cloud impls
/// live in `kx-cloud-resource-multitenant` per D28 (out of OSS workspace).
pub trait ResourceManager: Send + Sync {
    /// Reserve a slot meeting `ceiling`. Returns an opaque `Slot` that the
    /// caller MUST eventually pass to `release` (bracket discipline; verified
    /// by the executor's trace-assertion test per the kx-executor crate spec).
    fn acquire(&self, ceiling: &ResourceCeiling) -> Result<Slot, ResourceError>;

    /// Release a previously-acquired slot. Idempotent on a second call with
    /// the same `Slot` (per-Slot `id` accounts for the dedup).
    fn release(&self, slot: Slot) -> Result<(), ResourceError>;
}

/// Opaque slot handle. The trait-object boundary forbids the caller from
/// reaching into the manager's internal accounting; `id` is the only field
/// the executor reads (for tracing-span correlation).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Slot {
    /// Per-acquire monotonic id.
    pub id: u64,
}

/// Typed `ResourceManager` errors. Each variant is reachable by the
/// `LocalResourceManager` skeleton.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ResourceError {
    /// The requested `cpu_milli` exceeds the manager's configured cap.
    #[error("cpu cap exceeded: requested {requested} milli, cap {cap}")]
    CpuCapExceeded {
        /// What was requested.
        requested: u32,
        /// The configured cap.
        cap: u32,
    },

    /// The requested `mem_bytes` exceeds the manager's configured cap.
    #[error("memory cap exceeded: requested {requested} bytes, cap {cap}")]
    MemCapExceeded {
        /// What was requested.
        requested: u64,
        /// The configured cap.
        cap: u64,
    },

    /// The requested `fd_count` exceeds the manager's configured cap.
    #[error("fd cap exceeded: requested {requested}, cap {cap}")]
    FdCapExceeded {
        /// What was requested.
        requested: u32,
        /// The configured cap.
        cap: u32,
    },

    /// The manager has no remaining capacity for any axis right now.
    #[error("resource manager fully booked: no slots available")]
    NoCapacity,

    /// `release` called with a `Slot` not issued by this manager.
    #[error("unknown slot id {0}")]
    UnknownSlot(u64),

    /// Internal error (mutex poisoning, etc.).
    #[error("resource manager internal: {0}")]
    Internal(String),
}

/// OSS `ResourceManager` impl. PR 9a does in-memory accounting only; real
/// cgroup v2 / `setrlimit` enforcement ships in the PR 9a-hardening
/// follow-up.
#[derive(Debug)]
pub struct LocalResourceManager {
    inner: Arc<Mutex<LocalResourceState>>,
}

#[derive(Debug)]
struct LocalResourceState {
    cpu_cap_milli: u32,
    mem_cap_bytes: u64,
    fd_cap: u32,
    next_slot_id: u64,
    outstanding: std::collections::HashSet<u64>,
}

impl LocalResourceManager {
    /// Construct a new `LocalResourceManager` with caps on each axis. PR 9a
    /// uses generous defaults appropriate for the integration test path; the
    /// PR 9a-hardening follow-up wires cgroup v2 / `setrlimit` enforcement
    /// against these caps.
    #[must_use]
    pub fn new(cpu_cap_milli: u32, mem_cap_bytes: u64, fd_cap: u32) -> Self {
        Self {
            inner: Arc::new(Mutex::new(LocalResourceState {
                cpu_cap_milli,
                mem_cap_bytes,
                fd_cap,
                next_slot_id: 1,
                outstanding: std::collections::HashSet::new(),
            })),
        }
    }

    /// Convenience: a development-default manager with permissive caps.
    /// Suitable for integration tests + the cargo example demos.
    #[must_use]
    pub fn dev_defaults() -> Self {
        Self::new(u32::MAX, u64::MAX, u32::MAX)
    }
}

impl ResourceManager for LocalResourceManager {
    fn acquire(&self, ceiling: &ResourceCeiling) -> Result<Slot, ResourceError> {
        let mut state = self
            .inner
            .lock()
            .map_err(|e| ResourceError::Internal(format!("mutex poisoned: {e}")))?;

        if ceiling.cpu_milli > state.cpu_cap_milli {
            return Err(ResourceError::CpuCapExceeded {
                requested: ceiling.cpu_milli,
                cap: state.cpu_cap_milli,
            });
        }
        if ceiling.mem_bytes > state.mem_cap_bytes {
            return Err(ResourceError::MemCapExceeded {
                requested: ceiling.mem_bytes,
                cap: state.mem_cap_bytes,
            });
        }
        if ceiling.fd_count > state.fd_cap {
            return Err(ResourceError::FdCapExceeded {
                requested: ceiling.fd_count,
                cap: state.fd_cap,
            });
        }

        let id = state.next_slot_id;
        state.next_slot_id = state.next_slot_id.saturating_add(1);
        state.outstanding.insert(id);
        Ok(Slot { id })
    }

    fn release(&self, slot: Slot) -> Result<(), ResourceError> {
        let mut state = self
            .inner
            .lock()
            .map_err(|e| ResourceError::Internal(format!("mutex poisoned: {e}")))?;
        if state.outstanding.remove(&slot.id) {
            Ok(())
        } else {
            Err(ResourceError::UnknownSlot(slot.id))
        }
    }
}
