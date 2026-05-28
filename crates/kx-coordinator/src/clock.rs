//! [`Clock`] — the coordinator's liveness time source (P3.1).
//!
//! Worker-death detection measures the time elapsed since a worker's last
//! heartbeat. That measurement must read the **coordinator's** clock, not the
//! worker-supplied wire timestamp ([`HeartbeatRequest.timestamp_ms`]): a dead
//! worker sends nothing, so only the coordinator can observe the silence, and
//! stamping receipt time with a single clock makes liveness immune to cross-node
//! clock skew.
//!
//! The value is **liveness only — never hashed, never on the journal / Mote-identity
//! path** (SN-8). It is behind a trait so tests inject a deterministic clock and
//! advance time without real sleeps.
//!
//! [`HeartbeatRequest.timestamp_ms`]: kx_proto::proto::HeartbeatRequest

use std::time::{SystemTime, UNIX_EPOCH};

/// A wall-clock source in milliseconds since the Unix epoch.
///
/// Used only to time worker liveness; the value never enters a hash, a journal
/// entry, or any deterministic decision (SN-8). `Debug` is required so the
/// registry that owns a `dyn Clock` stays `Debug`.
pub trait Clock: Send + Sync + std::fmt::Debug {
    /// Milliseconds since the Unix epoch. Liveness only; never hashed.
    fn now_ms(&self) -> u64;
}

/// The production clock — the host wall clock.
///
/// A pre-epoch or overflowing reading collapses to `0` (the same total,
/// panic-free shape the worker's `now_ms` uses); `0` only ever makes a worker
/// look *older*, never spuriously live, so the conservative direction is kept.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|d| u64::try_from(d.as_millis()).ok())
            .unwrap_or(0)
    }
}
