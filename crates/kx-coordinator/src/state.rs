//! The orchestration core — a single owner thread holding the run's journal,
//! projection, and hosted scheduler, driven by a command channel.
//!
//! ## Why a thread, not a shared mutex
//!
//! [`kx_projection::Projection`] holds a non-`Send` `Box<dyn TopologyMaterializer>`,
//! so it cannot live inside a `Send + Sync` tonic service. Rather than refactor
//! the P1 `kx-projection` crate (Rule 1 — upstream refactors are their own PR), the
//! coordinator confines the projection (and the journal and scheduler) to one owner
//! thread. The async RPC handlers send [`Command`]s over an `mpsc` channel and await
//! a `oneshot` reply. This also makes the **D40 sole-writer invariant structural**:
//! there is exactly one thread, and it is the only code that ever appends.
//!
//! ## Hosting the scheduler verbatim (thesis test)
//!
//! Registration routes through [`kx_scheduler::Scheduler::submit`] exactly as the
//! single-node `kx-runtime` engine does — the scheduler source is unchanged.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use kx_content::{ContentStore, LocalFsContentStore};
use kx_journal::{
    FailureReason, IdempotencyClassTag, Journal, JournalEntry, RepudiationReason,
    ResolvedCapabilityRecord, ResolvedKindTag, INSTANCE_ID_LEN,
};
use kx_mote::{Mote, MoteId, NdClass};
use kx_projection::{MoteState, Projection};
use kx_refusal::{validate_mote_submission, ToolResolution};
use kx_scheduler::{LocalPlacement, Placement, Scheduler, SchedulerError, WorkerId};
use kx_tool_registry::{IdempotencyClass, ToolKind, ToolRegistry, ToolResolutionEvent};
use kx_warrant::{warrant_ref_of, ExecutorClass, WarrantSpec};
use tokio::sync::{mpsc, oneshot};

use crate::clock::Clock;
use crate::commit::CommitProposal;
use crate::error::CoordinatorError;
use crate::nonce::RunNonceSource;
use crate::placement::LoadAwarePlacement;
use crate::registry::{WorkerRegistry, WorkerStatus};
use crate::repudiation::{
    cascade_repudiation_entries, RepudiationError, RepudiationOutcome, DEFAULT_CASCADE_CEILING,
};
use crate::reschedule::{LeaseTracker, PURE_RETRY_BUDGET};

/// Reporter id stamped on a coordinator-written `Failed{WorkerCrashed}` entry (D57 §3
/// / D21 §6). The coordinator is the reporter when it observes a worker's death, as
/// distinct from a worker self-reporting; `0` is the reserved coordinator id.
const COORDINATOR_REPORTER_ID: u128 = 0;

/// Bound on in-flight commands queued to the orchestration core. A bounded
/// channel applies backpressure: when the core is saturated, `dispatch` awaits
/// instead of letting an unbounded queue grow without limit under a flood of RPCs.
const COMMAND_BUFFER: usize = 1024;

/// Max commands the core drains per wake. Consecutive `Commit`s within a drain
/// coalesce into one journal transaction (group commit); this bounds the size of
/// that transaction.
const MAX_DRAIN: usize = 256;

/// Outcome of a `SubmitMote`: the canonically re-derived id, whether it was a
/// duplicate (idempotent re-submit before commit), and the registered run's
/// `instance_id` if the run was registered (M1.2/D64 — the resume key surfaced
/// on the wire; `None` for an unregistered run, where M1.2 captures no metadata
/// and the worker falls back to the MoteId-only token).
#[derive(Debug, Clone, Copy)]
pub(crate) struct SubmitOutcome {
    pub(crate) mote_id: MoteId,
    pub(crate) duplicate: bool,
    pub(crate) instance_id: Option<[u8; INSTANCE_ID_LEN]>,
}

/// Leased work plus the run's `instance_id` (if registered) — the `LeaseWork`
/// reply shape (M1.2): the worker derives the run-scoped idempotency token from
/// the `instance_id`.
type LeasedWork = (Vec<(Mote, WarrantSpec)>, Option<[u8; INSTANCE_ID_LEN]>);

/// Outcome of a `ReportCommit`: the journal-assigned seq and whether the commit
/// was newly appended or a dedup-by-key hit (first-wins).
#[derive(Debug, Clone, Copy)]
pub(crate) struct CommitApplied {
    pub(crate) committed_seq: u64,
    pub(crate) already_committed: bool,
}

/// Messages the async handlers send to the owner thread.
pub(crate) enum Command {
    Submit {
        mote: Box<Mote>,
        warrant: Box<WarrantSpec>,
        // M1.3/D38 §2c: the per-Mote opt-in to dispatch an AtLeastOnce WM tool.
        accept_at_least_once: bool,
        // M1.3: the submit is now fallible — registration-before-submit + the
        // submission-refusal predicates can reject before anything is written.
        reply: oneshot::Sender<Result<SubmitOutcome, CoordinatorError>>,
    },
    Commit {
        proposal: Box<CommitProposal>,
        reply: oneshot::Sender<Result<CommitApplied, CoordinatorError>>,
    },
    StateOf {
        mote_id: MoteId,
        reply: oneshot::Sender<MoteState>,
    },
    CommittedCount {
        reply: oneshot::Sender<usize>,
    },
    ReadySet {
        reply: oneshot::Sender<Vec<MoteId>>,
    },
    LeaseWork {
        worker: WorkerId,
        executor_class: ExecutorClass,
        max: usize,
        // M1.2: the leased work PLUS the run's `instance_id` (if registered), so
        // the worker can derive the run-scoped cross-boundary idempotency token.
        reply: oneshot::Sender<LeasedWork>,
    },
    ReadEntries {
        since_seq: u64,
        max: usize,
        reply: oneshot::Sender<Result<(Vec<JournalEntry>, u64), CoordinatorError>>,
    },
    Repudiate {
        target: MoteId,
        reason: RepudiationReason,
        repudiator_id: u128,
        reply: oneshot::Sender<Result<RepudiationOutcome, RepudiationError>>,
    },
    ReportEffectStaged {
        mote_id: MoteId,
        idempotency_key: [u8; 32],
        reply: oneshot::Sender<Result<u64, CoordinatorError>>,
    },
    RegisterRun {
        recipe_fingerprint: [u8; 32],
        reply: oneshot::Sender<Result<[u8; INSTANCE_ID_LEN], CoordinatorError>>,
    },
    RunRegistration {
        reply: oneshot::Sender<Option<([u8; INSTANCE_ID_LEN], [u8; 32])>>,
    },
    RunResolvedVersions {
        reply: oneshot::Sender<Vec<kx_projection::RunResolvedVersions>>,
    },
}

/// Handle to the orchestration core. Cloneable + `Send + Sync` (it is just the
/// channel sender), so the gRPC service that holds it is too.
#[derive(Clone)]
pub(crate) struct CoreHandle {
    commands: mpsc::Sender<Command>,
}

impl CoreHandle {
    /// Spawn the owner thread, taking sole ownership of `journal`. When `store` is
    /// `Some`, the core verifies `store.contains(result_ref)` before committing a
    /// proposal (D55 phantom-ref guard); when `None`, commits are not content-checked
    /// (the P2.2/P2.3 behavior). `registry` is the live worker view the lease-time
    /// placement policy ranks over (D56).
    pub(crate) fn spawn<J: Journal + Send + 'static>(
        journal: J,
        store: Option<Arc<LocalFsContentStore>>,
        registry: Arc<dyn WorkerRegistry>,
        clock: Arc<dyn Clock>,
        nonce: Arc<dyn RunNonceSource>,
        tool_registry: Arc<dyn ToolRegistry>,
    ) -> Self {
        let (commands, inbox) = mpsc::channel(COMMAND_BUFFER);
        std::thread::spawn(move || {
            core_loop(
                &journal,
                store.as_deref(),
                &*registry,
                &*clock,
                &*nonce,
                &*tool_registry,
                inbox,
            );
        });
        Self { commands }
    }

    pub(crate) async fn submit(
        &self,
        mote: Mote,
        warrant: WarrantSpec,
        accept_at_least_once: bool,
    ) -> Result<SubmitOutcome, CoordinatorError> {
        let (reply, response) = oneshot::channel();
        self.dispatch(Command::Submit {
            mote: Box::new(mote),
            warrant: Box::new(warrant),
            accept_at_least_once,
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| CoordinatorError::CoreUnavailable)?
    }

    pub(crate) async fn commit(
        &self,
        proposal: CommitProposal,
    ) -> Result<CommitApplied, CoordinatorError> {
        let (reply, response) = oneshot::channel();
        self.dispatch(Command::Commit {
            proposal: Box::new(proposal),
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| CoordinatorError::CoreUnavailable)?
    }

    pub(crate) async fn state_of(&self, mote_id: MoteId) -> Result<MoteState, CoordinatorError> {
        let (reply, response) = oneshot::channel();
        self.dispatch(Command::StateOf { mote_id, reply }).await?;
        response
            .await
            .map_err(|_| CoordinatorError::CoreUnavailable)
    }

    pub(crate) async fn committed_count(&self) -> Result<usize, CoordinatorError> {
        let (reply, response) = oneshot::channel();
        self.dispatch(Command::CommittedCount { reply }).await?;
        response
            .await
            .map_err(|_| CoordinatorError::CoreUnavailable)
    }

    pub(crate) async fn ready_set(&self) -> Result<Vec<MoteId>, CoordinatorError> {
        let (reply, response) = oneshot::channel();
        self.dispatch(Command::ReadySet { reply }).await?;
        response
            .await
            .map_err(|_| CoordinatorError::CoreUnavailable)
    }

    /// Lease up to `max` ready PURE Motes runnable on `executor_class`, returning
    /// each with the warrant it was submitted under (the dispatch surface the worker
    /// pulls). Placement v2 (D56) prefers Motes the load-aware policy routes to
    /// `worker`, then fills to `max` with the rest so a live poller never idles while
    /// ready work exists. No lease/lock is held: double execution is harmless under
    /// the journal's dedupe-by-key, and worker-death reschedule is reserved for P3.
    pub(crate) async fn lease_work(
        &self,
        worker: WorkerId,
        executor_class: ExecutorClass,
        max: usize,
    ) -> Result<LeasedWork, CoordinatorError> {
        let (reply, response) = oneshot::channel();
        self.dispatch(Command::LeaseWork {
            worker,
            executor_class,
            max,
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| CoordinatorError::CoreUnavailable)
    }

    /// Read up to `max` committed journal entries with seq `> since_seq`, plus the
    /// cursor (`next_seq`) to pass on the next poll. The distributed-read surface
    /// (D55): peers pull committed-entry deltas and fold a local read model rather
    /// than round-tripping per result.
    pub(crate) async fn read_entries(
        &self,
        since_seq: u64,
        max: usize,
    ) -> Result<(Vec<JournalEntry>, u64), CoordinatorError> {
        let (reply, response) = oneshot::channel();
        self.dispatch(Command::ReadEntries {
            since_seq,
            max,
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| CoordinatorError::CoreUnavailable)?
    }

    /// Repudiate `target` and cascade the poison-invalidation to its committed downstream
    /// consumers (D22 / P0.7). Runs on the sole-writer thread: it computes the cascade
    /// against the live projection and appends the `Repudiated` batch atomically.
    pub(crate) async fn repudiate(
        &self,
        target: MoteId,
        reason: RepudiationReason,
        repudiator_id: u128,
    ) -> Result<RepudiationOutcome, RepudiationError> {
        let (reply, response) = oneshot::channel();
        self.dispatch(Command::Repudiate {
            target,
            reason,
            repudiator_id,
            reply,
        })
        .await
        .map_err(|_| RepudiationError::CoreUnavailable)?;
        response
            .await
            .map_err(|_| RepudiationError::CoreUnavailable)?
    }

    /// Record a WORLD-MUTATING Mote's staged-intent (D58): append an `EffectStaged`
    /// entry through the sole writer and return its seq. The worker calls this BEFORE
    /// firing the effect, so on worker death the coordinator's projection has the
    /// recovery hint. Dedupes by key (D15) — a re-stage on recovery returns the seq.
    pub(crate) async fn report_effect_staged(
        &self,
        mote_id: MoteId,
        idempotency_key: [u8; 32],
    ) -> Result<u64, CoordinatorError> {
        let (reply, response) = oneshot::channel();
        self.dispatch(Command::ReportEffectStaged {
            mote_id,
            idempotency_key,
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| CoordinatorError::CoreUnavailable)?
    }

    /// Register the run (M1.1, D64): assign a fresh, journaled, immutable
    /// `instance_id` and append the seq=1 `RunRegistered` fact. The client calls
    /// this once before submitting any Mote. Idempotent — a second call on the
    /// same run returns the existing `instance_id`. Errors with `RunAlreadyStarted`
    /// if the run has already begun without registration.
    pub(crate) async fn register_run(
        &self,
        recipe_fingerprint: [u8; 32],
    ) -> Result<[u8; INSTANCE_ID_LEN], CoordinatorError> {
        let (reply, response) = oneshot::channel();
        self.dispatch(Command::RegisterRun {
            recipe_fingerprint,
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| CoordinatorError::CoreUnavailable)?
    }

    /// The registered run identity (D64) as `(instance_id, recipe_fingerprint)`,
    /// or `None` if the run has not been registered. Read from the folded
    /// projection — on recovery this is the journaled fact, never recomputed.
    pub(crate) async fn run_registration(
        &self,
    ) -> Result<Option<([u8; INSTANCE_ID_LEN], [u8; 32])>, CoordinatorError> {
        let (reply, response) = oneshot::channel();
        self.dispatch(Command::RunRegistration { reply }).await?;
        response
            .await
            .map_err(|_| CoordinatorError::CoreUnavailable)
    }

    /// The resolved-version run metadata captured so far (M1.2, D79) — one record
    /// per resolved capability. Read from the folded projection; off the truth
    /// path (never gates anything).
    pub(crate) async fn run_resolved_versions(
        &self,
    ) -> Result<Vec<kx_projection::RunResolvedVersions>, CoordinatorError> {
        let (reply, response) = oneshot::channel();
        self.dispatch(Command::RunResolvedVersions { reply })
            .await?;
        response
            .await
            .map_err(|_| CoordinatorError::CoreUnavailable)
    }

    async fn dispatch(&self, command: Command) -> Result<(), CoordinatorError> {
        self.commands
            .send(command)
            .await
            .map_err(|_| CoordinatorError::CoreUnavailable)
    }
}

/// Recover the read-side from the durable journal at startup: fold the log into a
/// projection, read the current seq watermark, and seed the admission set from the
/// already-committed Motes (on recovery, committed Motes count as admitted). Returns
/// `None` (after logging) on a durable-layer fault, which stops the core — the
/// journal stays the truth and a restart re-folds from it.
fn recover<J: Journal>(journal: &J) -> Option<(Projection, u64, BTreeSet<MoteId>)> {
    let projection = match Projection::from_journal(journal) {
        Ok(projection) => projection,
        Err(error) => {
            tracing::error!(%error, "coordinator core failed to recover the projection");
            return None;
        }
    };
    let folded_through = match journal.current_seq() {
        Ok(seq) => seq,
        Err(error) => {
            tracing::error!(%error, "coordinator core failed to read the journal seq");
            return None;
        }
    };
    let submitted = projection
        .snapshot()
        .iter_motes()
        .map(|(id, _)| id)
        .collect();
    Some((projection, folded_through, submitted))
}

/// A `LeaseWork` request's parameters, bundled so [`serve_lease`] stays within the
/// argument-count budget.
struct LeaseReq {
    worker: WorkerId,
    executor_class: ExecutorClass,
    max: usize,
}

/// Serve one `LeaseWork` poll (D57): reap dead workers first so their in-flight Motes
/// re-enter the candidate set for *this* poll, select up to `req.max` runnable Motes
/// (ready ∪ rescheduleable), and record the new lease assignments for the next reap.
/// Returns the leased work plus the run's `instance_id` (M1.2: the worker derives
/// the run-scoped idempotency token from it; `None` for an unregistered run).
fn serve_lease<J: Journal>(
    journal: &J,
    projection: &mut Projection,
    folded_through: &mut u64,
    registry: &dyn WorkerRegistry,
    dispatch: &mut Dispatch,
    req: &LeaseReq,
) -> LeasedWork {
    reap_dead_workers(
        journal,
        projection,
        folded_through,
        registry,
        &mut dispatch.tracker,
    );
    let items = lease_ready(
        projection,
        &dispatch.defs,
        registry,
        dispatch.tracker.rescheduleable(),
        req.worker,
        req.executor_class,
        req.max,
    );
    dispatch
        .tracker
        .record_lease(req.worker, items.iter().map(|(mote, _)| mote.id));
    // M1.2 (O(1) off-DAG read): the run the leased work belongs to.
    let instance_id = projection.run_registration().map(|(id, _)| id);
    (items, instance_id)
}

/// Select up to `max` ready Motes for `worker` to run. Candidates are ready
/// (parents-all-committed) ∩ matching the worker's backend (`executor_class`). **D58
/// (P3.6) lifts the PURE-only restriction**: WORLD-MUTATING + READ-ONLY-NONDET Motes are
/// now leasable (the worker stages its intent via `ReportEffectStaged` before firing,
/// then proposes the commit). **R-13 under distribution (P3.6c):** a crash-failed *re-offer*
/// of a non-PURE Mote is gated on the recovery oracle (`redispatch_admissible`) — without a
/// durable `EffectStaged` hint the coordinator refuses to re-lease it (the effect may have
/// fired; re-dispatch would double it), exactly as single-node `pick_next` / the executor's
/// R-13 refuse. The executor's R-13 is single-node only, so the coordinator MUST enforce it
/// itself on the distributed re-dispatch path. Fresh `ready_set` Motes (first dispatch) are
/// ungated. The D55 phantom-ref guard backstops the commit. Placement v2 (D56) then orders them:
/// `worker`'s placement-preferred Motes come **first**, the rest **fill to `max`**
/// (starvation-free; double execution stays harmless under dedup, D54).
fn lease_ready(
    projection: &Projection,
    submitted_defs: &BTreeMap<MoteId, (Mote, WarrantSpec)>,
    registry: &dyn WorkerRegistry,
    rescheduleable: &BTreeSet<MoteId>,
    worker: WorkerId,
    executor_class: ExecutorClass,
    max: usize,
) -> Vec<(Mote, WarrantSpec)> {
    let placement = LoadAwarePlacement::new(registry, executor_class);
    let mut preferred: Vec<MoteId> = Vec::new();
    let mut rest: Vec<MoteId> = Vec::new();
    let mut seen: BTreeSet<MoteId> = BTreeSet::new();
    // Candidates = the projection's ready-set (Pending, parents committed) ∪ the
    // crash-failed-but-rescheduleable set (D57 §2): a crash-failed Mote is `Failed` in
    // the projection so it left the ready-set, but its parents are still committed
    // (commits are permanent) so it is genuinely re-runnable. A Mote already committed
    // since being crash-failed is skipped (first-wins resolved it).
    //
    // The rescheduleable (re-offer) half is gated by `redispatch_admissible` (R-13, P3.6c):
    // a non-PURE Mote with no `EffectStaged` hint may have fired its effect before crashing,
    // so re-dispatch would risk a double effect — it is left stuck (operator-recoverable),
    // never re-leased. The ready-set half is NOT gated (those are first dispatches; a
    // StageThenCommit Mote that staged-then-crashed is `Pending`/in the ready-set with the
    // hint present, so it is still offered there).
    for mote_id in projection
        .ready_set()
        .into_iter()
        .chain(
            rescheduleable
                .iter()
                .copied()
                .filter(|id| redispatch_admissible(submitted_defs, projection, id)),
        )
        // M2.3b (D65): an at-most-once effect that has already staged is NEVER
        // re-offered (no closing mechanism → a re-dispatch would double-fire).
        // Applied to the COMBINED stream because a staged-then-crashed
        // StageThenCommit Mote is `Pending` (in the ungated ready set).
        .filter(|id| !at_least_once_already_staged(submitted_defs, projection, id))
    {
        if !seen.insert(mote_id) {
            continue;
        }
        if projection.state_of(&mote_id) == MoteState::Committed {
            continue;
        }
        if let Some((_mote, warrant)) = submitted_defs.get(&mote_id) {
            if warrant.executor_class == executor_class {
                if placement.place(&mote_id) == worker {
                    preferred.push(mote_id);
                } else {
                    rest.push(mote_id);
                }
            }
        }
    }
    preferred
        .into_iter()
        .chain(rest)
        .take(max)
        .filter_map(|id| submitted_defs.get(&id).cloned())
        .collect()
}

/// R-13 under distribution (P3.6c): whether a **crash-failed** Mote is safe to *re-dispatch*.
///
/// PURE is always recomputable. A non-PURE (WORLD-MUTATING / READ-ONLY-NONDET) Mote is only
/// re-dispatchable when the recovery oracle says so — i.e. a durable `EffectStaged` hint was
/// recorded before the effect fired (`Projection::can_redispatch_world_effect`), so the broker's
/// tool-boundary idempotency dedupes the re-fire. Without the hint the effect *may already have
/// fired* (D58 lets `ValidateThenCommit` / `IdempotentByConstruction` dispatch without staging),
/// and re-dispatch would risk a double world-effect — which is unrecoverable
/// (`validate-then-commit.md`). So the coordinator refuses to re-lease it; the Mote is left
/// stuck and operator-recoverable (repudiation), mirroring single-node `pick_next`
/// (`kx-runtime`) + the executor R-13 gate (`kx-executor::redispatch_wm_mote`), whose checks are
/// single-node only. Unknown defs are never re-offered.
fn redispatch_admissible(
    submitted_defs: &BTreeMap<MoteId, (Mote, WarrantSpec)>,
    projection: &Projection,
    id: &MoteId,
) -> bool {
    match submitted_defs.get(id) {
        Some((mote, _)) => {
            mote.nd_class() == NdClass::Pure || projection.can_redispatch_world_effect(id)
        }
        None => false,
    }
}

/// **M2.3b (D65 / D105.4) — the distributed class-aware quarantine.** `true` iff
/// re-dispatching `id` would re-fire an **at-most-once** (`IdempotencyClass::AtLeastOnce`)
/// effect that has **already staged** — there is no closing mechanism, so a re-lease
/// double-fires. Such a Mote is excluded from BOTH lease candidate halves (the ready
/// set AND the crash-failed rescheduleable set): a staged-then-crashed
/// `StageThenCommit` Mote is `Pending` (in the ungated ready set), so gating only
/// the rescheduleable half would miss it. The Mote is left stuck + operator-recoverable
/// (the distributed analogue of the single-node executor's quarantine arm).
///
/// A **fresh** at-most-once Mote (no `EffectStaged` → `can_redispatch_world_effect`
/// is `false`) is NOT excluded — its first dispatch proceeds normally. The class is
/// read from the durable folded `RunVersionsResolved` metadata; a tool with no
/// resolved record (e.g. a run that journaled no resolution) does not match, so the
/// behavior is unchanged where the class is not durably known (no regression).
fn at_least_once_already_staged(
    submitted_defs: &BTreeMap<MoteId, (Mote, WarrantSpec)>,
    projection: &Projection,
    id: &MoteId,
) -> bool {
    match submitted_defs.get(id) {
        Some((mote, _)) => {
            mote_dispatches_at_least_once(mote, projection)
                && projection.can_redispatch_world_effect(id)
        }
        None => false,
    }
}

/// `true` iff any tool in the Mote's `tool_contract` durably resolved to
/// [`IdempotencyClass::AtLeastOnce`] (M2.3b). Reads the off-DAG resolved-version
/// metadata folded from `RunVersionsResolved`; a tool with no resolved record
/// does not count.
fn mote_dispatches_at_least_once(mote: &Mote, projection: &Projection) -> bool {
    mote.def.tool_contract.keys().any(|tool| {
        projection.idempotency_class_for_tool(&tool.0) == Some(IdempotencyClassTag::AtLeastOnce)
    })
}

/// Reap dead workers (D57 §3). For each registered worker the registry now reports
/// [`WorkerStatus::Dead`] that still holds outstanding leases, write a
/// `Failed{WorkerCrashed}` for each of its unresolved Motes (the mandated death fact —
/// D21 §11, *no off-journal facts*), fold it, and — if the Mote is still under its retry
/// budget — re-add it to the rescheduleable set so the next poller picks it up. A Mote
/// that committed before the reap (raced ahead) is simply resolved, never crash-failed.
///
/// Runs at the head of every `LeaseWork`, so reschedule is driven by the same poll that
/// will service it — no background reaper, no owner-thread timer (matching P3.1's derived
/// liveness). Writes go through the owner thread, preserving the D40 sole-writer invariant.
fn reap_dead_workers<J: Journal>(
    journal: &J,
    projection: &mut Projection,
    folded_through: &mut u64,
    registry: &dyn WorkerRegistry,
    tracker: &mut LeaseTracker,
) {
    for worker in tracker.leasing_workers() {
        if registry.status(worker) != Some(WorkerStatus::Dead) {
            continue;
        }
        for mote_id in tracker.take_leases(worker) {
            if projection.state_of(&mote_id) == MoteState::Committed {
                tracker.resolve_committed(mote_id);
                continue;
            }
            match journal.append(failed_worker_crashed_entry(mote_id)) {
                Ok(durable) => {
                    let seq = durable.seq();
                    if seq > *folded_through && projection.fold(&durable).is_ok() {
                        *folded_through = seq;
                    }
                    tracker.record_crash(mote_id, PURE_RETRY_BUDGET);
                }
                Err(error) => {
                    tracing::error!(%error, ?mote_id, "failed to record worker-crash death");
                }
            }
        }
    }
}

/// Repudiate `target` and cascade to its committed downstream consumers (D22 / P0.7).
/// Computes the `Repudiated` batch against the live projection, appends it atomically
/// through the sole writer (group commit), and folds the new entries — so the very next
/// `lease_ready` / `ready_set` excludes the repudiated set (the distributed cascade is
/// observed by workers through the coordinator's lease gate; no off-journal facts).
/// Re-repudiating an already-repudiated set dedupes (D15) and folds nothing new.
fn repudiate_cascade<J: Journal>(
    journal: &J,
    projection: &mut Projection,
    folded_through: &mut u64,
    target: MoteId,
    reason: RepudiationReason,
    repudiator_id: u128,
) -> Result<RepudiationOutcome, RepudiationError> {
    let entries = cascade_repudiation_entries(
        &*projection,
        target,
        reason,
        repudiator_id,
        DEFAULT_CASCADE_CEILING,
    )?;
    // entry 0 is the target; the remainder is the downstream cascade.
    let cascade_size = entries.len().saturating_sub(1);
    let durable = journal
        .append_batch(entries)
        .map_err(|e| RepudiationError::Append(e.to_string()))?;
    for entry in &durable {
        let seq = entry.seq();
        if seq > *folded_through {
            projection
                .fold(entry)
                .map_err(|e| RepudiationError::Append(e.to_string()))?;
            *folded_through = seq;
        }
    }
    Ok(RepudiationOutcome {
        target,
        cascade_size,
    })
}

/// Build the `Failed{WorkerCrashed}` entry for a Mote whose leasing worker died. The
/// `idempotency_key` is the Mote's identity (`idempotency.md`: key == derived `MoteId`).
fn failed_worker_crashed_entry(mote_id: MoteId) -> JournalEntry {
    JournalEntry::Failed {
        mote_id,
        idempotency_key: *mote_id.as_bytes(),
        seq: 0,
        reason_class: FailureReason::WorkerCrashed,
        reporter_id: COORDINATOR_REPORTER_ID,
    }
}

/// Read up to `max` `Committed` entries with seq in `(since_seq, current_seq]`, in
/// seq order, plus the cursor to resume from. The cursor (`next_seq`) advances past
/// everything scanned: it is `current_seq` when the whole range was scanned (the peer
/// is caught up), or the last collected entry's seq when the `max` cap was hit (so the
/// next poll resumes right after it — no entry is skipped or re-scanned). Non-committed
/// entries (e.g. `Proposed`) are skipped but still advance the scan, so a peer that only
/// wants committed results never re-reads them. The whole journal below `current_seq` is
/// immutable + append-only, so this read is consistent without locking the writer.
fn read_committed_since<J: Journal>(
    journal: &J,
    since_seq: u64,
    max: usize,
) -> Result<(Vec<JournalEntry>, u64), CoordinatorError> {
    let current = journal.current_seq()?;
    if since_seq >= current || max == 0 {
        return Ok((Vec::new(), current));
    }
    let mut entries = Vec::new();
    let mut next_seq = current;
    for entry in journal.read_entries_by_seq((since_seq + 1)..(current + 1))? {
        if matches!(entry, JournalEntry::Committed { .. }) {
            if entries.len() == max {
                // Cap hit: stop one short and resume after the last collected entry.
                next_seq = entries.last().map_or(since_seq, JournalEntry::seq);
                break;
            }
            entries.push(entry);
        }
    }
    Ok((entries, next_seq))
}

/// The coordinator's dispatch bookkeeping, grouped so the lease/commit helpers take one
/// `&mut` rather than a long argument list:
/// - `submitted` — every admitted Mote id (commit admission: only a submitted Mote may
///   commit; recovery seeds it from the already-committed set);
/// - `defs` — admitted-but-not-yet-committed Motes, kept so `LeaseWork` can hand a worker
///   the Mote + warrant to run (kx-scheduler consumes the Mote on submit, exposes no
///   get-by-id accessor, and is frozen by the thesis test — so the coordinator retains
///   its own copy); freed on commit, so the map is bounded by in-flight work;
/// - `tracker` — the D57 reschedule bookkeeping (leases, crash-failed, retry counts).
///
/// Recovery does not repopulate `defs`/`tracker`: committed Motes are never ready, and a
/// leased-but-uncommitted Mote lost to a coordinator restart is re-leased afresh (still
/// `Pending`), the journal dedupe keeping any double-commit first-wins.
struct Dispatch {
    submitted: BTreeSet<MoteId>,
    defs: BTreeMap<MoteId, (Mote, WarrantSpec)>,
    tracker: LeaseTracker,
}

/// The owner-thread loop. Recovers the projection from the journal, then services
/// commands until every sender drops (the channel closes on coordinator shutdown).
fn core_loop<J: Journal>(
    journal: &J,
    store: Option<&LocalFsContentStore>,
    registry: &dyn WorkerRegistry,
    clock: &dyn Clock,
    nonce: &dyn RunNonceSource,
    tool_registry: &dyn ToolRegistry,
    mut inbox: mpsc::Receiver<Command>,
) {
    let Some((mut projection, mut folded_through, submitted)) = recover(journal) else {
        return;
    };
    let mut scheduler = Scheduler::new(LocalPlacement);
    let mut dispatch = Dispatch {
        submitted,
        defs: BTreeMap::new(),
        tracker: LeaseTracker::default(),
    };

    while let Some(first) = inbox.blocking_recv() {
        // Drain everything immediately available (up to MAX_DRAIN) so consecutive
        // ReportCommits coalesce into one journal transaction (group commit).
        let mut drained = vec![first];
        while drained.len() < MAX_DRAIN {
            match inbox.try_recv() {
                Ok(command) => drained.push(command),
                Err(_) => break,
            }
        }

        // Process in arrival order, accumulating a run of consecutive `Commit`s and
        // flushing it (as one group commit) whenever a non-`Commit` command is
        // reached — so `Submit`s and reads keep their exact in-order semantics
        // (a read or submit always observes the commits queued before it).
        let mut pending: Vec<PendingCommit> = Vec::new();
        for command in drained {
            // `Commit`s accumulate into the pending run; every other command must
            // observe the commits queued before it, so flush the run first (one
            // group commit), then handle the command.
            let command = match command {
                Command::Commit { proposal, reply } => {
                    pending.push((*proposal, reply));
                    continue;
                }
                other => other,
            };
            flush_commits(
                journal,
                store,
                &mut projection,
                &mut folded_through,
                &mut dispatch,
                &mut pending,
            );
            handle_command(
                journal,
                &mut projection,
                &mut folded_through,
                registry,
                clock,
                nonce,
                tool_registry,
                &mut dispatch,
                &mut scheduler,
                command,
            );
        }
        flush_commits(
            journal,
            store,
            &mut projection,
            &mut folded_through,
            &mut dispatch,
            &mut pending,
        );
    }
}

/// Service one non-`Commit` command against the owner-thread state (Commits are
/// coalesced into group commits before this is reached, so the `Commit` arm is
/// unreachable). Each arm sends its `oneshot` reply; a dropped receiver is ignored.
///
/// `too_many_lines` is allowed: this is a flat one-arm-per-`Command` dispatch
/// match (each arm a thin delegation to a named helper). The length is the
/// command count, not cognitive complexity; splitting the match into
/// sub-dispatchers would be artificial.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn handle_command<J: Journal>(
    journal: &J,
    projection: &mut Projection,
    folded_through: &mut u64,
    registry: &dyn WorkerRegistry,
    clock: &dyn Clock,
    nonce: &dyn RunNonceSource,
    tool_registry: &dyn ToolRegistry,
    dispatch: &mut Dispatch,
    scheduler: &mut Scheduler<LocalPlacement>,
    command: Command,
) {
    match command {
        Command::Commit { .. } => unreachable!("Commit is handled above"),
        Command::Submit {
            mote,
            warrant,
            accept_at_least_once,
            reply,
        } => {
            let outcome = submit_and_capture(
                journal,
                projection,
                folded_through,
                tool_registry,
                dispatch,
                scheduler,
                *mote,
                *warrant,
                accept_at_least_once,
            );
            let _ = reply.send(outcome);
        }
        Command::StateOf { mote_id, reply } => {
            let _ = reply.send(projection.state_of(&mote_id));
        }
        Command::CommittedCount { reply } => {
            let _ = reply.send(projection.snapshot().committed_count());
        }
        Command::ReadySet { reply } => {
            let _ = reply.send(projection.ready_set());
        }
        Command::LeaseWork {
            worker,
            executor_class,
            max,
            reply,
        } => {
            let leased = serve_lease(
                journal,
                projection,
                folded_through,
                registry,
                dispatch,
                &LeaseReq {
                    worker,
                    executor_class,
                    max,
                },
            );
            let _ = reply.send(leased);
        }
        Command::ReadEntries {
            since_seq,
            max,
            reply,
        } => {
            let _ = reply.send(read_committed_since(journal, since_seq, max));
        }
        Command::Repudiate {
            target,
            reason,
            repudiator_id,
            reply,
        } => {
            let outcome = repudiate_cascade(
                journal,
                projection,
                folded_through,
                target,
                reason,
                repudiator_id,
            );
            let _ = reply.send(outcome);
        }
        Command::ReportEffectStaged {
            mote_id,
            idempotency_key,
            reply,
        } => {
            let seq = stage_effect(
                journal,
                projection,
                folded_through,
                mote_id,
                idempotency_key,
            );
            let _ = reply.send(seq);
        }
        Command::RegisterRun {
            recipe_fingerprint,
            reply,
        } => {
            let result = register_run(
                journal,
                projection,
                folded_through,
                clock,
                nonce,
                recipe_fingerprint,
            );
            let _ = reply.send(result);
        }
        Command::RunRegistration { reply } => {
            let _ = reply.send(projection.run_registration());
        }
        Command::RunResolvedVersions { reply } => {
            let _ = reply.send(projection.run_resolved_versions().to_vec());
        }
    }
}

/// Append a WORLD-MUTATING Mote's `EffectStaged` entry through the sole writer (D58 —
/// the durable staged-intent the worker records before firing) and fold it, returning
/// the assigned seq. Dedupes by key (D15): a re-stage on recovery returns the existing
/// seq and folds nothing new. This is what gives the coordinator's projection the
/// `effect_staged_observed` recovery hint, so a worker death between stage and commit is
/// safely re-dispatchable (the oracle permits it; the tool's idempotency dedupes the effect).
fn stage_effect<J: Journal>(
    journal: &J,
    projection: &mut Projection,
    folded_through: &mut u64,
    mote_id: MoteId,
    idempotency_key: [u8; 32],
) -> Result<u64, CoordinatorError> {
    let entry = JournalEntry::EffectStaged {
        mote_id,
        idempotency_key,
        seq: 0,
    };
    let durable = journal.append(entry)?;
    let seq = durable.seq();
    if seq > *folded_through {
        projection.fold(&durable)?;
        *folded_through = seq;
    }
    Ok(seq)
}

/// Register the run (M1.1, D63/D64): append the seq=1 `RunRegistered` fact —
/// the run's registered, journaled, immutable `instance_id` (a fresh OS-entropy
/// nonce) plus the client's `recipe_fingerprint` (discovery/dedup only) and an
/// audit timestamp — then fold it (O(1), off the Mote-DAG) and return the id.
///
/// **Idempotent:** if the run is already registered (its seq=1 fact folded into
/// the projection), the existing `instance_id` is returned and nothing is written
/// — `instance_id` is read on replay, never recomputed. **Once-per-run, seq=1:**
/// if the journal already has entries but no registration (a run that began
/// without it), registration is refused (`RunAlreadyStarted`) so the fact can
/// never land mid-run.
fn register_run<J: Journal>(
    journal: &J,
    projection: &mut Projection,
    folded_through: &mut u64,
    clock: &dyn Clock,
    nonce: &dyn RunNonceSource,
    recipe_fingerprint: [u8; 32],
) -> Result<[u8; INSTANCE_ID_LEN], CoordinatorError> {
    // Idempotent: a registered run returns its existing identity (read on replay,
    // never recomputed). The fingerprint argument is ignored on re-registration —
    // the journaled fact is immutable.
    if let Some((instance_id, _)) = projection.run_registration() {
        return Ok(instance_id);
    }
    // Registration must be the FIRST journal fact (seq=1). A non-empty journal with
    // no registration means the run already began without it — refuse rather than
    // append a registration fact in the middle of a run.
    if *folded_through != 0 {
        return Err(CoordinatorError::RunAlreadyStarted);
    }
    let instance_id = nonce.fresh_instance_id();
    let entry = JournalEntry::RunRegistered {
        instance_id,
        recipe_fingerprint,
        // Audit-only; never hashed, never on the identity/scheduling path (SN-8).
        ts: clock.now_ms(),
        seq: 0,
    };
    let durable = journal.append(entry)?;
    let seq = durable.seq();
    if seq > *folded_through {
        projection.fold(&durable)?;
        *folded_through = seq;
    }
    Ok(instance_id)
}

/// Submit a Mote and, on a fresh submit of a registered run, capture its
/// resolved versions (M1.2). Extracted from `handle_command`'s Submit arm to
/// keep that function within the line budget.
#[allow(clippy::too_many_arguments)]
fn submit_and_capture<J: Journal>(
    journal: &J,
    projection: &mut Projection,
    folded_through: &mut u64,
    tool_registry: &dyn ToolRegistry,
    dispatch: &mut Dispatch,
    scheduler: &mut Scheduler<LocalPlacement>,
    mote: Mote,
    warrant: WarrantSpec,
    accept_at_least_once: bool,
) -> Result<SubmitOutcome, CoordinatorError> {
    // GATE 1 — registration-before-submit (M1.3, D64/D98). An unregistered run
    // has no journaled identity to anchor capture (M1.2) or the run-scoped
    // idempotency token, so submit is refused BEFORE the scheduler/journal is
    // touched. The seq=1 RunRegistered fact is read on replay, never recomputed.
    let Some((instance_id, _)) = projection.run_registration() else {
        return Err(CoordinatorError::RunNotRegistered);
    };

    // GATE 2 — resolve the warrant's tool grants ONCE, then run the single-Mote
    // submission-refusal predicate (M1.3). A WORLD-MUTATING Mote with
    // unresolvable tools (D66 fail-closed) or an AtLeastOnce-without-accept tool
    // (R-10) — or any sibling-independent unsafe construction (R-1/R-7/R-8/R-14/
    // R-15) — is refused with NOTHING written. A PURE/READ-ONLY-NONDET Mote is
    // never refused on resolution grounds (no double-fire hazard).
    let (resolution, events) = resolve_for_submit(tool_registry, &warrant);
    validate_mote_submission(&mote, accept_at_least_once, &resolution)
        .map_err(CoordinatorError::SubmissionRefused)?;

    // Admit through the hosted scheduler (verbatim — the P2 thesis test).
    let warrant_for_capture = warrant.clone();
    let mut outcome = handle_submit(scheduler, projection, dispatch, mote, warrant);

    // M1.2 (D79): on a FRESH (non-duplicate) submit, surface the run's
    // instance_id and — when the tools resolved cleanly — capture the resolved
    // tool/model/warrant versions as off-DAG run metadata. Registration is now
    // guaranteed (Gate 1), so every fresh submit anchors to a real instance_id.
    // A WM Unresolved submit never reaches here (refused at Gate 2); a PURE/ROND
    // Unresolved submit reaches here and skips capture (the M1.2 behavior).
    if !outcome.duplicate {
        outcome.instance_id = Some(instance_id);
        if let ToolResolution::Resolved(_) = resolution {
            capture_run_versions(
                journal,
                projection,
                folded_through,
                instance_id,
                &warrant_for_capture,
                events,
            );
        }
    }
    Ok(outcome)
}

/// Resolve the warrant's tool grants ONCE per fresh submit (canonical
/// `(tool_id, tool_version)` order) — feeding BOTH the M1.3 refusal predicate
/// and the M1.2 metadata capture from a single pass over
/// [`ToolRegistry::resolve`](kx_tool_registry::ToolRegistry::resolve).
///
/// Returns the per-submit [`ToolResolution`] (the resolved
/// [`IdempotencyClass`]es, or `Unresolved` on a miss) plus the resolved
/// [`ToolResolutionEvent`]s (for capture). On ANY resolution miss (`NotFound` /
/// `CapabilityExceedsWarrant` / `PendingHumanReview` / `McpUnreachable`) returns
/// `(Unresolved, vec![])`: a WM Mote is then refused fail-closed (D66), a
/// PURE/ROND Mote is admitted with capture skipped.
fn resolve_for_submit(
    tool_registry: &dyn ToolRegistry,
    warrant: &WarrantSpec,
) -> (ToolResolution, Vec<ToolResolutionEvent>) {
    let mut classes: Vec<IdempotencyClass> = Vec::with_capacity(warrant.tool_grants.len());
    let mut events: Vec<ToolResolutionEvent> = Vec::with_capacity(warrant.tool_grants.len());
    for grant in &warrant.tool_grants {
        match tool_registry.resolve(grant, warrant) {
            Ok(resolved) => {
                classes.push(resolved.def.idempotency_class);
                events.push(resolved.event);
            }
            Err(_) => return (ToolResolution::Unresolved, Vec::new()),
        }
    }
    (ToolResolution::Resolved(classes), events)
}

/// Register a submitted Mote through the hosted scheduler (verbatim, thesis test) and
/// retain its def for `LeaseWork`. Returns the canonical id + whether it was an
/// idempotent re-submit (already admitted before commit).
fn handle_submit(
    scheduler: &mut Scheduler<LocalPlacement>,
    projection: &mut Projection,
    dispatch: &mut Dispatch,
    mote: Mote,
    warrant: WarrantSpec,
) -> SubmitOutcome {
    let mote_id = mote.id;
    let duplicate = match scheduler.submit(mote.clone(), warrant.clone(), projection) {
        Ok(()) => {
            dispatch.submitted.insert(mote_id);
            dispatch.defs.insert(mote_id, (mote, warrant));
            false
        }
        Err(SchedulerError::DuplicateSubmission(_)) => true,
    };
    // `instance_id` is filled by the caller (the Submit arm) after this returns —
    // only for a fresh submit of a registered run (M1.2).
    SubmitOutcome {
        mote_id,
        duplicate,
        instance_id: None,
    }
}

/// Capture the resolved tool/model/warrant versions of a fresh submit as off-DAG
/// run **metadata** (M1.2, D79): append one `RunVersionsResolved` fact per
/// resolved capability (a zero-grant warrant gets one fact with no capability),
/// each anchored to the run's `instance_id`. **Metadata, never identity** —
/// never folded into `MoteId`. O(1) per append, off the Mote-DAG.
///
/// `events` are the already-resolved [`ToolResolutionEvent`]s from
/// [`resolve_for_submit`] (resolved once per submit, shared with the M1.3 refusal
/// predicate). This is only called once resolution SUCCEEDED — a resolution miss
/// is handled upstream in [`submit_and_capture`] (a WM Mote is refused, D66; a
/// non-WM Mote is admitted and this capture is skipped), so no partial or
/// over-privileged tuple is ever recorded.
fn capture_run_versions<J: Journal>(
    journal: &J,
    projection: &mut Projection,
    folded_through: &mut u64,
    instance_id: [u8; INSTANCE_ID_LEN],
    warrant: &WarrantSpec,
    events: Vec<ToolResolutionEvent>,
) {
    let warrant_ref = warrant_ref_of(warrant);
    let model_id = warrant.model_route.model_id.0.clone();
    if events.is_empty() {
        // Zero-grant warrant: still capture model + warrant as one metadata fact.
        append_run_versions(
            journal,
            projection,
            folded_through,
            instance_id,
            warrant_ref,
            &model_id,
            None,
        );
        return;
    }
    for event in events {
        let capability = ResolvedCapabilityRecord {
            tool_id: event.tool_id.0,
            tool_version: event.tool_version.0,
            resolved_kind: tool_kind_tag(&event.resolved_kind),
            resolved_def_hash: event.resolved_def_hash,
            // M2.3b (D105.4): persist the resolved class so crash recovery can
            // pick the class-correct action (see `redispatch_admissible`).
            idempotency_class: idempotency_class_tag(event.idempotency_class),
        };
        append_run_versions(
            journal,
            projection,
            folded_through,
            instance_id,
            warrant_ref,
            &model_id,
            Some(capability),
        );
    }
}

/// Append one `RunVersionsResolved` metadata fact through the sole writer and
/// fold it (O(1), off the Mote-DAG). A journal append failure is logged and
/// swallowed — capture is best-effort metadata, never on the truth path.
fn append_run_versions<J: Journal>(
    journal: &J,
    projection: &mut Projection,
    folded_through: &mut u64,
    instance_id: [u8; INSTANCE_ID_LEN],
    warrant_ref: kx_content::ContentRef,
    model_id: &str,
    capability: Option<ResolvedCapabilityRecord>,
) {
    let entry = JournalEntry::RunVersionsResolved {
        instance_id,
        warrant_ref,
        model_id: model_id.to_owned(),
        capability,
        seq: 0,
    };
    match journal.append(entry) {
        Ok(durable) => {
            let seq = durable.seq();
            if seq > *folded_through {
                if let Err(err) = projection.fold(&durable) {
                    tracing::warn!(?err, "fold of RunVersionsResolved metadata failed");
                } else {
                    *folded_through = seq;
                }
            }
        }
        Err(err) => {
            tracing::warn!(?err, "append of RunVersionsResolved metadata failed");
        }
    }
}

/// Map a resolved [`ToolKind`] to its journal [`ResolvedKindTag`] (the closed
/// mirror kept in `kx-journal` so the journal stays dependency-clean).
fn tool_kind_tag(kind: &ToolKind) -> ResolvedKindTag {
    match kind {
        ToolKind::Builtin => ResolvedKindTag::Builtin,
        ToolKind::LocalScript { .. } => ResolvedKindTag::LocalScript,
        ToolKind::External { .. } => ResolvedKindTag::External,
        ToolKind::Mcp { .. } => ResolvedKindTag::Mcp,
        ToolKind::SelfGenerated { .. } => ResolvedKindTag::SelfGenerated,
    }
}

/// Map a resolved [`IdempotencyClass`] to its journal [`IdempotencyClassTag`]
/// (the closed mirror kept in `kx-journal`). M2.3b (D105.4).
fn idempotency_class_tag(class: IdempotencyClass) -> IdempotencyClassTag {
    match class {
        IdempotencyClass::Token => IdempotencyClassTag::Token,
        IdempotencyClass::Readback => IdempotencyClassTag::Readback,
        IdempotencyClass::Staged => IdempotencyClassTag::Staged,
        IdempotencyClass::AtLeastOnce => IdempotencyClassTag::AtLeastOnce,
    }
}

/// One queued `ReportCommit`: its validated proposal + the reply channel.
type PendingCommit = (
    CommitProposal,
    oneshot::Sender<Result<CommitApplied, CoordinatorError>>,
);

/// Build the durable `Committed` entry from a validated proposal.
fn committed_entry(proposal: CommitProposal) -> JournalEntry {
    JournalEntry::Committed {
        mote_id: proposal.mote_id,
        idempotency_key: proposal.idempotency_key,
        seq: 0,
        nondeterminism: proposal.nd_class,
        result_ref: proposal.result_ref,
        parents: proposal.parents,
        warrant_ref: proposal.warrant_ref,
        mote_def_hash: proposal.mote_def_hash,
    }
}

/// Flush a run of queued commits as ONE group commit
/// ([`Journal::append_batch`](kx_journal::Journal::append_batch) — a single journal
/// transaction), then fold the new range once. Never-submitted Motes are rejected
/// individually with no write; the admitted ones are appended atomically. When
/// `store` is `Some`, each proposal's `result_ref` must be present in the content
/// store (D55 phantom-ref guard) — an absent ref is rejected individually, never
/// blocking its batch-mates.
fn flush_commits<J: Journal>(
    journal: &J,
    store: Option<&LocalFsContentStore>,
    projection: &mut Projection,
    folded_through: &mut u64,
    dispatch: &mut Dispatch,
    pending: &mut Vec<PendingCommit>,
) {
    if pending.is_empty() {
        return;
    }
    let batch = std::mem::take(pending);

    // Per-proposal admission: reject never-submitted Motes and (when a store is
    // configured) results whose bytes were never published — neither writes, so one
    // inadmissible commit never blocks its valid batch-mates.
    let mut entries: Vec<JournalEntry> = Vec::with_capacity(batch.len());
    let mut replies: Vec<oneshot::Sender<Result<CommitApplied, CoordinatorError>>> =
        Vec::with_capacity(batch.len());
    let mut committed_ids: Vec<MoteId> = Vec::with_capacity(batch.len());
    for (proposal, reply) in batch {
        if !dispatch.submitted.contains(&proposal.mote_id) {
            let _ = reply.send(Err(CoordinatorError::UnknownMote(proposal.mote_id)));
        } else if store.is_some_and(|s| !s.contains(&proposal.result_ref)) {
            let _ = reply.send(Err(CoordinatorError::ResultRefAbsent(proposal.mote_id)));
        } else {
            committed_ids.push(proposal.mote_id);
            entries.push(committed_entry(proposal));
            replies.push(reply);
        }
    }
    if entries.is_empty() {
        return;
    }

    match apply_batch(journal, projection, folded_through, entries) {
        Ok(applied) => {
            // The defs are no longer leasable once committed (a committed Mote is
            // never in the ready-set); free them to keep the map bounded by
            // in-flight work, not total submissions. Resolving the commit in the
            // reschedule tracker (D57) clears every outstanding lease + crash-failed
            // entry for the Mote — first-wins resolves all concurrent attempts at once.
            for id in &committed_ids {
                dispatch.defs.remove(id);
                dispatch.tracker.resolve_committed(*id);
            }
            for (reply, applied) in replies.into_iter().zip(applied) {
                let _ = reply.send(Ok(applied));
            }
        }
        Err(message) => {
            // The batch is atomic, so on failure nothing was durably written; report
            // the same fault to every waiter (they may retry).
            for reply in replies {
                let _ = reply.send(Err(CoordinatorError::CommitFailed(message.clone())));
            }
        }
    }
}

/// Append a pre-validated, pre-admitted batch in one transaction, fold the newly
/// appended entries **in-hand**, and derive each entry's [`CommitApplied`].
///
/// `append_batch` returns each entry's durable form, so we fold those directly
/// instead of re-reading the new range back from the journal (one fewer query +
/// decode per batch). A commit is **newly committed** iff its returned seq is past
/// the pre-batch watermark AND is the first occurrence of that seq in this batch —
/// a re-report (across batches → older seq; within the batch → an already-seen seq)
/// is `already_committed` and is NOT re-folded (re-folding a `Committed` would trip
/// `DuplicateCommitted`). The newly appended entries arrive in ascending-seq order,
/// so the watermark advances monotonically as we fold. Returns a stringified error
/// (relayable to every waiter) only on a catastrophic journal/projection fault; the
/// batch is atomic, so on error nothing was durably written.
fn apply_batch<J: Journal>(
    journal: &J,
    projection: &mut Projection,
    folded_through: &mut u64,
    entries: Vec<JournalEntry>,
) -> Result<Vec<CommitApplied>, String> {
    // The watermark equals the journal's current seq by invariant (everything
    // <= `folded_through` is folded), so use it directly as the pre-batch boundary
    // — no extra `current_seq()` query.
    let seq_before = *folded_through;
    let durable = journal.append_batch(entries).map_err(|e| e.to_string())?;

    let mut new_seqs = BTreeSet::new();
    let mut applied = Vec::with_capacity(durable.len());
    for entry in &durable {
        let seq = entry.seq();
        let is_new = seq > seq_before && new_seqs.insert(seq);
        if is_new {
            projection.fold(entry).map_err(|e| e.to_string())?;
            *folded_through = seq; // ascending-seq → monotonic advance
        }
        applied.push(CommitApplied {
            committed_seq: seq,
            already_committed: !is_new,
        });
    }
    Ok(applied)
}
