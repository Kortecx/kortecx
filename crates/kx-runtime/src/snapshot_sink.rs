//! [`SnapshotSink`] — the D78 context-publishing seam.
//!
//! The orchestrator ([`crate::run_with_seams`]) owns the live
//! [`kx_projection::Projection`] at the dispatch point; the real-model dispatch
//! seams (a model [`kx_executor::MoteExecutor`] / [`kx_capability::CapabilityBroker`],
//! e.g. `kx-model-harness`) live behind frozen `&self` trait methods that receive
//! **no** snapshot. This sink bridges them: the orchestrator publishes the
//! current committed-state [`kx_projection::Snapshot`] here immediately before
//! each dispatch, and the seam reads it back to assemble the Mote's upstream
//! context + tool menu (`kx_context_assembler::assemble`, D78).
//!
//! It is an **opt-in, additive** seam: the canonical demo passes `None`, so no
//! snapshot is ever published and the deterministic truth path (digest
//! `7d22d4bd…`) is byte-unchanged. The published snapshot is **model input
//! only** — it never enters Mote identity or the journal (D64).

use std::sync::{Arc, PoisonError, RwLock};

use kx_projection::Snapshot;

/// A cheap, cloneable handle that carries the orchestrator's latest committed-
/// state [`Snapshot`] to the real-model dispatch seams (D78).
///
/// Clones share one slot (an `Arc`), so the orchestrator and the executor/broker
/// it injected into `run_with_seams` observe the same published value. Reads
/// return a clone of the current snapshot (or `None` before the first publish);
/// the seam treats `None` as "no upstream context" and falls back to the leaf
/// path — never an error.
#[derive(Clone, Default, Debug)]
pub struct SnapshotSink {
    slot: Arc<RwLock<Option<Snapshot>>>,
}

impl SnapshotSink {
    /// An empty sink (no snapshot published yet).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Publish the current committed-state snapshot, replacing any prior value.
    /// Called by the orchestrator once per loop iteration, before dispatching
    /// the iteration's Mote.
    ///
    /// Lock poisoning is recovered (never propagated): the published snapshot is
    /// non-authoritative **model input**, so a prior panic-while-publishing must
    /// not crash the runtime — the worst case is a stale-by-one snapshot, and
    /// this publish immediately overwrites it.
    pub fn publish(&self, snapshot: Snapshot) {
        *self.slot.write().unwrap_or_else(PoisonError::into_inner) = Some(snapshot);
    }

    /// The most recently published snapshot, or `None` if nothing was published
    /// yet. Returns a clone so the caller holds no lock across assembly. Lock
    /// poisoning is recovered (see [`Self::publish`]).
    #[must_use]
    pub fn latest(&self) -> Option<Snapshot> {
        self.slot
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}
