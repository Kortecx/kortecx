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

use kx_content::{ContentRef, ContentStore, LocalFsContentStore};
use kx_journal::{
    FailureReason, IdempotencyClassTag, Journal, JournalEntry, ReactBranch, RepudiationReason,
    ResolvedCapabilityRecord, ResolvedKindTag, INSTANCE_ID_LEN,
};
use kx_mote::{
    ConfigKey, EdgeKind, ModelId, Mote, MoteDef, MoteId, NdClass, PROMPT_KEY, REACT_TURN_KEY,
};
use kx_projection::{
    ContentStoreVerdicts, MoteState, Projection, ReactRoundRecord, RegisterMote, ReplanRoundRecord,
};
use kx_refusal::{validate_mote_submission, ToolResolution};
use kx_scheduler::{LocalPlacement, Placement, Scheduler, SchedulerError, WorkerId};
use kx_tool_registry::{IdempotencyClass, ToolKind, ToolRegistry, ToolResolutionEvent};
use kx_warrant::{
    decode_warrant, encode_warrant, warrant_ref_of, ExecutorClass, RoleRegistry, WarrantSpec,
};
use smallvec::SmallVec;
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
/// `instance_id` (M1.2/D64 — the resume key surfaced on the wire). Registration
/// is enforced before submit (Gate 1), so a returned outcome always carries
/// `Some(instance_id)` for BOTH a fresh and a duplicate submit; the `Option` only
/// reflects the internal pre-fill before `submit_and_capture` resolves the run.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SubmitOutcome {
    pub(crate) mote_id: MoteId,
    pub(crate) duplicate: bool,
    pub(crate) instance_id: Option<[u8; INSTANCE_ID_LEN]>,
}

/// One unit of leased work: the Mote to run, its warrant, and the **F-7
/// parent-context** — the committed `(MoteId, ContentRef)` of the leaf's Data
/// context, resolved on the sole-writer thread from the projection (NO journal
/// write). The worker forwards these to the executor so a model Mote assembles
/// real upstream context (`WorkItem.parent_results`, the reserved seam). A Mote
/// with no Data context (the demo / a root) carries an EMPTY list ⇒ byte-identical
/// to the pre-F-7 leaf path.
#[derive(Debug, Clone)]
pub(crate) struct LeasedItem {
    pub(crate) mote: Mote,
    pub(crate) warrant: WarrantSpec,
    pub(crate) parent_results: Vec<(MoteId, ContentRef)>,
}

/// Leased work plus the run's `instance_id` (if registered) — the `LeaseWork`
/// reply shape (M1.2): the worker derives the run-scoped idempotency token from
/// the `instance_id`.
type LeasedWork = (Vec<LeasedItem>, Option<[u8; INSTANCE_ID_LEN]>);

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
        // PR-2d-1: seed a live ReAct chain — the coordinator swaps in the
        // run-salted turn 0 and anchors a durable ReactRound fact.
        react_seed: bool,
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
    // F4 worker dead-letter: a LIVE worker reports a TERMINAL Mote failure so the
    // coordinator appends a terminal `Failed` (the Mote leaves `ready_set` and is
    // never re-leased) instead of the worker re-leasing it forever.
    ReportFailure {
        mote_id: MoteId,
        idempotency_key: [u8; 32],
        reason_class: FailureReason,
        worker: WorkerId,
        reply: oneshot::Sender<Result<(u64, bool), CoordinatorError>>,
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
        // PR-2b: the role registry the topology materializer narrows child warrants
        // against (SN-8). `Some` enables live shaper-child materialization (the agentic
        // loop in serve); `None` keeps the pre-PR-2b behavior byte-identical (no shaper
        // fan-out — `kx run`, non-inference serve). Held `Send + Sync` (Arc) for the move
        // into the owner thread; materialization itself runs on that one thread.
        shaper_roles: Option<Arc<dyn RoleRegistry>>,
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
                shaper_roles.as_deref(),
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
        react_seed: bool,
    ) -> Result<SubmitOutcome, CoordinatorError> {
        let (reply, response) = oneshot::channel();
        self.dispatch(Command::Submit {
            mote: Box::new(mote),
            warrant: Box::new(warrant),
            accept_at_least_once,
            react_seed,
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

    /// Record a worker-observed TERMINAL Mote failure (F4 dead-letter): append a
    /// terminal `Failed` through the sole writer so the Mote leaves `ready_set` and
    /// is never re-leased. Returns `(seq, appended)` — `appended == false` for an
    /// idempotent no-op (the Mote already committed or already terminal). The worker
    /// calls this after exhausting its attempt budget on a Mote whose execution
    /// cannot succeed (a malformed model proposal, a body that always non-zero-exits),
    /// closing the live-worker spin (PR-9b F2 / the deferred F4 analog).
    pub(crate) async fn report_failure(
        &self,
        mote_id: MoteId,
        idempotency_key: [u8; 32],
        reason_class: FailureReason,
        worker: WorkerId,
    ) -> Result<(u64, bool), CoordinatorError> {
        let (reply, response) = oneshot::channel();
        self.dispatch(Command::ReportFailure {
            mote_id,
            idempotency_key,
            reason_class,
            worker,
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
    store: Option<&LocalFsContentStore>,
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
        store,
    );
    dispatch
        .tracker
        .record_lease(req.worker, items.iter().map(|it| it.mote.id));
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
/// The lease + `ReadySet` ready set with the **P4.2-3 critic exit-gate**
/// auto-activated (PR-2c-3 critic-live). A critic-free run takes the byte-identical
/// `projection.ready_set()` path (zero gate cost). When a critic IS declared, a WM
/// producer's consumers are withheld until its critic commits `Valid`, read by
/// content-address from `store`; **`None` store ⇒ fail-closed** (withhold) so the
/// gate is a pure deterministic fold of the journal, never dependent on a store
/// handle being wired (B2). Used by BOTH `lease_ready` and the `ReadySet` command
/// arm so they can never disagree (H4).
fn gated_ready_set(projection: &Projection, store: Option<&LocalFsContentStore>) -> Vec<MoteId> {
    match store {
        Some(s) => {
            let verdicts = ContentStoreVerdicts::new(s.clone());
            projection.ready_set_auto(Some(&verdicts))
        }
        None => projection.ready_set_auto(None),
    }
}

// One more arg than the clippy default (the optional content store for the
// PR-2c-3 critic exit gate); an internal sole-writer helper, not a public seam.
#[allow(clippy::too_many_arguments)]
fn lease_ready(
    projection: &Projection,
    submitted_defs: &BTreeMap<MoteId, (Mote, WarrantSpec)>,
    registry: &dyn WorkerRegistry,
    rescheduleable: &BTreeSet<MoteId>,
    worker: WorkerId,
    executor_class: ExecutorClass,
    max: usize,
    store: Option<&LocalFsContentStore>,
) -> Vec<LeasedItem> {
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
    for mote_id in gated_ready_set(projection, store)
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
        .filter_map(|id| {
            submitted_defs.get(&id).map(|(mote, warrant)| {
                let parent_results = resolve_parent_context(mote, projection, submitted_defs);
                LeasedItem {
                    mote: mote.clone(),
                    warrant: warrant.clone(),
                    parent_results,
                }
            })
        })
        .collect()
}

/// F-7 (assemble-into-serve): resolve, on the sole-writer thread, the committed
/// `(MoteId, ContentRef)` of `mote`'s **Data context** — the upstream a model Mote
/// must see to reason. This is a pure projection read (NO journal write, NO edge
/// mutation — the canonical digest is untouched):
///
/// - **direct Data parents** of `mote` (the general rule, mirrors
///   `kx_context_assembler::assemble`'s parent loop), and
/// - for a **materialized shaper-child** (whose only parent is a Control edge to
///   its shaper, PR-2b), the shaper's OWN Data parents — so the run's source/prompt
///   context flows down to the leaf *without* giving the child a new edge.
///
/// A Mote with no resolvable Data context (the canonical demo, a root) ⇒ an EMPTY
/// list ⇒ the worker assembles an empty leaf context, byte-identical to pre-F-7.
/// Results are de-duplicated and sorted by `MoteId` so the wire payload (and thus
/// the assembled prompt and the leaf's content-addressed `result_ref`) is
/// deterministic across leases and recovery re-folds (R49).
fn resolve_parent_context(
    mote: &Mote,
    projection: &Projection,
    submitted_defs: &BTreeMap<MoteId, (Mote, WarrantSpec)>,
) -> Vec<(MoteId, ContentRef)> {
    // PR-2c-3 critic-live (B1): a native deterministic critic evaluates its declared
    // check over EXACTLY its producer's committed bytes (`critic_for`) — byte-for-byte
    // the same source the FROZEN `run_native_critic_mote` reads from the journal. So a
    // critic gets ONLY `[(critic_for, ref)]`, never its other Data parents (if any),
    // so the live verdict arm can never be fed a different parent's bytes (which would
    // make a `Valid` verdict promote a producer whose real output was never checked).
    // Empty (producer not yet committed) ⇒ the arm fails closed.
    if let Some(producer_id) = mote.def.critic_for {
        return match projection.result_ref_of(&producer_id) {
            Some(result_ref) => vec![(producer_id, result_ref)],
            None => Vec::new(),
        };
    }
    // PR-2d-1 (the F-7 react special-case): a coordinator-materialized ReAct turn
    // is EDGE-FREE, so its trajectory cannot come from parents — turn T receives
    // the committed `(turn_mote_id, result_ref)` of turns 0..T of ITS run, in
    // TURN-ascending (transcript) order (D78 — deliberately not MoteId order: the
    // model must read the conversation in time order). Guarded by the marker key
    // (cheap) + the folded facts; a react-free run never enters this branch.
    if mote
        .def
        .config_subset
        .contains_key(&ConfigKey(REACT_TURN_KEY.to_string()))
    {
        if let Some(this) = projection
            .react_rounds()
            .iter()
            .find(|r| r.turn_mote_id == mote.id)
        {
            let mut turns: Vec<(u32, MoteId)> = projection
                .react_rounds()
                .iter()
                .filter(|r| r.instance_id == this.instance_id && r.turn < this.turn)
                .map(|r| (r.turn, r.turn_mote_id))
                .collect();
            turns.sort_unstable_by_key(|(t, _)| *t);
            turns.dedup_by_key(|(t, _)| *t);
            return turns
                .into_iter()
                .filter_map(|(_, id)| projection.result_ref_of(&id).map(|r| (id, r)))
                .collect();
        }
        return Vec::new(); // a marker without folded facts: no context (fail-closed)
    }
    let mut out: Vec<(MoteId, ContentRef)> = Vec::new();
    for parent in &mote.parents {
        match parent.edge.kind {
            EdgeKind::Data => {
                if let Some(result_ref) = projection.result_ref_of(&parent.parent_id) {
                    out.push((parent.parent_id, result_ref));
                }
            }
            EdgeKind::Control => {
                // Materialized shaper-child: lift the shaper's Data context one level
                // (never recursive — the shaper's own Control parents are not followed).
                if let Some((shaper, _)) = submitted_defs.get(&parent.parent_id) {
                    for sp in &shaper.parents {
                        if sp.edge.kind == EdgeKind::Data {
                            if let Some(result_ref) = projection.result_ref_of(&sp.parent_id) {
                                out.push((sp.parent_id, result_ref));
                            }
                        }
                    }
                }
            }
        }
    }
    out.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    out.dedup_by(|a, b| a.0 == b.0);
    out
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

/// Reporter id stamped on a worker SELF-reported terminal `Failed` (F4 dead-letter),
/// distinct from the coordinator's reserved `COORDINATOR_REPORTER_ID = 0`. Worker ids
/// start at 0, so the raw id would collide with the coordinator's; the high bit tags
/// the reporter as a worker self-report and the low 64 bits carry the worker id for
/// audit. `reporter_id` is audit metadata only — never folded into a `MoteId`, the
/// projection digest, `ready_set`, or any identity (so this value is free to choose).
fn worker_reporter_id(worker: WorkerId) -> u128 {
    (1u128 << 64) | u128::from(worker.0)
}

/// Append a TERMINAL `Failed` for a Mote a LIVE worker reports it cannot complete
/// (F4 dead-letter), through the sole writer. Mirrors `reap_dead_workers`'s
/// append+fold, but the reporter is the worker (its own terminal verdict — e.g.
/// `DeadLettered` after a budget-exhausted/terminal-logic dispatch failure), not the
/// coordinator observing a death. The Mote becomes terminal `Failed` → excluded from
/// `ready_set` (which yields only `Pending` Motes) → never re-leased, closing the
/// live-worker spin (PR-9b F2 / the deferred F4 analog).
///
/// Fail-safe + idempotent. Returns `(seq, appended)`:
/// - **admission**: the Mote must currently be leased to `worker` (a worker cannot
///   dead-letter work it does not hold) — else `NotLeased`;
/// - a Mote that raced to `Committed` is NEVER dead-lettered (Committed wins): the
///   lease is resolved and the call is a no-op `(0, false)`;
/// - an already-terminal Mote (`Failed`/`Repudiated`/`Inconsistent`) is a no-op
///   `(0, false)` (no second `Failed`);
/// - otherwise a `Failed` is appended + folded, the Mote is dropped from the dispatch
///   admission set + lease tracking, and `(seq, true)` is returned.
// The journal/projection/dispatch trio plus the failure identity (mote/key/reason/worker)
// are each distinct, named, and threaded once through the owner thread — bundling them
// into a struct would be churn for no clarity gain (Rule 1), matching the sibling
// owner-thread handlers (`submit_and_capture`, `repudiate_cascade`).
#[allow(clippy::too_many_arguments)]
fn dead_letter_failure<J: Journal>(
    journal: &J,
    projection: &mut Projection,
    folded_through: &mut u64,
    dispatch: &mut Dispatch,
    mote_id: MoteId,
    idempotency_key: [u8; 32],
    reason_class: FailureReason,
    worker: WorkerId,
) -> Result<(u64, bool), CoordinatorError> {
    // No-op cases first — these write nothing, so they are benign for ANY reporter and
    // must NOT be rejected by the admission gate (the lease is already resolved by the
    // time they occur): a Committed Mote (raced ahead) wins; an already-terminal Mote is
    // an idempotent duplicate report (the first dead-letter resolved the lease). Either
    // way clear any lingering lease tracking and ack a no-op.
    match projection.state_of(&mote_id) {
        MoteState::Pending | MoteState::Scheduled => {}
        _ => {
            dispatch.tracker.resolve_committed(mote_id);
            return Ok((0, false));
        }
    }
    // We are about to write a terminal `Failed`: enforce the admission gate. Only the
    // worker that HOLDS the lease (recorded at `serve_lease`) may dead-letter a live,
    // in-flight Mote — keeping one worker from terminating another's (or a phantom) Mote.
    if !dispatch.tracker.is_held_by(mote_id, worker) {
        return Err(CoordinatorError::NotLeased {
            mote: mote_id,
            worker,
        });
    }
    let entry = JournalEntry::Failed {
        mote_id,
        idempotency_key,
        seq: 0,
        reason_class,
        reporter_id: worker_reporter_id(worker),
    };
    let durable = journal.append(entry)?;
    let seq = durable.seq();
    if seq > *folded_through {
        projection.fold(&durable)?;
        *folded_through = seq;
    }
    // Never re-lease it: drop from the dispatch admission set + lease tracking. The
    // terminal `Failed` already removes it from `ready_set`; this keeps `dispatch.defs`
    // from carrying a dead Mote and resolves the lease (`resolve_committed` clears all
    // tracking for the Mote — the operation is identical for a terminal failure).
    dispatch.submitted.remove(&mote_id);
    dispatch.defs.remove(&mote_id);
    dispatch.tracker.resolve_committed(mote_id);
    Ok((seq, true))
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
#[allow(clippy::too_many_arguments)]
fn core_loop<J: Journal>(
    journal: &J,
    store: Option<&LocalFsContentStore>,
    registry: &dyn WorkerRegistry,
    clock: &dyn Clock,
    nonce: &dyn RunNonceSource,
    tool_registry: &dyn ToolRegistry,
    shaper_roles: Option<&dyn RoleRegistry>,
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
    // PR-2c-2: re-derive the live re-plan chain from committed facts (re-materialize
    // committed round shapers' children, re-insert the in-flight round shaper, and
    // complete any round interrupted by the crash). A no-op when no run has driven a
    // re-plan round (no round-0 anchor) — the canonical demo path is untouched.
    recover_replan_chain(
        journal,
        store,
        shaper_roles,
        &mut projection,
        &mut folded_through,
        &mut dispatch,
    );
    // PR-2d-1: re-derive the live ReAct chains from committed facts (re-insert the
    // in-flight turn, re-decode + settle the committed tail). A no-op when no run
    // has anchored a chain (the has_react_turn sentinel) — the demo is untouched.
    recover_react_chain(
        journal,
        store,
        &mut projection,
        &mut folded_through,
        &mut dispatch,
    );

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
                shaper_roles,
                &mut projection,
                &mut folded_through,
                &mut dispatch,
                &mut pending,
            );
            handle_command(
                journal,
                store,
                shaper_roles,
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
            shaper_roles,
            &mut projection,
            &mut folded_through,
            &mut dispatch,
            &mut pending,
        );
        // PR-2c-2: after this drain's commits + dead-letters fold, drive any re-plan
        // round that just settled (a shaper's children all reached a terminal state
        // with ≥1 failure). Idempotent + O(rounds≤4); a no-op without a round-0 anchor.
        settle_replan_rounds(
            journal,
            store,
            &mut projection,
            &mut folded_through,
            &mut dispatch,
        );
        // PR-2d-1: after this drain's commits + dead-letters fold, settle any ReAct
        // turn that just reached a terminal state (decode → freeze the branch →
        // advance under budget). Idempotent; a one-bool no-op without react facts.
        settle_react_rounds(
            journal,
            store,
            &mut projection,
            &mut folded_through,
            &mut dispatch,
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
    store: Option<&LocalFsContentStore>,
    shaper_roles: Option<&dyn RoleRegistry>,
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
            react_seed,
            reply,
        } => {
            let outcome = submit_and_capture(
                journal,
                store,
                shaper_roles,
                projection,
                folded_through,
                tool_registry,
                dispatch,
                scheduler,
                *mote,
                *warrant,
                accept_at_least_once,
                react_seed,
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
            let _ = reply.send(gated_ready_set(projection, store));
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
                store,
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
        Command::ReportFailure {
            mote_id,
            idempotency_key,
            reason_class,
            worker,
            reply,
        } => {
            let outcome = dead_letter_failure(
                journal,
                projection,
                folded_through,
                dispatch,
                mote_id,
                idempotency_key,
                reason_class,
                worker,
            );
            let _ = reply.send(outcome);
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
    store: Option<&LocalFsContentStore>,
    shaper_roles: Option<&dyn RoleRegistry>,
    projection: &mut Projection,
    folded_through: &mut u64,
    tool_registry: &dyn ToolRegistry,
    dispatch: &mut Dispatch,
    scheduler: &mut Scheduler<LocalPlacement>,
    mote: Mote,
    warrant: WarrantSpec,
    accept_at_least_once: bool,
    react_seed: bool,
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

    // PR-2d-1 — the react SEED-SWAP. The client's seed Mote is validated above
    // (strictly stricter) but never admitted: the coordinator builds the
    // RUN-SALTED turn-0 Mote from the seed's instruction + model route (the salt
    // is the server-assigned `instance_id`, unknowable client-side — SN-8: the
    // admitted identity is server-derived) and substitutes it before the
    // verbatim scheduler path below. LOUD refusals (the flag is explicit intent,
    // unlike replan's silent non-anchor): a promptless seed cannot reason; a
    // storeless coordinator cannot write the durable anchor, so the chain could
    // never crash-recover (the durability law).
    let mote = if react_seed {
        if store.is_none() {
            return Err(CoordinatorError::ReactSeedRefused(
                "this coordinator has no content store; the durable ReactRound \
                 anchor (crash recovery) is impossible",
            ));
        }
        let Some(instruction) = mote
            .def
            .config_subset
            .get(&ConfigKey(PROMPT_KEY.to_string()))
            .and_then(|v| std::str::from_utf8(&v.0).ok().map(str::to_owned))
        else {
            return Err(CoordinatorError::ReactSeedRefused(
                "the seed Mote carries no utf-8 instruction prompt",
            ));
        };
        crate::react_shape::build_react_turn(
            &mote.def.model_id,
            &instruction,
            0,
            &instance_id,
            warrant.model_route.max_output_tokens,
        )
    } else {
        mote
    };

    // PR-2b: capture the shaper identity BEFORE `handle_submit` consumes `mote`/`warrant`,
    // so a re-submitted-but-already-committed shaper (recovery: the in-memory dispatch.defs
    // + materialized children are gone on restart, but the journal still has the committed
    // shaper fact) can re-materialize its children below.
    let turn0_for_anchor = react_seed.then(|| mote.clone());
    let shaper_def = mote.def.is_topology_shaper.then(|| mote.def.clone());
    let shaper_mote_id = mote.id;
    let shaper_warrant = warrant.clone();

    // Admit through the hosted scheduler (verbatim — the P2 thesis test).
    let warrant_for_capture = warrant.clone();
    let mut outcome = handle_submit(scheduler, projection, dispatch, mote, warrant);

    // The run's instance_id is known (Gate 1 guarantees registration) for BOTH a
    // fresh AND a duplicate submit — surface it ALWAYS (the M2 resume key / server-
    // derived identity). A DUPLICATE (idempotent re-submit: the same already-
    // committed Mote, e.g. an Invoke of the same recipe+args) must still resolve to
    // its run, not return an empty instance_id that a client decodes into a 16-byte
    // id. M1.2 (D79) version CAPTURE stays FRESH-only — a duplicate already recorded
    // its run metadata, and re-capture would double-append. (A WM Unresolved submit
    // never reaches here — refused at Gate 2; a PURE/ROND Unresolved submit reaches
    // here and skips capture, the M1.2 behavior.)
    outcome.instance_id = Some(instance_id);
    if !outcome.duplicate {
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
        // PR-2c-2: enable the live re-plan loop for a prompt-carrying topology shaper by
        // writing its run-fixed round-0 anchor (a no-op for non-shapers / prompt-less
        // shapers / runs with no content store). FRESH-submit only — a re-submit re-derives.
        if let (Some(def), Some(store)) = (shaper_def.as_ref(), store) {
            write_replan_anchor(
                journal,
                store,
                projection,
                folded_through,
                shaper_mote_id,
                def,
                &shaper_warrant,
            );
        }
        // PR-2d-1: anchor the live ReAct chain — the durable turn-0 ReactRound fact
        // (base prompt + warrant + budget caps) the settle/recover trio re-derives
        // the chain from. FRESH-submit only; idempotent on the folded facts.
        if let (Some(turn0), Some(store)) = (turn0_for_anchor.as_ref(), store) {
            write_react_anchor(
                journal,
                store,
                projection,
                folded_through,
                instance_id,
                turn0,
                &shaper_warrant,
            )?;
        }
    }

    // PR-2b recovery: if this submit is a shaper that is ALREADY committed (a re-submit
    // after a restart, or an idempotent re-invoke), materialize its children into the
    // projection + dispatch admission set now — they were derived in-memory the first
    // time but lost on restart, while the committed `TopologyDecision` fact survives. The
    // children's identities are re-derived from that fact (R49: served, never re-sampled).
    if let (Some(def), Some(store), Some(roles)) = (shaper_def, store, shaper_roles) {
        if projection.state_of(&shaper_mote_id) == MoteState::Committed {
            materialize_committed_shaper(
                projection,
                dispatch,
                store,
                roles,
                shaper_mote_id,
                &def,
                &shaper_warrant,
            );
        }
    }
    Ok(outcome)
}

/// Materialize a COMMITTED topology shaper's children into BOTH the projection (so they
/// enter `ready_set`) and the dispatch admission set (`Dispatch.defs`, so `lease_ready`
/// can hand them to a worker) — the splice that closes the "materialized children never
/// reach dispatch" gap (§2.149). Derives each child's full `(Mote, WarrantSpec)` from the
/// committed `TopologyDecision` fact via [`crate::materialize::derive_shaper_children`],
/// so the dispatch entry's `MoteId` equals the one a `DefaultTopologyMaterializer` would
/// register (one source of truth). Runs on the live commit-fold (`flush_commits`) AND on a
/// recovery re-submit (`submit_and_capture`); idempotent — `register_mote` overwrites and
/// the dispatch inserts are keyed by id, so re-running is a no-op. A derivation error is
/// logged (the children just do not dispatch — the shaper's commit stands, degraded-safe).
fn materialize_committed_shaper(
    projection: &mut Projection,
    dispatch: &mut Dispatch,
    store: &LocalFsContentStore,
    roles: &dyn RoleRegistry,
    shaper_mote_id: MoteId,
    shaper_def: &MoteDef,
    shaper_warrant: &WarrantSpec,
) {
    let Some(result_ref) = projection.result_ref_of(&shaper_mote_id) else {
        return; // not committed (no result_ref yet) — nothing to materialize
    };
    match crate::materialize::derive_shaper_children(
        store,
        roles,
        shaper_mote_id,
        shaper_def,
        result_ref,
        shaper_warrant,
    ) {
        Ok(children) => {
            for child in children {
                let child_id = child.mote.id;
                projection.register_mote(child.register);
                dispatch.submitted.insert(child_id);
                dispatch.defs.insert(child_id, (child.mote, child.warrant));
            }
        }
        Err(reason) => {
            tracing::error!(
                ?shaper_mote_id,
                %reason,
                "shaper child materialization failed; children will not dispatch"
            );
        }
    }
}

/// The bounded re-plan round budget (PR-2c-2): round 0 (the recipe's own shaper) +
/// up to 3 corrective rounds = 4 TOTAL shaper invocations, mirroring the harness
/// `LoopBudget::max_rounds` default (4) byte-for-byte so the live coordinator and the
/// harness drive identical-length chains (the cross-impl equivalence pin, R49).
const MAX_SHAPER_ROUNDS: u32 = 4;

/// A Mote is **terminal** (no longer in-flight) once it is not `Pending`/`Scheduled`
/// — i.e. it committed, failed (dead-lettered), was repudiated, or went inconsistent.
/// A round settles once every declared child is terminal.
fn is_terminal(state: MoteState) -> bool {
    !matches!(state, MoteState::Pending | MoteState::Scheduled)
}

/// Register + admit a re-plan round's shaper Mote into the projection (so it enters
/// `ready_set`) and the dispatch admission set (`Dispatch.defs`, so `lease_ready` can
/// hand it to a worker). The shaper is EDGE-FREE (empty parents) — immediately ready
/// — exactly like a client-submitted root shaper. Idempotent: `register_mote`
/// overwrites and the id-keyed dispatch inserts are no-ops on a repeat.
fn materialize_replan_shaper(
    projection: &mut Projection,
    dispatch: &mut Dispatch,
    shaper: &Mote,
    warrant_ref: ContentRef,
    warrant: WarrantSpec,
) {
    projection.register_mote(RegisterMote {
        mote_id: shaper.id,
        nd_class: shaper.def.nd_class,
        effect_pattern: shaper.def.effect_pattern,
        critic_for: None,
        is_topology_shaper: true,
        parents: SmallVec::new(),
        warrant_ref,
    });
    dispatch.submitted.insert(shaper.id);
    dispatch.defs.insert(shaper.id, (shaper.clone(), warrant));
}

/// Write the run's round-0 re-plan ANCHOR (PR-2c-2) for a freshly-submitted, prompt-carrying
/// topology shaper: content-store the run-fixed base prompt + warrant and append a durable
/// `ReplanRound{round:0}`. This ENABLES the live re-plan-on-failure loop for the run (the
/// settle pass is inert without an anchor) and makes the chain crash-recoverable from
/// committed facts alone. Idempotent — an existing round-0 anchor for this shaper is a no-op
/// (a re-submit / replay re-derives the same content refs). A shaper carrying no planning
/// prompt is not anchored (re-plan needs the immutable base prompt); the demo + non-shaper
/// submits are untouched (this is gated on `is_topology_shaper` + a present prompt).
fn write_replan_anchor<J: Journal>(
    journal: &J,
    store: &LocalFsContentStore,
    projection: &mut Projection,
    folded_through: &mut u64,
    shaper_id: MoteId,
    shaper_def: &MoteDef,
    warrant: &WarrantSpec,
) {
    if !shaper_def.is_topology_shaper {
        return;
    }
    if projection
        .replan_rounds()
        .iter()
        .any(|r| r.round == 0 && r.shaper_mote_id == shaper_id)
    {
        return; // already anchored (idempotent)
    }
    let Some(prompt) = shaper_def
        .config_subset
        .get(&ConfigKey(PROMPT_KEY.to_string()))
    else {
        return; // no planning prompt ⇒ this shaper is not re-plan-able
    };
    let (Ok(base_ref), Ok(warrant_ref)) =
        (store.put(&prompt.0), store.put(&encode_warrant(warrant)))
    else {
        return; // a store fault leaves the run un-anchored (re-plan simply stays off) — fail-safe
    };
    let entry = JournalEntry::ReplanRound {
        round: 0,
        shaper_mote_id: shaper_id,
        base_prompt_ref: base_ref,
        corrected_prompt_ref: base_ref,
        warrant_ref,
        model_id: shaper_def.model_id.0.clone(),
        failed_steps: SmallVec::new(),
        escalation_reason_ref: None,
        seq: 0,
    };
    match journal.append(entry) {
        Ok(durable) => {
            let seq = durable.seq();
            if seq > *folded_through && projection.fold(&durable).is_ok() {
                *folded_through = seq;
            }
        }
        Err(error) => tracing::error!(%error, "failed to append round-0 ReplanRound anchor"),
    }
}

/// Drive any newly-settled re-plan round (PR-2c-2). **Idempotent + deterministic**:
/// when the latest tracked round's shaper has fully settled with ≥1 failed child,
/// it commits a durable `ReplanRound` fact (the crash-safety anchor) and materializes
/// the next round's shaper. Runs LIVE (after each drain's commits + dead-letters) AND
/// as the recovery catch-up (completing a round interrupted by a crash — the fact +
/// the rebuilt shaper are a pure function of committed facts, so re-driving converges
/// to the same chain). A no-op unless a round-0 anchor exists (the gateway writes it at
/// provision), so non-replan runs + the canonical demo are byte-untouched.
fn settle_replan_rounds<J: Journal>(
    journal: &J,
    store: Option<&LocalFsContentStore>,
    projection: &mut Projection,
    folded_through: &mut u64,
    dispatch: &mut Dispatch,
) {
    let Some(store) = store else {
        return;
    };
    let rounds: Vec<ReplanRoundRecord> = projection.replan_rounds().to_vec();
    // The run-fixed anchor (round 0) carries base_prompt_ref / warrant_ref / model_id.
    let Some(anchor) = rounds.iter().find(|r| r.round == 0) else {
        return; // no anchor ⇒ re-plan not enabled for this run
    };
    // Only the latest round can newly-settle (an earlier round was already settled
    // when it spawned its successor). Budget + dedup off the highest round.
    let Some(latest) = rounds.iter().max_by_key(|r| r.round) else {
        return;
    };
    let round = latest.round;
    let shaper_id = latest.shaper_mote_id;

    // Budget: round N may spawn N+1 only while N+1 < MAX_SHAPER_ROUNDS.
    if round + 1 >= MAX_SHAPER_ROUNDS {
        return;
    }
    // Dedup: a successor round already exists (live double-settle or a recovery re-run).
    if rounds.iter().any(|r| r.round == round + 1) {
        return;
    }
    // The shaper must have committed its TopologyDecision (else no children exist yet).
    if projection.state_of(&shaper_id) != MoteState::Committed {
        return;
    }
    // The round settles only once EVERY declared child is terminal. (A cold re-fold
    // restores failed children's parent edges via the recovery pass BEFORE this runs,
    // so `children_of` is complete here.)
    let children = projection.children_of(&shaper_id);
    if children.is_empty() {
        return; // a shaper with zero children (an inert FlagHuman / empty plan) quiesces
    }
    if !children
        .iter()
        .all(|(c, _)| is_terminal(projection.state_of(c)))
    {
        return; // round still in flight
    }

    // This round's failures, MoteId-byte-sorted (deterministic corrected prompt → a
    // replay-stable next-round shaper identity, R49).
    let mut failures: Vec<(MoteId, Option<FailureReason>)> = children
        .iter()
        .map(|(c, _)| *c)
        .filter(|c| projection.state_of(c) == MoteState::Failed)
        .map(|c| (c, projection.failure_reason_of(&c)))
        .collect();
    failures.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    if failures.is_empty() {
        return; // every step committed — success, no re-plan
    }

    // Build the next round (N+1) from the run-fixed anchor. Any I/O fault fails closed
    // (the round simply doesn't advance this pass; a later pass retries).
    let Ok(base_bytes) = store.get(&anchor.base_prompt_ref) else {
        return;
    };
    let Ok(base_prompt) = std::str::from_utf8(base_bytes.as_ref()) else {
        return;
    };
    let Ok(warrant_bytes) = store.get(&anchor.warrant_ref) else {
        return;
    };
    let Ok(warrant) = decode_warrant(warrant_bytes.as_ref()) else {
        return;
    };
    let next_round = round + 1;
    let corrected = crate::replan_shape::corrected_prompt(base_prompt, &failures);
    let Ok(corrected_ref) = store.put(corrected.as_bytes()) else {
        return;
    };
    let model_id = ModelId(anchor.model_id.clone());
    let shaper = crate::replan_shape::build_replan_shaper(&model_id, &corrected, next_round);
    let failed_steps: SmallVec<[MoteId; 4]> = failures.iter().map(|(id, _)| *id).collect();

    // Commit the durable ReplanRound fact + fold it in-hand (the crash-safety anchor —
    // recovery re-derives this round from it). Append failures BEFORE materializing so
    // a crash after this point resumes from the fact.
    let entry = JournalEntry::ReplanRound {
        round: next_round,
        shaper_mote_id: shaper.id,
        base_prompt_ref: anchor.base_prompt_ref,
        corrected_prompt_ref: corrected_ref,
        warrant_ref: anchor.warrant_ref,
        model_id: anchor.model_id.clone(),
        failed_steps,
        escalation_reason_ref: None,
        seq: 0,
    };
    let durable = match journal.append(entry) {
        Ok(d) => d,
        Err(error) => {
            tracing::error!(%error, round = next_round, "failed to append ReplanRound fact");
            return;
        }
    };
    let seq = durable.seq();
    if seq > *folded_through {
        if let Err(error) = projection.fold(&durable) {
            tracing::error!(%error, "failed to fold ReplanRound fact");
            return;
        }
        *folded_through = seq;
    }
    // Materialize the round-(N+1) shaper (empty parents ⇒ immediately leasable). The
    // worker runs the model → commits a TopologyDecision → `flush_commits` materializes
    // its children, and a later settle pass drives the round after THIS one.
    materialize_replan_shaper(projection, dispatch, &shaper, anchor.warrant_ref, warrant);
    tracing::info!(round = next_round, shaper = ?shaper.id, "re-plan round materialized");
}

/// Recover the live re-plan chain from committed facts after a restart (PR-2c-2).
/// Ordered because a `Failed` fold registers no parent edge, so `children_of(shaper)`
/// is EMPTY for a failed (never-committed) child on a cold re-fold until its edge is
/// restored: **Phase A** re-materializes every committed round≥1 shaper's children
/// (restoring the edges settlement reads); **Phase B** re-inserts the latest, still
/// in-flight round shaper into the dispatch admission set (re-leased afresh);
/// **Phase C** drives any un-acted settled round (a crash after the children failed but
/// before the next `ReplanRound` fact). A no-op unless a round-0 anchor exists.
///
/// (Round 0 = the client's own shaper; its `config_subset` is not journaled, so its
/// children re-materialize via the existing idempotent client re-submit path — exactly
/// as PR-2b recovery. Rounds ≥1 are fully self-contained here.)
fn recover_replan_chain<J: Journal>(
    journal: &J,
    store: Option<&LocalFsContentStore>,
    shaper_roles: Option<&dyn RoleRegistry>,
    projection: &mut Projection,
    folded_through: &mut u64,
    dispatch: &mut Dispatch,
) {
    let (Some(store), Some(roles)) = (store, shaper_roles) else {
        return;
    };
    let rounds: Vec<ReplanRoundRecord> = projection.replan_rounds().to_vec();
    if rounds.is_empty() {
        return;
    }

    // Phase A — re-materialize committed round≥1 shapers' children (restore edges).
    for r in rounds.iter().filter(|r| r.round >= 1) {
        if projection.state_of(&r.shaper_mote_id) != MoteState::Committed {
            continue;
        }
        match crate::materialize::rebuild_replan_shaper(store, r) {
            Ok((shaper, warrant)) => {
                materialize_committed_shaper(
                    projection,
                    dispatch,
                    store,
                    roles,
                    shaper.id,
                    &shaper.def,
                    &warrant,
                );
            }
            Err(reason) => {
                tracing::error!(round = r.round, %reason, "re-plan shaper rebuild failed on recovery");
            }
        }
    }

    // Phase B — the latest round shaper, if still in-flight (not committed), is
    // re-inserted so a worker can re-lease it (it was lost from dispatch.defs on restart).
    if let Some(latest) = rounds
        .iter()
        .filter(|r| r.round >= 1)
        .max_by_key(|r| r.round)
    {
        if projection.state_of(&latest.shaper_mote_id) != MoteState::Committed {
            match crate::materialize::rebuild_replan_shaper(store, latest) {
                Ok((shaper, warrant)) => {
                    materialize_replan_shaper(
                        projection,
                        dispatch,
                        &shaper,
                        latest.warrant_ref,
                        warrant,
                    );
                }
                Err(reason) => {
                    tracing::error!(round = latest.round, %reason, "in-flight re-plan shaper rebuild failed on recovery");
                }
            }
        }
    }

    // Phase C — complete an interrupted settle (idempotent; dedups on the durable fact).
    settle_replan_rounds(journal, Some(store), projection, folded_through, dispatch);
}

/// Write the run's turn-0 ReAct ANCHOR (PR-2d-1): content-store the run-fixed base
/// instruction + warrant and append a durable `ReactRound{turn:0, branch:Pending}`
/// carrying the durable budget caps. This ENABLES the live ReAct chain for the run
/// (the settle pass is inert without folded react facts — the `has_react_turn`
/// sentinel) and makes the chain crash-recoverable from committed facts alone
/// (red-team BLOCKER #2: `Committed` stores only `mote_def_hash`). Idempotent —
/// an existing turn-0 anchor for this `instance_id` is a no-op. LOUD on a store
/// or journal fault (unlike replan's fail-safe non-anchor): the client explicitly
/// asked for a react chain, and an un-anchored chain cannot recover.
#[allow(clippy::too_many_arguments)]
fn write_react_anchor<J: Journal>(
    journal: &J,
    store: &LocalFsContentStore,
    projection: &mut Projection,
    folded_through: &mut u64,
    instance_id: [u8; INSTANCE_ID_LEN],
    turn0: &Mote,
    warrant: &WarrantSpec,
) -> Result<(), CoordinatorError> {
    if projection
        .react_rounds()
        .iter()
        .any(|r| r.instance_id == instance_id && r.turn == 0)
    {
        return Ok(()); // already anchored (idempotent re-submit / replay)
    }
    let Some(instruction) = turn0
        .def
        .config_subset
        .get(&ConfigKey(PROMPT_KEY.to_string()))
    else {
        // Unreachable for a seed-swapped turn (the swap requires the prompt) —
        // kept total + loud rather than panicking on the sole-writer thread.
        return Err(CoordinatorError::ReactSeedRefused(
            "turn 0 carries no instruction prompt",
        ));
    };
    let (Ok(base_ref), Ok(warrant_ref)) = (
        store.put(&instruction.0),
        store.put(&encode_warrant(warrant)),
    ) else {
        return Err(CoordinatorError::ReactSeedRefused(
            "content store fault while writing the react anchor",
        ));
    };
    let entry = JournalEntry::ReactRound {
        turn: 0,
        turn_mote_id: turn0.id,
        instance_id,
        base_prompt_ref: base_ref,
        warrant_ref,
        model_id: turn0.def.model_id.0.clone(),
        branch: ReactBranch::Pending,
        max_turns: crate::react_shape::REACT_MAX_TURNS,
        max_tool_calls: crate::react_shape::REACT_MAX_TOOL_CALLS,
        seq: 0,
    };
    let durable = journal.append(entry)?;
    let seq = durable.seq();
    if seq > *folded_through {
        projection.fold(&durable)?;
        *folded_through = seq;
    }
    Ok(())
}

/// Register + admit a coordinator-materialized ReAct turn into the projection (so
/// it enters `ready_set`) and the dispatch admission set (`Dispatch.defs`, so
/// `lease_ready` can hand it to a worker). The turn is EDGE-FREE (empty parents)
/// — immediately ready; its trajectory context is served out-of-band by the F-7
/// react special-case in [`resolve_parent_context`]. Idempotent: `register_mote`
/// overwrites and the id-keyed dispatch inserts are no-ops on a repeat.
fn materialize_react_turn(
    projection: &mut Projection,
    dispatch: &mut Dispatch,
    turn: &Mote,
    warrant_ref: ContentRef,
    warrant: WarrantSpec,
) {
    projection.register_mote(RegisterMote {
        mote_id: turn.id,
        nd_class: turn.def.nd_class,
        effect_pattern: turn.def.effect_pattern,
        critic_for: None,
        is_topology_shaper: false,
        parents: SmallVec::new(),
        warrant_ref,
    });
    dispatch.submitted.insert(turn.id);
    dispatch.defs.insert(turn.id, (turn.clone(), warrant));
}

/// The distinct `instance_id`s with folded react facts — each is an independent
/// chain in serve's SHARED journal (the run-salt keying, red-team BLOCKER #1).
fn react_instances(rounds: &[ReactRoundRecord]) -> Vec<[u8; INSTANCE_ID_LEN]> {
    let mut out: Vec<[u8; INSTANCE_ID_LEN]> = rounds.iter().map(|r| r.instance_id).collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// Drive every newly-settled ReAct turn (PR-2d-1). **Idempotent + deterministic**:
/// for each run's chain, when the latest turn's Mote has reached a terminal state,
/// decode its committed output ON the sole writer via [`kx_toolcall::parse_tool_call`]
/// (the ONE authority gate — the same crate the gateway fence and the harness call)
/// and append the turn's FROZEN branch fact; a `Tool` branch advances the chain
/// (budget permitting) by appending the next `Pending` fact BEFORE materializing the
/// next turn (crash-safety order — recovery resumes from the fact). Runs LIVE
/// (after each drain's commits + dead-letters) AND as the recovery catch-up
/// (Phase C). The FIRST line is the zero-cost gate: a react-free run (and the
/// canonical demo) folds no `ReactRound` facts, so this returns immediately.
///
/// Budget counters are FOLD-RE-DERIVED from the recorded branches (never an
/// in-memory count, red-team BLOCKER #4), and the gate is a line-for-line mirror
/// of the harness `react.rs` (`>=`, tool-budget-then-turn-budget).
fn settle_react_rounds<J: Journal>(
    journal: &J,
    store: Option<&LocalFsContentStore>,
    projection: &mut Projection,
    folded_through: &mut u64,
    dispatch: &mut Dispatch,
) {
    if !projection.has_react_turn() {
        return; // the zero-cost sentinel: react-free runs pay one bool read
    }
    let Some(store) = store else {
        return; // anchored chains need a store (the anchor write guaranteed one)
    };
    for instance_id in react_instances(projection.react_rounds()) {
        settle_react_chain(
            journal,
            store,
            projection,
            folded_through,
            dispatch,
            instance_id,
        );
    }
}

/// Settle ONE run's chain (see [`settle_react_rounds`]). Bounded: at most one
/// branch fact + one advance per pass per chain (the next pass continues).
#[allow(clippy::too_many_lines)]
fn settle_react_chain<J: Journal>(
    journal: &J,
    store: &LocalFsContentStore,
    projection: &mut Projection,
    folded_through: &mut u64,
    dispatch: &mut Dispatch,
    instance_id: [u8; INSTANCE_ID_LEN],
) {
    let rounds: Vec<ReactRoundRecord> = projection
        .react_rounds()
        .iter()
        .filter(|r| r.instance_id == instance_id)
        .cloned()
        .collect();
    // The run-fixed anchor (turn 0) carries base_prompt_ref / warrant_ref /
    // model_id / the durable budget caps.
    let Some(anchor) = rounds.iter().find(|r| r.turn == 0) else {
        return; // facts without an anchor (defensive) — nothing recoverable
    };
    // The work frontier: the latest fact for the highest turn (a turn's facts are
    // anchor/advance `Pending` then a settled branch; highest seq wins).
    let Some(latest) = rounds
        .iter()
        .max_by(|a, b| a.turn.cmp(&b.turn).then(a.seq.cmp(&b.seq)))
    else {
        return;
    };
    let turn = latest.turn;

    match &latest.branch {
        // Terminal branches: the chain is done (Answer) or dead (DeadLettered).
        ReactBranch::Answer | ReactBranch::DeadLettered => {}
        // A frozen Tool decision whose advance was interrupted (crash between the
        // Tool fact and the next Pending fact) — or just decided this pass:
        // continue the advance under the budget gate.
        ReactBranch::Tool { .. } => {
            advance_react_chain(
                journal,
                store,
                projection,
                folded_through,
                dispatch,
                anchor,
                &rounds,
                turn,
            );
        }
        // The in-flight turn: settle it once its Mote reaches a terminal state.
        ReactBranch::Pending => {
            let turn_state = projection.state_of(&latest.turn_mote_id);
            if !is_terminal(turn_state) {
                return; // still in flight (Pending/Scheduled)
            }
            if turn_state != MoteState::Committed {
                // Dead-lettered / repudiated / inconsistent ⇒ the chain is dead.
                append_react_branch(
                    journal,
                    projection,
                    folded_through,
                    anchor,
                    latest.turn_mote_id,
                    turn,
                    ReactBranch::DeadLettered,
                );
                return;
            }
            // Committed: decode the RAW output via the ONE authority gate.
            let Some(result_ref) = projection.result_ref_of(&latest.turn_mote_id) else {
                return; // defensive: committed without a result_ref
            };
            let Ok(raw) = store.get(&result_ref) else {
                return; // store fault — retry next pass (fail-safe)
            };
            let Ok(warrant_bytes) = store.get(&anchor.warrant_ref) else {
                return;
            };
            let Ok(warrant) = decode_warrant(warrant_bytes.as_ref()) else {
                return;
            };
            let max_args = kx_toolcall::max_args_bytes(&warrant);
            let branch = match kx_toolcall::parse_tool_call(raw.as_ref(), &warrant, max_args) {
                // A normal completion IS the final answer — the committed turn
                // fact is the answer (the harness two-fact contract).
                Ok(None) => ReactBranch::Answer,
                // A warrant-granted tool proposal: freeze the decision. (In
                // PR-2d-1 the live gateway fences tool proposals pre-commit, so
                // this arm is reached only by staged/model-free drivers — the
                // substrate is exercised; firing lands in PR-2d-2.)
                Ok(Some(call)) => ReactBranch::Tool {
                    tool_id: call.name.0,
                    tool_version: call.version.0,
                },
                // Malformed / ungranted / oversize ⇒ the chain dead-letters
                // (fail-closed; the committed turn fact remains, the CHAIN ends).
                Err(_) => ReactBranch::DeadLettered,
            };
            let advanced = matches!(branch, ReactBranch::Tool { .. });
            append_react_branch(
                journal,
                projection,
                folded_through,
                anchor,
                latest.turn_mote_id,
                turn,
                branch,
            );
            if advanced {
                // Re-read the folded rounds (the Tool fact just folded) and
                // continue the advance in the same pass (no wasted drain).
                let rounds: Vec<ReactRoundRecord> = projection
                    .react_rounds()
                    .iter()
                    .filter(|r| r.instance_id == instance_id)
                    .cloned()
                    .collect();
                advance_react_chain(
                    journal,
                    store,
                    projection,
                    folded_through,
                    dispatch,
                    anchor,
                    &rounds,
                    turn,
                );
            }
        }
    }
}

/// Append one FROZEN branch fact for `turn` (idempotent: an identical settled
/// branch already folded for this `(instance_id, turn)` is a no-op).
fn append_react_branch<J: Journal>(
    journal: &J,
    projection: &mut Projection,
    folded_through: &mut u64,
    anchor: &ReactRoundRecord,
    turn_mote_id: MoteId,
    turn: u32,
    branch: ReactBranch,
) {
    if projection
        .react_rounds()
        .iter()
        .any(|r| r.instance_id == anchor.instance_id && r.turn == turn && r.branch == branch)
    {
        return; // already recorded (recovery re-drive)
    }
    let entry = JournalEntry::ReactRound {
        turn,
        turn_mote_id,
        instance_id: anchor.instance_id,
        base_prompt_ref: anchor.base_prompt_ref,
        warrant_ref: anchor.warrant_ref,
        model_id: anchor.model_id.clone(),
        branch,
        max_turns: anchor.max_turns,
        max_tool_calls: anchor.max_tool_calls,
        seq: 0,
    };
    match journal.append(entry) {
        Ok(durable) => {
            let seq = durable.seq();
            if seq > *folded_through && projection.fold(&durable).is_ok() {
                *folded_through = seq;
            }
        }
        Err(error) => tracing::error!(%error, turn, "failed to append ReactRound branch fact"),
    }
}

/// Advance the chain past a frozen `Tool` decision at `turn`: run the budget gate
/// (a line-for-line mirror of the harness `react.rs:336-342` — `>=`,
/// tool-budget-then-turn-budget, counters FOLD-RE-DERIVED from the recorded
/// branches) and, under budget, append the next turn's `Pending` fact BEFORE
/// materializing its Mote (crash-safety order: recovery resumes from the fact).
#[allow(clippy::too_many_arguments)]
fn advance_react_chain<J: Journal>(
    journal: &J,
    store: &LocalFsContentStore,
    projection: &mut Projection,
    folded_through: &mut u64,
    dispatch: &mut Dispatch,
    anchor: &ReactRoundRecord,
    rounds: &[ReactRoundRecord],
    turn: u32,
) {
    // Dedup: the successor turn already exists (live double-settle or recovery).
    if rounds.iter().any(|r| r.turn == turn + 1) {
        return;
    }
    // FOLD-RE-DERIVED counters (BLOCKER #4): tool_calls = the Tool-branch facts
    // recorded so far (this turn's included — it folded before this call);
    // turns_used = turns 0..=turn ran. Then the harness gate, line-for-line.
    let tool_calls = u32::try_from(
        rounds
            .iter()
            .filter(|r| matches!(r.branch, ReactBranch::Tool { .. }))
            .count(),
    )
    .unwrap_or(u32::MAX);
    let turns_used = turn.saturating_add(1);
    if tool_calls >= anchor.max_tool_calls {
        return; // BudgetExhausted (the harness ReactStop semantics)
    }
    if turns_used >= anchor.max_turns {
        return; // BudgetExhausted
    }
    // Build the next turn from the run-fixed anchor. Any I/O fault fails safe
    // (the chain simply doesn't advance this pass; a later pass retries).
    let Ok(base_bytes) = store.get(&anchor.base_prompt_ref) else {
        return;
    };
    let Ok(instruction) = std::str::from_utf8(base_bytes.as_ref()) else {
        return;
    };
    let Ok(warrant_bytes) = store.get(&anchor.warrant_ref) else {
        return;
    };
    let Ok(warrant) = decode_warrant(warrant_bytes.as_ref()) else {
        return;
    };
    let next_turn = turn + 1;
    let model_id = ModelId(anchor.model_id.clone());
    let next = crate::react_shape::build_react_turn(
        &model_id,
        instruction,
        next_turn,
        &anchor.instance_id,
        warrant.model_route.max_output_tokens,
    );
    // Fact BEFORE materialize (crash-safety order, the replan precedent).
    append_react_branch(
        journal,
        projection,
        folded_through,
        anchor,
        next.id,
        next_turn,
        ReactBranch::Pending,
    );
    materialize_react_turn(projection, dispatch, &next, anchor.warrant_ref, warrant);
    tracing::info!(turn = next_turn, mote = ?next.id, "react turn materialized");
}

/// Recover the live ReAct chains from committed facts after a restart (PR-2d-1).
/// **Phase A** is a structural no-op (turns are childless — nothing to
/// re-materialize); **Phase B** re-inserts each chain's in-flight `Pending` turn
/// into the dispatch admission set (it was lost from `dispatch.defs` on restart;
/// rebuilt from the anchor via `react_shape` — identical bytes, R49);
/// **Phase C** re-drives the settle pass, which re-DECODES the committed tail
/// (never trusts a count) and completes any interrupted advance. A no-op without
/// folded react facts (the `has_react_turn` sentinel).
fn recover_react_chain<J: Journal>(
    journal: &J,
    store: Option<&LocalFsContentStore>,
    projection: &mut Projection,
    folded_through: &mut u64,
    dispatch: &mut Dispatch,
) {
    if !projection.has_react_turn() {
        return;
    }
    let Some(store) = store else {
        return;
    };
    // Phase B — re-insert each chain's in-flight turn (its fact is Pending and its
    // Mote is not yet terminal) so a worker can re-lease it.
    for instance_id in react_instances(projection.react_rounds()) {
        let rounds: Vec<ReactRoundRecord> = projection
            .react_rounds()
            .iter()
            .filter(|r| r.instance_id == instance_id)
            .cloned()
            .collect();
        let Some(latest) = rounds
            .iter()
            .max_by(|a, b| a.turn.cmp(&b.turn).then(a.seq.cmp(&b.seq)))
        else {
            continue;
        };
        if !matches!(latest.branch, ReactBranch::Pending) {
            continue; // settled tail — Phase C re-drives it
        }
        if is_terminal(projection.state_of(&latest.turn_mote_id)) {
            continue; // committed/failed — Phase C decodes/settles it
        }
        let Ok(base_bytes) = store.get(&latest.base_prompt_ref) else {
            continue;
        };
        let Ok(instruction) = std::str::from_utf8(base_bytes.as_ref()) else {
            continue;
        };
        let Ok(warrant_bytes) = store.get(&latest.warrant_ref) else {
            continue;
        };
        let Ok(warrant) = decode_warrant(warrant_bytes.as_ref()) else {
            continue;
        };
        let model_id = ModelId(latest.model_id.clone());
        let rebuilt = crate::react_shape::build_react_turn(
            &model_id,
            instruction,
            latest.turn,
            &instance_id,
            warrant.model_route.max_output_tokens,
        );
        if rebuilt.id != latest.turn_mote_id {
            tracing::error!(
                turn = latest.turn,
                expected = ?latest.turn_mote_id,
                rebuilt = ?rebuilt.id,
                "react turn rebuild diverged from the durable fact — not re-inserting (fail-closed)"
            );
            continue;
        }
        materialize_react_turn(projection, dispatch, &rebuilt, latest.warrant_ref, warrant);
    }
    // Phase C — complete any interrupted settle/advance (idempotent; dedups on
    // the durable facts; re-decodes the committed tail).
    settle_react_rounds(journal, Some(store), projection, folded_through, dispatch);
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
#[allow(clippy::too_many_arguments)]
fn flush_commits<J: Journal>(
    journal: &J,
    store: Option<&LocalFsContentStore>,
    shaper_roles: Option<&dyn RoleRegistry>,
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
                // PR-2b: a freshly-committed shaper materializes its children into the
                // projection + dispatch admission set BEFORE its own def is freed below
                // (the shaper's def + warrant are still in `dispatch.defs` here). Clone
                // them out first so the `&mut dispatch` materialize call does not alias the
                // immutable `defs.get` borrow.
                if let (Some(store), Some(roles)) = (store, shaper_roles) {
                    let shaper = dispatch
                        .defs
                        .get(id)
                        .filter(|(m, _)| m.def.is_topology_shaper)
                        .map(|(m, w)| (m.def.clone(), w.clone()));
                    if let Some((def, warrant)) = shaper {
                        materialize_committed_shaper(
                            projection, dispatch, store, roles, *id, &def, &warrant,
                        );
                    }
                }
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
