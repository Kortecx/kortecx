//! Placement policy trait + two trivial impls.
//!
//! Per the P1.10 crate spec, placement is a **trait seam**:
//! the scheduler holds a single `P: Placement` and the impl decides which
//! `WorkerId` each Mote routes to. The DoD requires a "trivial local impl"
//! plus a "second trivial impl" to prove the trait is substitutable;
//! [`LocalPlacement`] and [`RoundRobinPlacement`] fill those slots.
//!
//! Placement does NOT see Mote bodies, parents, or warrants — only the
//! `MoteId`. This keeps the trait small enough that a P5 cloud impl
//! (rack-locality-aware, KV-cache-affinity-aware, NUMA-aware) can fit behind
//! the same surface without leaking placement-internal state into the
//! scheduler.

use std::sync::atomic::{AtomicU64, Ordering};

use kx_mote::MoteId;

use crate::worker::WorkerId;

/// Trait seam: decide which [`WorkerId`] receives a ready Mote.
///
/// **Inputs.** Only the `MoteId`. Placement decisions do not depend on the
/// Mote's body, its parents, or its warrant — those concerns live in
/// `kx-warrant` (capability scope) and `kx-executor` (resource fit).
///
/// **Determinism is NOT required.** [`RoundRobinPlacement`] mutates an
/// internal counter; that's allowed. The scheduler relies on the projection
/// for deterministic *ordering* (the projection's ready_set is deterministic
/// in the journal-fold prefix); placement is a *routing* concern and may
/// carry state.
///
/// `Send + Sync` is required: the P5 cloud impl will be shared across
/// dispatch threads via `Arc<dyn Placement>`.
///
/// ```
/// use kx_mote::MoteId;
/// use kx_scheduler::{LocalPlacement, Placement, WorkerId};
///
/// // The trait shape is small — `place` takes a MoteId and returns a WorkerId.
/// fn route<P: Placement>(p: &P, id: &MoteId) -> WorkerId {
///     p.place(id)
/// }
/// let w = route(&LocalPlacement, &MoteId::from_bytes([0u8; 32]));
/// assert_eq!(w, WorkerId(0));
/// ```
pub trait Placement: Send + Sync {
    /// Choose a worker for the given Mote. Called once per dispatch.
    fn place(&self, mote_id: &MoteId) -> WorkerId;
}

/// The trivial local placement — every Mote routes to `WorkerId(0)`.
///
/// Used by the P1 single-process runtime. The local executor runs
/// in-process and there is exactly one worker; placement is a no-op
/// decoration over that fact.
///
/// # Examples
///
/// ```
/// use kx_mote::MoteId;
/// use kx_scheduler::{LocalPlacement, Placement, WorkerId};
///
/// let p = LocalPlacement;
/// assert_eq!(p.place(&MoteId::from_bytes([0u8; 32])), WorkerId(0));
/// assert_eq!(p.place(&MoteId::from_bytes([0xff; 32])), WorkerId(0));
/// ```
#[derive(Debug, Default, Clone, Copy)]
pub struct LocalPlacement;

impl Placement for LocalPlacement {
    fn place(&self, _mote_id: &MoteId) -> WorkerId {
        WorkerId(0)
    }
}

/// The second trivial impl required by the DoD's "placement-trait swap" test.
///
/// Cycles ready Motes through `0..n_workers` in dispatch order via an
/// atomic counter. Useful for tests + as a worked example that the trait
/// seam is real (a second impl actually substitutes for the first).
///
/// # Examples
///
/// ```
/// use kx_mote::MoteId;
/// use kx_scheduler::{Placement, RoundRobinPlacement, WorkerId};
///
/// let p = RoundRobinPlacement::new(3);
/// // The counter advances independently of `mote_id` content.
/// assert_eq!(p.place(&MoteId::from_bytes([1u8; 32])), WorkerId(0));
/// assert_eq!(p.place(&MoteId::from_bytes([2u8; 32])), WorkerId(1));
/// assert_eq!(p.place(&MoteId::from_bytes([3u8; 32])), WorkerId(2));
/// assert_eq!(p.place(&MoteId::from_bytes([4u8; 32])), WorkerId(0));
/// ```
#[derive(Debug)]
pub struct RoundRobinPlacement {
    n_workers: u64,
    next: AtomicU64,
}

impl RoundRobinPlacement {
    /// Construct a round-robin placement cycling through `n_workers` workers.
    ///
    /// # Panics
    ///
    /// Panics if `n_workers == 0` — a placement that returns no worker is
    /// not a valid placement.
    #[must_use]
    pub fn new(n_workers: u64) -> Self {
        assert!(
            n_workers > 0,
            "RoundRobinPlacement requires at least one worker"
        );
        Self {
            n_workers,
            next: AtomicU64::new(0),
        }
    }
}

impl Placement for RoundRobinPlacement {
    fn place(&self, _mote_id: &MoteId) -> WorkerId {
        let i = self.next.fetch_add(1, Ordering::Relaxed);
        WorkerId(i % self.n_workers)
    }
}
