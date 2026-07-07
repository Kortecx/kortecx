//! [`LeaseTracker`] — coordinator-side reschedule bookkeeping (D57).
//!
//! When a worker dies (P3.1 [`WorkerStatus::Dead`](crate::WorkerStatus)) the coordinator
//! must re-lease its in-flight PURE Motes and record each death as a journal fact, bounded
//! by a retry budget. This type holds the owner-thread state that makes that possible —
//! who currently holds an unresolved lease on what, which crash-failed Motes remain
//! rescheduleable, and how many times each has crash-failed. It is **pure bookkeeping**:
//! it writes no journal (the owner thread does) and reads no clock. Design: D57
//! (`distributed-reschedule.md`); recovery action it serves: D21 (`stuck-vs-dead.md`) §6/§7.
//!
//! A forward index (`worker → leased Motes`) plus a reverse index (`Mote → holding
//! workers`) keep every operation sub-linear in the worker fleet: resolving a commit
//! touches only the Motes' actual holders, not every worker — so reschedule bookkeeping
//! scales with in-flight work, not fleet size.

use std::collections::{BTreeMap, BTreeSet};

use kx_mote::MoteId;
use kx_scheduler::WorkerId;

/// Default PURE retry budget — the sanity ceiling from D21 §7 (D57 §4). A Mote whose
/// successive leasing workers all crash accumulates this many `Failed{WorkerCrashed}`
/// entries and is then terminal (no further replacement); workflow likely has a
/// structural problem.
pub(crate) const PURE_RETRY_BUDGET: u32 = 100;

/// Owner-thread reschedule bookkeeping (D57). See the module docs.
#[derive(Debug, Default)]
pub(crate) struct LeaseTracker {
    /// Forward: each worker's currently-outstanding (unresolved) leases.
    leases: BTreeMap<WorkerId, BTreeSet<MoteId>>,
    /// Reverse: each Mote's currently-holding workers (so `resolve_committed` is
    /// O(holders), not O(fleet)). An entry is dropped when its set empties.
    holders: BTreeMap<MoteId, BTreeSet<WorkerId>>,
    /// Crash-failed-but-still-rescheduleable Motes — the extra lease candidates
    /// `lease_ready` unions with the projection's ready-set (D57 §2).
    crash_failed: BTreeSet<MoteId>,
    /// Per-Mote count of worker-crash failures (the retry-budget counter, D57 §4).
    attempt_failures: BTreeMap<MoteId, u32>,
}

impl LeaseTracker {
    /// Record that `worker` now holds an outstanding lease on each of `motes`.
    pub(crate) fn record_lease(
        &mut self,
        worker: WorkerId,
        motes: impl IntoIterator<Item = MoteId>,
    ) {
        let forward = self.leases.entry(worker).or_default();
        for mote in motes {
            forward.insert(mote);
            self.holders.entry(mote).or_default().insert(worker);
        }
    }

    /// The crash-failed Motes that remain rescheduleable — unioned into the lease
    /// candidate set by `lease_ready`.
    pub(crate) fn rescheduleable(&self) -> &BTreeSet<MoteId> {
        &self.crash_failed
    }

    /// Workers that currently hold ≥1 outstanding lease (the reap scan set).
    pub(crate) fn leasing_workers(&self) -> Vec<WorkerId> {
        self.leases.keys().copied().collect()
    }

    /// Whether `worker` currently holds an outstanding lease on `mote` — the
    /// admission gate for a worker self-reported terminal failure (F4): a worker
    /// may only dead-letter work it was actually leased. O(holders of `mote`).
    pub(crate) fn is_held_by(&self, mote: MoteId, worker: WorkerId) -> bool {
        self.holders
            .get(&mote)
            .is_some_and(|workers| workers.contains(&worker))
    }

    /// Whether ANY worker currently holds an outstanding lease on `mote`. Used by
    /// the PR-9a settle-pass dead-letter (BUG-27) to leave an in-flight observation
    /// to its normal commit/fail lifecycle and dead-letter ONLY a wedged
    /// (materialized-but-never-leased) one — avoiding a race with a worker about to
    /// commit. O(1) via the reverse index.
    pub(crate) fn is_leased(&self, mote: MoteId) -> bool {
        self.holders.contains_key(&mote)
    }

    /// Whether `mote` is currently leased by a live worker OTHER than `worker` — the
    /// pool admission gate. `lease_ready` skips such a Mote so two pool workers
    /// never redundantly run the same one, WHILE a worker may always re-lease its OWN
    /// outstanding holds (the mid-batch-error self-heal). This makes a single-worker
    /// serve byte-identical: a lone worker is never an "other" holder, so the gate
    /// never fires. Dead workers' holds are dropped by `reap_dead_workers` BEFORE
    /// `lease_ready` runs, so this reflects only LIVE holds — a crashed worker's Mote
    /// is re-offered via `record_crash`/`rescheduleable` after the liveness window, not
    /// stranded. O(holders of `mote`).
    pub(crate) fn is_leased_by_other(&self, mote: MoteId, worker: WorkerId) -> bool {
        self.holders
            .get(&mote)
            .is_some_and(|workers| workers.iter().any(|w| *w != worker))
    }

    /// Remove and return `worker`'s outstanding leases (used when reaping a dead
    /// worker), keeping the reverse index consistent.
    pub(crate) fn take_leases(&mut self, worker: WorkerId) -> BTreeSet<MoteId> {
        let leased = self.leases.remove(&worker).unwrap_or_default();
        for mote in &leased {
            if let Some(holders) = self.holders.get_mut(mote) {
                holders.remove(&worker);
                if holders.is_empty() {
                    self.holders.remove(mote);
                }
            }
        }
        leased
    }

    /// Record a worker-crash failure of `mote` against `budget`. Returns `true` iff
    /// `mote` remains rescheduleable (failures still `< budget`), `false` iff the
    /// budget is now exhausted (terminal — caller stops re-leasing it). The caller
    /// writes the `Failed{WorkerCrashed}` journal entry regardless (the death is a
    /// fact either way, D21 §11).
    pub(crate) fn record_crash(&mut self, mote: MoteId, budget: u32) -> bool {
        let failures = self.attempt_failures.entry(mote).or_insert(0);
        *failures += 1;
        if *failures < budget {
            self.crash_failed.insert(mote);
            true
        } else {
            // Terminal: never lease it again; drop it from the candidate set.
            self.crash_failed.remove(&mote);
            false
        }
    }

    /// Resolve `mote` — a commit (first-wins) clears it from **all** tracking. O(holders
    /// of `mote`) via the reverse index, not O(fleet).
    pub(crate) fn resolve_committed(&mut self, mote: MoteId) {
        if let Some(holders) = self.holders.remove(&mote) {
            for worker in holders {
                if let Some(forward) = self.leases.get_mut(&worker) {
                    forward.remove(&mote);
                    if forward.is_empty() {
                        self.leases.remove(&worker);
                    }
                }
            }
        }
        self.crash_failed.remove(&mote);
        self.attempt_failures.remove(&mote);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mote(b: u8) -> MoteId {
        MoteId::from_bytes([b; 32])
    }

    #[test]
    fn lease_then_resolve_clears_all_tracking() {
        let mut t = LeaseTracker::default();
        t.record_lease(WorkerId(1), [mote(1), mote(2)]);
        t.record_lease(WorkerId(2), [mote(2)]); // mote(2) double-leased (poll-only)
        assert_eq!(t.leasing_workers(), vec![WorkerId(1), WorkerId(2)]);

        // Committing mote(2) clears it from BOTH holders; worker 2 had only it → gone.
        t.resolve_committed(mote(2));
        assert_eq!(
            t.leasing_workers(),
            vec![WorkerId(1)],
            "worker with no remaining leases is dropped"
        );
        // worker 1 still holds mote(1).
        let w1 = t.take_leases(WorkerId(1));
        assert_eq!(w1.into_iter().collect::<Vec<_>>(), vec![mote(1)]);
        assert!(t.leasing_workers().is_empty());
    }

    #[test]
    fn crash_makes_rescheduleable_until_budget_exhausted() {
        let mut t = LeaseTracker::default();
        let m = mote(7);
        // budget = 3 → first two crashes stay rescheduleable, the third is terminal.
        assert!(t.record_crash(m, 3));
        assert!(t.rescheduleable().contains(&m));
        assert!(t.record_crash(m, 3));
        assert!(t.rescheduleable().contains(&m));
        assert!(!t.record_crash(m, 3), "third crash exhausts budget=3");
        assert!(
            !t.rescheduleable().contains(&m),
            "terminal Mote is no longer rescheduleable"
        );
    }

    #[test]
    fn commit_after_crash_clears_the_retry_counter() {
        let mut t = LeaseTracker::default();
        let m = mote(9);
        assert!(t.record_crash(m, 100));
        assert!(t.rescheduleable().contains(&m));
        t.resolve_committed(m);
        assert!(
            !t.rescheduleable().contains(&m),
            "a commit resolves the crash-failed Mote"
        );
        // A fresh crash after a commit starts the count over (counter was cleared).
        for _ in 0..50 {
            assert!(t.record_crash(m, 100));
        }
    }

    #[test]
    fn take_leases_of_unknown_worker_is_empty() {
        let mut t = LeaseTracker::default();
        assert!(t.take_leases(WorkerId(42)).is_empty());
    }

    #[test]
    fn is_leased_by_other_is_the_pool_admission_gate() {
        // The lease_ready gate. A lone holder never sees an "other" holder
        // (pool=1 byte-identical); a second worker's hold makes it true for the first.
        let mut t = LeaseTracker::default();
        let m = mote(5);
        assert!(
            !t.is_leased_by_other(m, WorkerId(1)),
            "unheld: no other holder"
        );
        t.record_lease(WorkerId(1), [m]);
        // Held ONLY by worker 1 → not "leased by other" for worker 1 (self-heal), but
        // IS for any other worker (the partitioning that makes pool>1 real).
        assert!(
            !t.is_leased_by_other(m, WorkerId(1)),
            "a worker may re-lease its OWN hold (mid-batch-error self-heal)"
        );
        assert!(
            t.is_leased_by_other(m, WorkerId(2)),
            "worker 2 must skip a Mote worker 1 already holds"
        );
        // After worker 1 crashes + is reaped (take_leases), the hold clears → the Mote
        // becomes available again (re-offered via rescheduleable), not stranded.
        t.take_leases(WorkerId(1));
        assert!(
            !t.is_leased_by_other(m, WorkerId(2)),
            "a reaped worker's hold is cleared → the Mote is leasable again"
        );
    }
}
