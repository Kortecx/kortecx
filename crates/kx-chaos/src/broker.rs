//! The chaos broker — the instrument that proves exactly-once at the world boundary.
//!
//! It implements [`CapabilityBroker`] but performs no real I/O: it returns a
//! deterministic staged ref (content-addressed on the Mote id) and, crucially,
//! **counts net world effects** by the tool-boundary idempotency key (the Mote id,
//! per [`idempotency_token_for`](kx_capability::idempotency_token_for)). A
//! re-dispatch of an already-applied key is a no-op at the world — exactly what a
//! `Token`-class tool guarantees — so it bumps `dispatch_calls` but not
//! `net_effects`. "Killed mid-effect, replacement re-fires, still ≤1 net effect" is
//! read straight off these two counters.
//!
//! One broker is shared (cloned `Arc`s) across every simulated worker in a run, so the
//! applied-key set spans the dying worker's fire and the replacement's re-fire.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use kx_capability::{BrokerError, BrokerHandle, CapabilityBroker, EffectRequest};
use kx_content::ContentRef;
use kx_mote::{Mote, ToolName, ToolVersion};
use kx_warrant::WarrantSpec;

/// A counting, side-effect-free [`CapabilityBroker`]. Clones share the counters.
#[derive(Clone)]
pub(crate) struct ChaosBroker {
    dispatch_calls: Arc<AtomicUsize>,
    net_effects: Arc<AtomicUsize>,
    applied_keys: Arc<Mutex<BTreeSet<[u8; 32]>>>,
}

impl ChaosBroker {
    /// A fresh broker with zeroed counters.
    pub(crate) fn new() -> Self {
        Self {
            dispatch_calls: Arc::new(AtomicUsize::new(0)),
            net_effects: Arc::new(AtomicUsize::new(0)),
            applied_keys: Arc::new(Mutex::new(BTreeSet::new())),
        }
    }

    /// Total dispatch calls seen (a re-fire after a death counts here).
    pub(crate) fn dispatch_calls(&self) -> usize {
        self.dispatch_calls.load(Ordering::SeqCst)
    }

    /// Net world effects = count of distinct idempotency keys applied. This is the
    /// exactly-once witness: it must never exceed the number of WM Motes that fired.
    pub(crate) fn net_effects(&self) -> usize {
        self.net_effects.load(Ordering::SeqCst)
    }

    /// The deterministic staged ref for `mote` — content-addressed on its id, so a
    /// re-stage yields the identical ref (mirrors a real content-addressed store).
    pub(crate) fn staged_ref(mote: &Mote) -> ContentRef {
        ContentRef::from_bytes(*blake3::hash(mote.id.as_bytes()).as_bytes())
    }
}

impl CapabilityBroker for ChaosBroker {
    fn dispatch(
        &self,
        mote: &Mote,
        _warrant: &WarrantSpec,
        capability: &ToolName,
        request: EffectRequest,
    ) -> Result<BrokerHandle, BrokerError> {
        self.dispatch_calls.fetch_add(1, Ordering::SeqCst);
        // The tool-boundary idempotency key dedupes a re-fire: only a never-before-seen
        // key is a real world effect. A WM dispatch always carries it (the executor sets
        // it via `idempotency_token_for`); absent (a misuse) we conservatively count it.
        let key = request.idempotency_key.unwrap_or(*mote.id.as_bytes());
        if let Ok(mut applied) = self.applied_keys.lock() {
            if applied.insert(key) {
                self.net_effects.fetch_add(1, Ordering::SeqCst);
            }
        }
        Ok(BrokerHandle {
            staged_ref: Self::staged_ref(mote),
            capability: capability.clone(),
            capability_version: ToolVersion("0.1.0".into()),
        })
    }

    fn probe_readback(
        &self,
        _mote: &Mote,
        _warrant: &WarrantSpec,
        _capability: &ToolName,
        _probe: EffectRequest,
    ) -> Result<Option<BrokerHandle>, BrokerError> {
        Ok(None)
    }
}
