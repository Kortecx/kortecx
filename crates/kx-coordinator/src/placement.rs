//! [`LoadAwarePlacement`] — the P2.5 placement-v2 policy (D56).

use kx_mote::MoteId;
use kx_scheduler::{Placement, WorkerId};
use kx_warrant::ExecutorClass;

use crate::registry::WorkerRegistry;

/// Placement v2: route a ready Mote to the **least-loaded** registered worker that can
/// run it (matching `executor_class`), breaking equal-load ties by a **stable shard of
/// the `mote_id`** so idle workers split work evenly instead of piling onto the lowest
/// id. Implements the frozen [`kx_scheduler::Placement`] trait — the v2 policy swaps in
/// with **zero scheduler-core changes** (the P2.5 exit-gate obligation).
///
/// Built once per lease from a single registry [`snapshot`](WorkerRegistry::snapshot),
/// so it tracks **live load** (the worker reports `in_flight` via Heartbeat) without
/// re-locking the registry per Mote. Load-awareness is the scalability core; GPU-slot +
/// data-locality are forward seams behind the same `place` surface (they need
/// worker-reported capacity / result→worker tracking — P3/P5).
pub(crate) struct LoadAwarePlacement {
    /// Capable workers (class-matched), sorted by id, with their last-known load.
    candidates: Vec<(WorkerId, u32)>,
}

impl LoadAwarePlacement {
    /// Snapshot the registry once and keep the workers that can run `class`.
    pub(crate) fn new(registry: &dyn WorkerRegistry, class: ExecutorClass) -> Self {
        let mut candidates: Vec<(WorkerId, u32)> = registry
            .snapshot()
            .into_iter()
            .filter(|w| w.executor_class == class)
            .map(|w| (w.id, w.in_flight))
            .collect();
        candidates.sort_by_key(|(id, _)| id.0);
        Self { candidates }
    }
}

impl Placement for LoadAwarePlacement {
    fn place(&self, mote_id: &MoteId) -> WorkerId {
        // The polling worker is always capable (it registered with this class), so the
        // candidate set is non-empty in practice; the guard keeps `place` total.
        if self.candidates.is_empty() {
            return WorkerId(0);
        }
        let min_load = self
            .candidates
            .iter()
            .map(|(_, load)| *load)
            .min()
            .unwrap_or(0);
        let least_loaded: Vec<WorkerId> = self
            .candidates
            .iter()
            .filter(|(_, load)| *load == min_load)
            .map(|(id, _)| *id)
            .collect();
        least_loaded[shard_index(mote_id, least_loaded.len())]
    }
}

/// A stable index in `0..len` derived from the Mote's identity. `mote_id` is a BLAKE3
/// hash, so its leading bytes are uniformly distributed — an even shard.
fn shard_index(mote_id: &MoteId, len: usize) -> usize {
    let b = mote_id.as_bytes();
    let head = u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]);
    let len_u64 = u64::try_from(len).unwrap_or(u64::MAX);
    usize::try_from(head % len_u64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::InMemoryWorkerRegistry;

    const BWRAP: ExecutorClass = ExecutorClass::Bwrap;
    const MAC: ExecutorClass = ExecutorClass::MacOsSandbox;

    fn mote(byte: u8) -> MoteId {
        MoteId::from_bytes([byte; 32])
    }

    #[test]
    fn single_worker_gets_everything() {
        let reg = InMemoryWorkerRegistry::new();
        let w = reg.register(MAC, "a".into());
        let p = LoadAwarePlacement::new(&reg, MAC);
        assert_eq!(p.place(&mote(0)), w);
        assert_eq!(p.place(&mote(7)), w);
        assert_eq!(p.place(&mote(255)), w);
    }

    #[test]
    fn least_loaded_wins() {
        let reg = InMemoryWorkerRegistry::new();
        let busy = reg.register(MAC, "busy".into());
        let idle = reg.register(MAC, "idle".into());
        reg.heartbeat(busy, 1, 5).unwrap(); // busy: in_flight = 5
        reg.heartbeat(idle, 1, 0).unwrap(); // idle: in_flight = 0
        let p = LoadAwarePlacement::new(&reg, MAC);
        // Every Mote routes to the idle worker regardless of its shard.
        for b in 0..32u8 {
            assert_eq!(
                p.place(&mote(b)),
                idle,
                "byte {b} should route to the idle worker"
            );
        }
        assert_ne!(idle, busy);
    }

    #[test]
    fn equal_load_shards_across_workers() {
        let reg = InMemoryWorkerRegistry::new();
        let w0 = reg.register(MAC, "w0".into());
        let w1 = reg.register(MAC, "w1".into());
        // Both idle (in_flight defaults to 0) → ties broken by mote shard.
        let p = LoadAwarePlacement::new(&reg, MAC);
        // head([0;32]) = 0 (even) → w0; head([1;32]) is odd → w1.
        assert_eq!(p.place(&mote(0)), w0);
        assert_eq!(p.place(&mote(1)), w1);
        // Over many Motes, both workers are used (even split, not all-lowest-id).
        let mut seen_w0 = false;
        let mut seen_w1 = false;
        for b in 0..64u8 {
            match p.place(&mote(b)) {
                w if w == w0 => seen_w0 = true,
                w if w == w1 => seen_w1 = true,
                _ => unreachable!(),
            }
        }
        assert!(
            seen_w0 && seen_w1,
            "equal load must spread across both workers"
        );
    }

    #[test]
    fn only_class_matching_workers_are_candidates() {
        let reg = InMemoryWorkerRegistry::new();
        let bwrap = reg.register(BWRAP, "bwrap".into());
        let mac = reg.register(MAC, "mac".into());
        // A MacOsSandbox lease never routes to the Bwrap worker, and vice-versa.
        let p_mac = LoadAwarePlacement::new(&reg, MAC);
        let p_bwrap = LoadAwarePlacement::new(&reg, BWRAP);
        for b in 0..32u8 {
            assert_eq!(p_mac.place(&mote(b)), mac);
            assert_eq!(p_bwrap.place(&mote(b)), bwrap);
        }
    }

    #[test]
    fn no_capable_worker_is_total() {
        // No worker of the requested class → `place` stays total (never panics).
        let reg = InMemoryWorkerRegistry::new();
        reg.register(BWRAP, "bwrap".into());
        let p = LoadAwarePlacement::new(&reg, MAC);
        let _ = p.place(&mote(3));
    }
}
