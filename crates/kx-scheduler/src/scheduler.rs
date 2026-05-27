//! [`Scheduler`] — the per-run dispatch state machine.
//!
//! On each [`Scheduler::tick`] the scheduler asks the
//! [`Projection`](kx_projection::Projection) which submitted Motes are ready
//! (parents all `Committed-and-not-Repudiated`), routes each through the
//! [`Placement`] policy, and hands the Mote + warrant to the
//! [`kx_executor::MoteExecutor`].
//!
//! The scheduler owns no journal handle and no content-store handle; it
//! never appends to the journal, never reconstructs Mote inputs, never
//! looks up parent `result_ref`s. The executor reads what it needs from the
//! journal directly (see `kx-executor/src/lifecycle.rs` recovery path).

use std::collections::BTreeMap;

use kx_executor::{MoteExecutionResult, MoteExecutor, MoteExecutorError};
use kx_mote::{Mote, MoteId};
use kx_projection::{Projection, RegisterMote};
use kx_warrant::WarrantSpec;

use crate::errors::SchedulerError;
use crate::placement::Placement;
use crate::worker::WorkerId;

/// One row of [`DispatchSummary`]: the outcome of a single
/// [`MoteExecutor::run`] invocation during a tick.
///
/// The scheduler does not interpret `result` — the caller (P1.13 binary or
/// test harness) is responsible for translating successes into `Committed`
/// journal entries and failures into `Failed` entries, then folding those
/// into the [`Projection`] before the next tick.
#[derive(Debug)]
pub struct DispatchedMote {
    /// The Mote that was dispatched.
    pub mote_id: MoteId,
    /// Where the [`Placement`] routed it.
    pub worker: WorkerId,
    /// The executor's typed outcome. The scheduler surfaces this verbatim.
    pub result: Result<MoteExecutionResult, MoteExecutorError>,
}

/// Per-tick outcome — one [`DispatchedMote`] for each Mote dispatched.
///
/// An empty `dispatched` vector means the tick was a no-op (no ready Motes
/// in the projection's submitted set). Caller may sleep and tick again, or
/// fold new journal entries first.
#[derive(Debug, Default)]
pub struct DispatchSummary {
    /// One row per Mote dispatched during the tick.
    pub dispatched: Vec<DispatchedMote>,
}

/// The per-run scheduler — placement + pending submitted Motes.
///
/// **No journal handle. No content-store handle. No projection ownership.**
/// The caller (P1.13 binary or test harness) owns the [`Projection`] and
/// passes it in on each call. This makes "never writes the journal" and
/// "reads only the projection" structural: the scheduler crate cannot
/// reach a `Journal` because the type is not in its production deps.
///
/// # End-to-end usage
///
/// ```
/// use kx_mote::MoteId;
/// use kx_projection::Projection;
/// use kx_scheduler::{LocalPlacement, Scheduler};
///
/// // The caller owns the projection (P1.13 runtime binary's role).
/// let projection = Projection::new();
/// let scheduler = Scheduler::new(LocalPlacement);
///
/// // Brand-new scheduler has nothing pending and nothing ready.
/// assert_eq!(scheduler.pending_count(), 0);
/// assert!(projection.ready_set().is_empty());
/// # let _ = MoteId::from_bytes([0u8; 32]);
/// ```
///
/// Full submit + tick flows live in `tests/integration_dag_ordering.rs`
/// (linear-chain + diamond + multi-root cases).
#[derive(Debug)]
pub struct Scheduler<P: Placement> {
    placement: P,
    pending: BTreeMap<MoteId, (Mote, WarrantSpec)>,
}

impl<P: Placement> Scheduler<P> {
    /// Construct a scheduler with the given placement policy.
    ///
    /// ```
    /// use kx_scheduler::{LocalPlacement, Scheduler};
    /// let s = Scheduler::new(LocalPlacement);
    /// assert_eq!(s.pending_count(), 0);
    /// ```
    #[must_use]
    pub fn new(placement: P) -> Self {
        Self {
            placement,
            pending: BTreeMap::new(),
        }
    }

    /// Number of Motes currently in the pending map.
    ///
    /// A Mote enters the map on [`Scheduler::submit`] and leaves on
    /// [`Scheduler::tick`] once dispatched. This count is for diagnostics
    /// only — orchestration decisions go through the [`Projection`].
    #[must_use]
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// Register a workflow-declared Mote with the projection and store it
    /// for later dispatch.
    ///
    /// Two side effects:
    ///
    /// 1. Calls `projection.register_mote(...)` so the projection's
    ///    `state_of(mote.id)` returns [`kx_projection::MoteState::Pending`]
    ///    and `ready_set` will yield this Mote once its parents commit.
    /// 2. Inserts `(mote, warrant)` into the scheduler's pending map keyed
    ///    on `mote.id`.
    ///
    /// Returns [`SchedulerError::DuplicateSubmission`] if the id is already
    /// in the pending map (i.e., submitted twice before dispatch). The
    /// projection's `register_mote` itself is idempotent (per
    /// `kx-projection/src/projection.rs:84-101`), so re-submitting a Mote
    /// after it has been dispatched is fine — the projection just updates
    /// the declared info, and the scheduler stores the new (mote, warrant)
    /// pair.
    pub fn submit(
        &mut self,
        mote: Mote,
        warrant: WarrantSpec,
        projection: &mut Projection,
    ) -> Result<(), SchedulerError> {
        if self.pending.contains_key(&mote.id) {
            return Err(SchedulerError::DuplicateSubmission(mote.id));
        }
        projection.register_mote(RegisterMote {
            mote_id: mote.id,
            nd_class: mote.def.nd_class,
            effect_pattern: mote.def.effect_pattern,
            critic_for: mote.def.critic_for,
            is_topology_shaper: mote.def.is_topology_shaper,
            parents: mote.parents.clone(),
        });
        self.pending.insert(mote.id, (mote, warrant));
        Ok(())
    }

    /// Drive one round of dispatch.
    ///
    /// 1. Read `projection.ready_set()` — the canonical "parents all
    ///    Committed-and-not-Repudiated AND WORLD-MUTATING parents promoted"
    ///    filter, sourced from `projection.md` §7.
    /// 2. For each ready `MoteId` that is also in the scheduler's pending
    ///    map: ask placement which worker, then hand the Mote + warrant to
    ///    `executor.run(&mote, &warrant, None)`.
    /// 3. Collect every per-Mote outcome into [`DispatchSummary`].
    ///
    /// Note the `None` environment argument: P1 dispatches without an
    /// explicit rootfs ref; the executor's pre-spawn step derives rootfs
    /// from `warrant.environment_ref` when that lands at PR 11+.
    ///
    /// The dispatched Mote is removed from the pending map regardless of
    /// the executor's outcome. The caller is responsible for translating
    /// `Ok(MoteExecutionResult)` into a `Committed` journal entry and
    /// `Err(MoteExecutorError)` into a `Failed` entry; the scheduler does
    /// not journal anything itself.
    pub fn tick<E: MoteExecutor>(
        &mut self,
        projection: &Projection,
        executor: &E,
    ) -> Result<DispatchSummary, SchedulerError> {
        let ready = projection.ready_set();
        let mut dispatched = Vec::new();
        for mote_id in ready {
            // Skip ready ids the scheduler has no pending entry for — those
            // were either submitted to a different scheduler instance or
            // already dispatched in a prior tick.
            let Some((mote, warrant)) = self.pending.remove(&mote_id) else {
                continue;
            };
            let worker = self.placement.place(&mote_id);
            let result = executor.run(&mote, &warrant, None);
            dispatched.push(DispatchedMote {
                mote_id,
                worker,
                result,
            });
        }
        Ok(DispatchSummary { dispatched })
    }
}
