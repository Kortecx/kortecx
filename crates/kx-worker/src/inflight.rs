//! The in-process register of effects that are **still running**.
//!
//! Moving `broker.dispatch` onto `spawn_blocking` is what finally makes the per-Mote
//! wall-clock deadline ([`crate::WorkerError::ExecutionTimedOut`]) fire: the awaited
//! `JoinHandle` is a real await point, so `tokio::time::timeout` can observe the elapsed
//! timer. Before this change the dispatch was a synchronous call inside the future, `Timeout`
//! polled it to completion in a single poll, and the deadline was unreachable.
//!
//! That fix opens a hazard the old code could not have: **`spawn_blocking` is not
//! cancellable.** Dropping its `JoinHandle` abandons the *result*, not the *work* — the
//! closure runs to completion on its blocking thread. `ExecutionTimedOut` classifies
//! `TransientInfra`, so the coordinator re-offers the Mote and the worker would fire the
//! SAME effect again while the abandoned one is still in flight. The D38 §1 tool-boundary
//! key dedups that at the world boundary for token-class capabilities, but it is the only
//! thing that would, and it is not universal.
//!
//! So the worker refuses to re-dispatch a Mote whose abandoned effect has not returned.
//! The registry is in-memory and off the truth path (like `Worker::attempts`): a restart
//! resets it harmlessly, because a restart also destroys the orphaned blocking threads it
//! was tracking. The coordinator's durable `EffectStaged` hint + R-13 remain the
//! cross-process guard; this covers only the in-process window the deadline opens.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use kx_mote::MoteId;

/// The set of Motes whose effect dispatch has begun and not yet returned.
///
/// Cheap to clone (one `Arc`), so every concurrently-dispatched item in a lease batch
/// shares one view.
#[derive(Clone, Default)]
pub(crate) struct InFlightEffects {
    running: Arc<Mutex<BTreeSet<MoteId>>>,
}

impl InFlightEffects {
    /// Claim `mote_id` for dispatch. Returns a guard on success; `None` when an earlier
    /// dispatch of the SAME Mote is still running (the caller must refuse, not fire).
    ///
    /// A poisoned lock is treated as "claim refused" rather than unwrapped: refusing to
    /// fire is the fail-closed direction for a world-mutating effect.
    pub(crate) fn claim(&self, mote_id: MoteId) -> Option<EffectGuard> {
        let mut running = self.running.lock().ok()?;
        if !running.insert(mote_id) {
            return None;
        }
        Some(EffectGuard {
            registry: self.clone(),
            mote_id,
        })
    }

    fn release(&self, mote_id: MoteId) {
        if let Ok(mut running) = self.running.lock() {
            running.remove(&mote_id);
        }
    }
}

/// Releases its Mote's claim on drop.
///
/// **The guard is moved INTO the `spawn_blocking` closure**, which is what makes this
/// correct: the closure — and therefore the drop — happens when the effect genuinely
/// finishes, not when a timed-out caller stops waiting for it.
pub(crate) struct EffectGuard {
    registry: InFlightEffects,
    mote_id: MoteId,
}

impl Drop for EffectGuard {
    fn drop(&mut self) {
        self.registry.release(self.mote_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(seed: u8) -> MoteId {
        MoteId::from_bytes([seed; 32])
    }

    /// A second claim on the same Mote is refused while the first is live, and admitted
    /// again once the guard drops — the whole contract in one test.
    #[test]
    fn a_live_claim_blocks_a_second_dispatch_of_the_same_mote() {
        let registry = InFlightEffects::default();
        let first = registry.claim(id(1)).expect("the first claim is admitted");
        assert!(
            registry.claim(id(1)).is_none(),
            "a Mote whose effect is still running must NOT be re-dispatched"
        );
        // A DIFFERENT Mote is unaffected — this is a per-Mote guard, not a global lock.
        let other = registry
            .claim(id(2))
            .expect("a distinct Mote still dispatches");
        drop(first);
        assert!(
            registry.claim(id(1)).is_some(),
            "once the abandoned effect returns, the Mote is dispatchable again"
        );
        drop(other);
    }
}
