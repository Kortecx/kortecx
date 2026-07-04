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
    approval_request_id, ApprovalState, FailureReason, IdempotencyClassTag, Journal, JournalEntry,
    ReRankOutcome, ReactBranch, RepudiationReason, ResolvedCapabilityRecord, ResolvedKindTag,
    INSTANCE_ID_LEN,
};
use kx_mote::{
    ConfigKey, EdgeKind, EffectPattern, ModelId, Mote, MoteDef, MoteId, NdClass, ToolName,
    ToolVersion, CONTEXT_ITEMS_KEY, IMAGE_REF_KEY, PROMPT_KEY, REACT_INSTRUCTION_KEY,
    REACT_MAX_TOOL_CALLS_KEY, REACT_MAX_TURNS_KEY, REACT_REQUIRE_APPROVAL_KEY, REACT_TURN_KEY,
    RERANK_CANDIDATES_KEY, RERANK_TURN_KEY, TOOL_ARGS_KEY,
};
use kx_projection::{
    ContentStoreVerdicts, MoteState, Projection, ReRankRoundRecord, ReactRoundRecord, RegisterMote,
    ReplanRoundRecord,
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
    /// PR-2d-2 (react-tools-live): the coordinator-VALIDATED tool args + the
    /// resolved tool's declared egress for a ReAct OBSERVATION — re-derived at
    /// every (re-)lease as a PURE function of committed facts (decode the parent
    /// turn's committed output through the ONE authority gate, validate against
    /// the tool's typed `inputSchema`), so a crash/re-lease carries byte-identical
    /// args with nothing staged. `None` for every non-observation Mote (the
    /// legacy WM/leaf paths are byte-unchanged).
    pub(crate) tool_args: Option<(Vec<u8>, kx_warrant::NetScope, kx_warrant::FsScope)>,
    /// PR-9d (per-turn context-carry): the content-store ref of the run's encoded
    /// context-items bundle, re-derived at every (re-)lease as a PURE function of
    /// committed facts (the chain's turn-0 `ReactRound` anchor `context_items_ref`).
    /// `None` for every Mote that already carries its bundle inline in
    /// `config_subset[CONTEXT_ITEMS_KEY]` (turn 0 / a non-react leaf) or has no
    /// attached context — the legacy path is byte-unchanged. Delivered out-of-band
    /// via `WorkItem.context_items` (edge-free, the F-7 / tool_args precedent).
    pub(crate) context_items: Option<ContentRef>,
    /// AGENTIC-VISION (image-in-the-ReAct-loop): the content-store ref of the run's
    /// grounding image, re-derived at every (re-)lease as a PURE function of committed
    /// facts (the chain's turn-0 `ReactRound` anchor `image_ref`). `None` for every Mote
    /// that already carries its image inline in `config_subset[IMAGE_REF_KEY]` (turn 0)
    /// or has no attached image — the legacy path is byte-unchanged. Delivered out-of-band
    /// via `WorkItem.image_ref` (edge-free, the `context_items` precedent), so a SUCCESSOR
    /// react turn sees the SAME image turn 0 saw.
    pub(crate) image_ref: Option<ContentRef>,
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
    // D114: the operator control plane over pending world-mutating approvals. Grant/
    // Deny are OPERATOR decisions over a SERVER-derived request_id (SN-8) — they
    // release/reject a STAGED action, never mint a client warrant.
    ListPendingApprovals {
        reply: oneshot::Sender<Vec<PendingApprovalView>>,
    },
    GrantApproval {
        request_id: [u8; kx_journal::APPROVAL_REQUEST_ID_LEN],
        approver_id: u64,
        reason: String,
        reply: oneshot::Sender<bool>,
    },
    DenyApproval {
        request_id: [u8; kx_journal::APPROVAL_REQUEST_ID_LEN],
        denier_id: u64,
        reason: String,
        reply: oneshot::Sender<bool>,
    },
    // M11: the run's committed (turns, tool_calls) counts, summed over its react
    // chains — the host prices them into a display-only spend estimate.
    RunCostCounts {
        instance_id: [u8; INSTANCE_ID_LEN],
        reply: oneshot::Sender<(u64, u64)>,
    },
}

/// A pending HITL approval, flattened for the operator inbox (D114). Display-only —
/// it carries NO authority; the grant/deny decision is keyed by the server-derived
/// `request_id`, never by any client-supplied identity (SN-8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingApprovalView {
    /// The server-derived handshake handle (grant/deny key).
    pub request_id: [u8; kx_journal::APPROVAL_REQUEST_ID_LEN],
    /// The run awaiting approval.
    pub instance_id: [u8; INSTANCE_ID_LEN],
    /// The world-mutating observation Mote awaiting approval.
    pub mote_id: MoteId,
    /// The proposed tool's name (display).
    pub tool_id: String,
    /// The proposed tool's pinned version (display).
    pub tool_version: String,
    /// A short prose summary of the proposed action (display).
    pub intent: String,
    /// Approval deadline in unix-ms (`0` ⇒ operator-driven, no auto-expiry).
    pub deadline_unix_ms: u64,
    /// Request creation time in unix-ms (audit wall-clock).
    pub created_unix_ms: u64,
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

    /// D114: the operator's pending-approvals inbox — every world-mutating action
    /// withheld awaiting a decision. Read from the folded handshake facts; off the
    /// truth path (never gates anything itself).
    pub(crate) async fn list_pending_approvals(
        &self,
    ) -> Result<Vec<PendingApprovalView>, CoordinatorError> {
        let (reply, response) = oneshot::channel();
        self.dispatch(Command::ListPendingApprovals { reply })
            .await?;
        response
            .await
            .map_err(|_| CoordinatorError::CoreUnavailable)
    }

    /// D114: GRANT a pending approval (an operator decision over a server-derived
    /// `request_id`, SN-8). Returns `true` iff a decision was recorded — `false` for
    /// an unknown or already-resolved request (idempotent).
    pub(crate) async fn grant_approval(
        &self,
        request_id: [u8; kx_journal::APPROVAL_REQUEST_ID_LEN],
        approver_id: u64,
        reason: String,
    ) -> Result<bool, CoordinatorError> {
        let (reply, response) = oneshot::channel();
        self.dispatch(Command::GrantApproval {
            request_id,
            approver_id,
            reason,
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| CoordinatorError::CoreUnavailable)
    }

    /// D114: DENY a pending approval (the chain dead-letters fail-closed). See
    /// [`Self::grant_approval`].
    pub(crate) async fn deny_approval(
        &self,
        request_id: [u8; kx_journal::APPROVAL_REQUEST_ID_LEN],
        denier_id: u64,
        reason: String,
    ) -> Result<bool, CoordinatorError> {
        let (reply, response) = oneshot::channel();
        self.dispatch(Command::DenyApproval {
            request_id,
            denier_id,
            reason,
            reply,
        })
        .await?;
        response
            .await
            .map_err(|_| CoordinatorError::CoreUnavailable)
    }

    /// M11: the run's committed `(turns, tool_calls)` counts (the host prices them).
    pub(crate) async fn run_cost_counts(
        &self,
        instance_id: [u8; INSTANCE_ID_LEN],
    ) -> Result<(u64, u64), CoordinatorError> {
        let (reply, response) = oneshot::channel();
        self.dispatch(Command::RunCostCounts { instance_id, reply })
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
// One more arg than the clippy default (the PR-2d-2 tool registry for
// observation-args resolution); an internal sole-writer helper, not a public seam.
#[allow(clippy::too_many_arguments)]
fn serve_lease<J: Journal>(
    journal: &J,
    projection: &mut Projection,
    folded_through: &mut u64,
    registry: &dyn WorkerRegistry,
    dispatch: &mut Dispatch,
    req: &LeaseReq,
    store: Option<&LocalFsContentStore>,
    tool_registry: &dyn ToolRegistry,
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
        &dispatch.tracker,
        req.worker,
        req.executor_class,
        req.max,
        store,
        tool_registry,
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

// More args than the clippy default (the optional content store for the
// PR-2c-3 critic exit gate + the PR-2d-2 tool registry for observation-args
// resolution); an internal sole-writer helper, not a public seam.
#[allow(clippy::too_many_arguments)]
fn lease_ready(
    projection: &Projection,
    submitted_defs: &BTreeMap<MoteId, (Mote, WarrantSpec)>,
    registry: &dyn WorkerRegistry,
    rescheduleable: &BTreeSet<MoteId>,
    tracker: &LeaseTracker,
    worker: WorkerId,
    executor_class: ExecutorClass,
    max: usize,
    store: Option<&LocalFsContentStore>,
    tool_registry: &dyn ToolRegistry,
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
        // RC-SW3 pool admission gate: skip a Mote already held by ANOTHER live worker
        // so two pool workers never redundantly run the same one (this is what turns
        // pool>1 into real work-PARTITIONING rather than duplicated leases). A worker
        // may still re-lease its OWN outstanding holds (mid-batch-error self-heal), and
        // a single-worker serve never sees an "other" holder ⇒ byte-identical to pre-
        // RC-SW3. Dead holders were dropped by `reap_dead_workers` above, so a crashed
        // worker's Mote is re-offered here (via `rescheduleable`), never stranded.
        if tracker.is_leased_by_other(mote_id, worker) {
            continue;
        }
        if let Some((mote, warrant)) = submitted_defs.get(&mote_id) {
            // PR-9b-2b: a deterministic-AGENTIC launch step is NEVER dispatched as a
            // plain model mote — the coordinator drives its bounded reason→tool→observe
            // loop on a private chain (`settle_agentic_launches`) and COMMITS the launch
            // mote with the loop's final answer to advance the frozen DAG. PARK it here
            // (skip candidate selection) BEFORE `.take(max)` so a launch — uniquely
            // long-lived in the ready-set (it stays `Pending` for its whole loop) — never
            // consumes a per-poll lease slot from genuinely-dispatchable motes. The
            // shape is disjoint from the observation/authored-tool arms (B1 tests).
            if is_agentic_launch(mote) {
                continue;
            }
            // RC4c-2b: HOLD a grounded chat-rag/vision-rag answer until its durable
            // rerank settles (the suppression gate) — it is edge-free + ready-at-submit,
            // so without this it would dispatch on the base order before the rerank.
            // Un-held (settled or not-eligible) answers fall through unchanged.
            if chat_rag_rerank_holds(mote, warrant, projection, store) {
                continue;
            }
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
            submitted_defs.get(&id).and_then(|(mote, warrant)| {
                let parent_results =
                    resolve_parent_context(mote, projection, submitted_defs, store);
                // PR-2d-2: a ReAct OBSERVATION leases WITH its coordinator-
                // validated args or NOT AT ALL — a transient resolution fault
                // (store I/O) skips the item this poll (fail-safe retry, the
                // settle pass's `Active` mirror); the worker never sees a react
                // observation without args. Every other Mote: `None`, unchanged.
                let tool_args = if is_react_observation(mote, projection) {
                    match resolve_tool_args(mote, warrant, projection, store, tool_registry) {
                        ArgResolution::Resolved(args) => Some(args),
                        // PR-9a: lease selection is READ-ONLY (it must not append a
                        // journal fact). A Transient fault skips this poll (retry). A
                        // Permanent fault ALSO skips here — the settle pass
                        // (`progress_tool_round`) is what appends the terminal
                        // `DeadLettered` branch + retires the wedged observation.
                        ArgResolution::Transient | ArgResolution::Permanent { .. } => {
                            return None;
                        }
                    }
                } else if is_authored_tool(mote, projection) {
                    // PR-6b-2: a STANDALONE authored `tool()` node derives its args
                    // from its OWN identity-bearing `config_subset` (no parent, no
                    // store I/O), validated fail-closed.
                    match resolve_authored_tool_args(mote, tool_registry) {
                        ArgResolution::Resolved(args) => Some(args),
                        // PR-9a: the primary fault (schema mismatch) is refused at
                        // AUTHORING (BUG-27 Path 1); a Permanent here is the rare
                        // deregister-after-authoring residual — skip defensively (a
                        // standalone node has no settle pass; the follow-up ticket
                        // closes it with a coordinator-self dead-letter). Transient
                        // never occurs (args are in the identity-bearing config).
                        ArgResolution::Transient | ArgResolution::Permanent { .. } => {
                            return None;
                        }
                    }
                } else {
                    None
                };
                // PR-9d: re-derive the run's grounding-context ref for a SUCCESSOR
                // ReAct turn from the chain's turn-0 anchor (pure over committed facts,
                // edge-free). `None` for turn 0 / a non-react leaf (which carries its
                // bundle inline) ⇒ the wire payload is byte-identical to pre-PR-9d.
                // RC4c-2b: a settled-Reranked chat-rag/vision-rag answer delivers its
                // RERANKED bundle out-of-band (replacing the inline base order via the
                // dispatch double-prepend fix); every other Mote is unchanged.
                let context_items = chat_rag_delivered_context(mote, warrant, projection, store)
                    .or_else(|| resolve_react_context_items(mote, projection));
                // AGENTIC-VISION: re-derive the run's grounding-image ref for a SUCCESSOR
                // ReAct turn from the chain's turn-0 anchor (pure over committed facts,
                // edge-free). `None` for turn 0 / a non-vision leaf (which carries its
                // image inline) ⇒ the wire payload is byte-identical to pre-AGENTIC-VISION.
                let image_ref = resolve_react_image_ref(mote, projection);
                Some(LeasedItem {
                    mote: mote.clone(),
                    warrant: warrant.clone(),
                    parent_results,
                    tool_args,
                    context_items,
                    image_ref,
                })
            })
        })
        .collect()
}

/// PR-2d-2: `true` iff `mote` is a coordinator-materialized ReAct OBSERVATION —
/// a tool-contract Mote whose single Data parent is a folded react TURN. Scoped
/// deliberately tight (the durable fact log is the witness, never the contract
/// alone) so the args-or-skip lease rule can NEVER wedge an ordinary WM Mote:
/// anything that isn't an observation leases exactly as before.
fn is_react_observation(mote: &Mote, projection: &Projection) -> bool {
    if mote.def.tool_contract.is_empty() || mote.parents.len() != 1 {
        return false;
    }
    // O(log n) off the projection's derived turn-Mote set (PR-2d-2 index).
    projection.is_react_turn_mote(&mote.parents[0].parent_id)
}

/// PR-9a (BUG-27): the outcome of re-deriving a tool-firing Mote's args on the
/// sole-writer thread. The pre-PR-9a resolvers returned `Option`, collapsing
/// EVERY fault to `None` — an infinite fail-safe lease skip even for a fault that
/// can never resolve (a silent wedge: a `tool()` run / ReAct chain stuck "in
/// progress" forever with no error). Splitting into three lets a PERMANENT fault
/// become a LOUD terminal dead-letter while a TRANSIENT fault keeps the fail-safe
/// retry:
///
/// - [`Resolved`](ArgResolution::Resolved) — `(args_bytes, net_scope, fs_scope)`;
///   lease / materialize exactly as before.
/// - [`Transient`](ArgResolution::Transient) — a retryable I/O fault (store
///   absent / store read error / a not-yet-folded parent): skip this poll, a
///   later pass retries.
/// - [`Permanent`](ArgResolution::Permanent) — a DETERMINISTIC fault that can
///   never resolve: a schema reject, an unknown / DEREGISTERED tool, a declared
///   version mismatch, a malformed contract, or a committed turn that no longer
///   decodes to the granted call. The chain / run must dead-letter, never retry.
///   `reason` is a human diagnostic for the trace log (the durable terminal is
///   the existing `DeadLettered` fact — the branch carries no reason field).
enum ArgResolution {
    Resolved((Vec<u8>, kx_warrant::NetScope, kx_warrant::FsScope)),
    Transient,
    Permanent { reason: String },
}

#[cfg(test)]
impl ArgResolution {
    /// The resolved tuple, or `None` for a Transient/Permanent fault (test ergonomics).
    fn into_resolved(self) -> Option<(Vec<u8>, kx_warrant::NetScope, kx_warrant::FsScope)> {
        match self {
            ArgResolution::Resolved(tuple) => Some(tuple),
            _ => None,
        }
    }

    /// Whether the resolution is a permanent (terminal, dead-letterable) fault.
    fn is_permanent(&self) -> bool {
        matches!(self, ArgResolution::Permanent { .. })
    }
}

/// PR-2d-2: re-derive a ReAct observation's `(args_bytes, net_scope)` — a PURE
/// function of committed facts, run on the sole-writer thread at every
/// (re-)lease:
///
/// 1. read the parent TURN's committed bytes (`result_ref_of` + store);
/// 2. decode through the ONE authority gate ([`kx_toolcall::parse_tool_call`] —
///    grant-checked against the chain warrant, size-capped);
/// 3. require the decoded call to name EXACTLY the observation's declared
///    `tool_contract` entry (the frozen `Tool` fact and the contract were both
///    derived from this same decode at settle time — a mismatch means the
///    committed facts changed underneath us, so we refuse);
/// 4. resolve the tool def + validate the args against its typed `inputSchema`
///    FAIL-CLOSED (the harness `dispatch_decoded_call` recipe, D110.4) and take
///    the tool's DECLARED egress requirement as the request's `net_scope` (the
///    broker's `precheck` still enforces request ⊆ warrant at dispatch).
///
/// Returns an [`ArgResolution`] (PR-9a): an I/O fault (missing store / store read
/// error / a not-yet-folded parent) is [`Transient`](ArgResolution::Transient)
/// (skip + retry — the settle validated these args once, so I/O is the only
/// fail-safe-retryable cause); a DETERMINISTIC fault (the committed turn no longer
/// decodes to a granted call, a tool/version/schema disagreement, or the granted
/// tool was DEREGISTERED since the `Tool` branch froze) is
/// [`Permanent`](ArgResolution::Permanent) — the settle pass dead-letters the chain
/// instead of skipping forever (BUG-27).
#[allow(clippy::too_many_lines)] // T-MULTI-ELEMENT-TOOLCALLS: + the call_index recovery branch
fn resolve_tool_args(
    mote: &Mote,
    warrant: &WarrantSpec,
    projection: &Projection,
    store: Option<&LocalFsContentStore>,
    tool_registry: &dyn ToolRegistry,
) -> ArgResolution {
    // Transient I/O: a missing store, a parent not yet folded, or a store read
    // error — a later pass retries (never a terminal decision).
    let Some(store) = store else {
        return ArgResolution::Transient;
    };
    let Some(parent) = mote.parents.first().map(|p| p.parent_id) else {
        return ArgResolution::Transient;
    };
    let Some(result_ref) = projection.result_ref_of(&parent) else {
        return ArgResolution::Transient;
    };
    let Ok(raw) = store.get(&result_ref) else {
        return ArgResolution::Transient;
    };
    // The committed turn + the immutable chain warrant produced this `Tool` branch
    // at freeze. If the SAME bytes no longer decode to a granted call, or the
    // decoded call disagrees with the frozen contract, the facts are inconsistent —
    // a permanent fault, never a transient skip.
    // T-MULTI-ELEMENT-TOOLCALLS: decode ALL the turn's calls (the plural gate). A
    // single-call turn yields one call (byte-identical to PR-2d-2); a ToolBatch turn
    // yields N, and THIS observation fires exactly one of them.
    let mut calls = match kx_toolcall::parse_tool_calls(
        raw.as_ref(),
        warrant,
        kx_toolcall::max_args_bytes(warrant),
    ) {
        Ok(calls) if !calls.is_empty() => calls,
        Ok(_) => {
            return ArgResolution::Permanent {
                reason: "committed turn output no longer decodes to a granted tool call"
                    .to_string(),
            };
        }
        Err(error) => {
            return ArgResolution::Permanent {
                reason: format!("tool-call decode rejected: {error:?}"),
            };
        }
    };
    // Select the call THIS observation fires. A single-call turn uses call[0]
    // (byte-identical). A ToolBatch turn's observation RECOVERS its `call_index` by
    // matching its own MoteId against the candidates rebuilt from the frozen branch's
    // turn coordinates — so two calls to the SAME tool resolve to their OWN args (a
    // PURE function of frozen facts ⇒ recovery re-derives byte-identical args).
    let call = if calls.len() == 1 {
        calls.remove(0)
    } else {
        let Some(record) = projection.react_tool_round_of_turn(&parent) else {
            return ArgResolution::Permanent {
                reason: "no tool-firing react round for the observation's parent turn".to_string(),
            };
        };
        let mut chosen: Option<usize> = None;
        for (i, c) in calls.iter().enumerate() {
            let candidate = crate::react_shape::build_chain_tool(
                &mote.def.model_id,
                &c.name,
                &c.version,
                record.turn,
                &record.instance_id,
                record.step_salt,
                u32::try_from(i).unwrap_or(u32::MAX),
                parent,
            );
            if candidate.id == mote.id {
                chosen = Some(i);
                break;
            }
        }
        let Some(i) = chosen else {
            return ArgResolution::Permanent {
                reason: "observation does not match any call in the frozen batch".to_string(),
            };
        };
        calls.swap_remove(i)
    };
    let Some(declared_version) = mote.def.tool_contract.get(&call.name) else {
        return ArgResolution::Permanent {
            reason: format!(
                "decoded tool {} is not in the observation contract",
                call.name.0
            ),
        };
    };
    if declared_version != &call.version {
        return ArgResolution::Permanent {
            reason: format!(
                "tool {} version mismatch (contract {} vs call {})",
                call.name.0, declared_version.0, call.version.0
            ),
        };
    }
    // Validated + present at branch-freeze; absent now ⇒ DEREGISTERED (or moved to
    // PendingHumanReview) — a permanent registry mutation, the BUG-27 wedge cause.
    let Some(def) = tool_registry.lookup(&call.name, &call.version) else {
        return ArgResolution::Permanent {
            reason: format!(
                "tool {}@{} is no longer registered (deregistered or pending review)",
                call.name.0, call.version.0
            ),
        };
    };
    if let Some(schema) = &def.input_schema {
        if let Err(error) = kx_tool_registry::validate_args(schema, &call.args_bytes) {
            return ArgResolution::Permanent {
                reason: format!(
                    "args do not match {}@{} inputSchema: {error}",
                    call.name.0, call.version.0
                ),
            };
        }
    }
    // PR-6a/D155 (fs-list): the resolved tool's declared fs requirement is taken
    // as the request's fs_scope (empty for echo ⇒ byte-identical). The broker's
    // precheck still enforces request.fs_scope ⊆ warrant.fs_scope at dispatch.
    // PR-3 (A3c): FIRE the normalized bytes (the same form `validate_args`
    // accepted) so a trailing-comma bag that validated also dispatches cleanly to
    // the MCP remote — a PURE function of the frozen turn output, so a recovery
    // re-derive yields byte-identical `WorkItem.tool_args` (args are off-digest).
    ArgResolution::Resolved((
        kx_tool_registry::normalize_lenient_args(&call.args_bytes).into_owned(),
        def.required_capability.net_scope_required.clone(),
        def.required_capability.fs_scope_required.clone(),
    ))
}

/// PR-6b-2: `true` iff `mote` is a STANDALONE authored `tool()` node — a
/// tool-contract Mote carrying its AUTHORED args in `config_subset[TOOL_ARGS_KEY]`,
/// declared `StageThenCommit`, and NOT a coordinator-materialized ReAct
/// observation. The two args-bearing shapes are provably DISJOINT: a react
/// observation has a folded react-turn Data parent and NO `TOOL_ARGS_KEY` (its
/// args are re-derived from the parent turn's output, [`resolve_tool_args`]); an
/// authored tool node carries the key and has no react-turn parent. Scoped as
/// tight as [`is_react_observation`] so the args-from-params lease branch can NEVER
/// wedge an ordinary WM Mote — anything failing this predicate (every canonical
/// run mote: no `tool_contract`, no `TOOL_ARGS_KEY`) leases exactly as before.
fn is_authored_tool(mote: &Mote, projection: &Projection) -> bool {
    !mote.def.tool_contract.is_empty()
        && mote.effect_pattern() == EffectPattern::StageThenCommit
        && mote
            .def
            .config_subset
            .contains_key(&ConfigKey(TOOL_ARGS_KEY.to_string()))
        && !is_react_observation(mote, projection)
}

/// PR-9b-2b: `true` iff `mote` is a deterministic-AGENTIC launch step — a frozen-DAG
/// MODEL step carrying an author-declared tool-grant SET that the coordinator must
/// run as a BOUNDED reason→tool→observe loop (never dispatch as a plain model mote).
/// The discriminant MIRRORS the gateway binder's own Model arm (`provision.rs`): a
/// non-empty `tool_contract` + `PROMPT_KEY` (the model directive) + NO `TOOL_ARGS_KEY`
/// (the loop GENERATES its own tool calls — vs an authored `tool()` node) +
/// ReadOnlyNondet + StageThenCommit (a `generator`). Provably DISJOINT from every
/// other tool-contract shape, so the lease-time park can never mis-fire on an
/// ordinary mote (the `is_react_observation` / `is_authored_tool` tightness contract):
/// - a react OBSERVATION ([`is_react_observation`]) carries an EMPTY `config_subset`
///   (no `PROMPT_KEY`) and a folded react-turn Data parent;
/// - an authored `tool()` node ([`is_authored_tool`]) carries `TOOL_ARGS_KEY` and is
///   WorldMutating (not ReadOnlyNondet);
/// - a react TURN has an EMPTY `tool_contract`.
///
/// PURE (no projection arg): the mote SHAPE alone is decisive, so a launch is parked
/// + the readiness scan keyed on the same shape BEFORE any chain fact exists.
fn is_agentic_launch(mote: &Mote) -> bool {
    !mote.def.tool_contract.is_empty()
        && mote.nd_class() == NdClass::ReadOnlyNondet
        && mote.effect_pattern() == EffectPattern::StageThenCommit
        && mote
            .def
            .config_subset
            .contains_key(&ConfigKey(PROMPT_KEY.to_string()))
        && !mote
            .def
            .config_subset
            .contains_key(&ConfigKey(TOOL_ARGS_KEY.to_string()))
}

/// PR-6b-2: derive a standalone authored `tool()` node's `(args_bytes, net_scope,
/// fs_scope)` — a PURE function of the Mote's OWN identity-bearing `config_subset`
/// (NO parent, NO store I/O), run on the sole-writer thread at every (re-)lease:
///
/// 1. require `tool_contract` to name EXACTLY one `(tool, version)` (an authored
///    tool step binds a single tool);
/// 2. read the authored args object from `config_subset[TOOL_ARGS_KEY]` — one
///    canonical-JSON object lowered byte-identically by the Chains DSL and copied
///    verbatim into `config_subset` by the gateway binder;
/// 3. resolve the tool def (`lookup` returns `None` for absent/PendingHumanReview
///    — fail-closed) and validate the args against its typed `inputSchema`
///    FAIL-CLOSED ([`kx_tool_registry::validate_args`]);
/// 4. take the tool's DECLARED net/fs requirement as the request scopes (the
///    broker's `precheck` still enforces request ⊆ warrant at dispatch).
///
/// Returns an [`ArgResolution`] (PR-9a). Because the args live in the
/// identity-bearing `config_subset` (no store I/O), EVERY fault here is
/// DETERMINISTIC and therefore [`Permanent`](ArgResolution::Permanent) — there is
/// no transient cause. The primary fault (args that do not match the tool's
/// `inputSchema`) is refused at AUTHORING (the gateway's `tool_step_def`) so a
/// wedge can never be authored into existence (BUG-27); a `Permanent` here is the
/// rare DEREGISTER-after-authoring residual (the tool was removed between admit
/// and lease) — the lease arm skips it (a standalone authored node has no settle
/// pass to dead-letter it; closing that residual is a flagged follow-up).
fn resolve_authored_tool_args(mote: &Mote, tool_registry: &dyn ToolRegistry) -> ArgResolution {
    // Exactly one (tool, version): the authored tool step binds a single tool.
    let mut contract = mote.def.tool_contract.iter();
    let Some((name, version)) = contract.next() else {
        return ArgResolution::Permanent {
            reason: "authored tool node has an empty tool_contract".to_string(),
        };
    };
    if contract.next().is_some() {
        return ArgResolution::Permanent {
            reason: "authored tool node binds more than one tool".to_string(),
        };
    }
    let Some(args) = mote
        .def
        .config_subset
        .get(&ConfigKey(TOOL_ARGS_KEY.to_string()))
    else {
        return ArgResolution::Permanent {
            reason: "authored tool node is missing its kx.tool.args config".to_string(),
        };
    };
    let args_bytes = args.0.clone();
    let Some(def) = tool_registry.lookup(name, version) else {
        return ArgResolution::Permanent {
            reason: format!(
                "tool {}@{} is not registered (deregistered or pending review)",
                name.0, version.0
            ),
        };
    };
    if let Some(schema) = &def.input_schema {
        if let Err(error) = kx_tool_registry::validate_args(schema, &args_bytes) {
            return ArgResolution::Permanent {
                reason: format!(
                    "authored args do not match {}@{} inputSchema: {error}",
                    name.0, version.0
                ),
            };
        }
    }
    // PR-3 (A3c): fire the normalized bytes (matching `validate_args`).
    ArgResolution::Resolved((
        kx_tool_registry::normalize_lenient_args(&args_bytes).into_owned(),
        def.required_capability.net_scope_required.clone(),
        def.required_capability.fs_scope_required.clone(),
    ))
}

/// Decode a [`REACT_TURN_KEY`] marker value into its `(instance_id, step_salt)` chain
/// key. 16 bytes = a LEGACY run-level chain (`step_salt = None`); 48 bytes =
/// `instance_id ‖ step_salt` (an agentic step's private chain OR, since PR-R1, a
/// per-invocation run-level chain salted by its seed `MoteId`). Any other length is a
/// malformed marker ⇒ `None` (fail-closed). Decoding BOTH lengths is LOAD-BEARING (a
/// 48-byte marker mis-read as 16 would lose the chain → an empty trajectory).
fn decode_react_marker(bytes: &[u8]) -> Option<([u8; INSTANCE_ID_LEN], Option<[u8; 32]>)> {
    if bytes.len() == INSTANCE_ID_LEN {
        bytes.try_into().ok().map(|i| (i, None))
    } else if bytes.len() == INSTANCE_ID_LEN + 32 {
        let instance_id: [u8; INSTANCE_ID_LEN] = bytes[..INSTANCE_ID_LEN].try_into().ok()?;
        let step_salt: [u8; 32] = bytes[INSTANCE_ID_LEN..].try_into().ok()?;
        Some((instance_id, Some(step_salt)))
    } else {
        None
    }
}

/// PR-9d: the run's grounding-context ref for a SUCCESSOR ReAct turn — the chain's
/// turn-0 `ReactRound` anchor `context_items_ref`, looked up EDGE-FREE from committed
/// facts (the F-7 / `tool_args` precedent — a pure projection read, never a fact
/// append). Returns `None` when the Mote already carries its bundle INLINE in
/// `config_subset[CONTEXT_ITEMS_KEY]` (turn 0 / the seed — `model_exec` prepends that
/// directly, so delivering it again out-of-band would double-prepend), or is not a
/// ReAct turn, or the run has no attached/retrieved context. So a successor turn (no
/// inline bundle) reasons over the SAME grounding context turn 0 saw — fixing the
/// drop where `build_react_turn` omits `CONTEXT_ITEMS_KEY` from a turn's config_subset.
fn resolve_react_context_items(mote: &Mote, projection: &Projection) -> Option<ContentRef> {
    if mote
        .def
        .config_subset
        .contains_key(&ConfigKey(CONTEXT_ITEMS_KEY.to_string()))
    {
        return None; // an inline bundle (turn 0 / leaf) — the config_subset path serves it
    }
    let marker = mote
        .def
        .config_subset
        .get(&ConfigKey(REACT_TURN_KEY.to_string()))?;
    let (instance_id, step_salt) = decode_react_marker(marker.0.as_slice())?;
    projection
        .react_rounds_of(&instance_id, &step_salt)
        .find(|r| r.turn == 0)
        .and_then(|anchor| anchor.context_items_ref)
}

/// AGENTIC-VISION: the run's grounding-image ref for a SUCCESSOR ReAct turn — the
/// chain's turn-0 `ReactRound` anchor `image_ref`, looked up EDGE-FREE from committed
/// facts (the `context_items` precedent — a pure projection read, never a fact append).
/// Returns `None` when the Mote already carries the image INLINE in
/// `config_subset[IMAGE_REF_KEY]` (turn 0 / the seed — `model_exec` reads that directly,
/// so delivering it again out-of-band would double-attach), or is not a ReAct turn, or
/// the run bound no image. So a successor turn (no inline image) reasons over the SAME
/// image turn 0 saw — fixing the drop where `build_react_turn` omits `IMAGE_REF_KEY` from
/// a turn's config_subset (the image must survive EVERY turn for agentic vision).
fn resolve_react_image_ref(mote: &Mote, projection: &Projection) -> Option<ContentRef> {
    if mote
        .def
        .config_subset
        .contains_key(&ConfigKey(IMAGE_REF_KEY.to_string()))
    {
        return None; // an inline image (turn 0) — the config_subset path serves it
    }
    let marker = mote
        .def
        .config_subset
        .get(&ConfigKey(REACT_TURN_KEY.to_string()))?;
    let (instance_id, step_salt) = decode_react_marker(marker.0.as_slice())?;
    projection
        .react_rounds_of(&instance_id, &step_salt)
        .find(|r| r.turn == 0)
        .and_then(|anchor| anchor.image_ref)
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
    store: Option<&LocalFsContentStore>,
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
        // The marker VALUE addresses the CHAIN directly (PR-2d-2 / PR-9b-2b) off
        // the derived index — never a scan across every chain's facts. 16 bytes =
        // `instance_id` (a LEGACY run-level chain from an old journal, `step_salt =
        // None`); 48 bytes = `instance_id ‖ step_salt` (an agentic step's PRIVATE
        // chain OR, since PR-R1, a per-invocation run-level chain salted by its seed
        // MoteId). Decoding BOTH lengths is LOAD-BEARING: a 48-byte marker decoded as
        // a 16-byte `instance_id` (the PR-2d-1 shape) would `try_into`-fail → no
        // chain → an EMPTY trajectory, so every turn past turn 0 would reason with no
        // memory of its own tool observations (a silent loop break).
        let chain: Option<([u8; INSTANCE_ID_LEN], Option<[u8; 32]>)> = mote
            .def
            .config_subset
            .get(&ConfigKey(REACT_TURN_KEY.to_string()))
            .and_then(|v| decode_react_marker(v.0.as_slice()));
        if let Some((instance_id, step_salt)) = chain {
            if let Some(this) = projection
                .react_rounds_of(&instance_id, &step_salt)
                .find(|r| r.turn_mote_id == mote.id)
            {
                // One entry per prior turn: its Mote id + its SETTLED branch (the
                // highest-seq fact — a turn's facts are `Pending` then a frozen
                // branch). PR-2d-2: a `Tool` turn ALSO contributes its OBSERVATION
                // immediately after it — the harness transcript shape
                // `[turn0, obs0, turn1, …]` (`react.rs` pushes turn + obs pairs), so
                // the model reads tool results in time order. The observation's Mote
                // is re-derived from the frozen fact (pure — `build_react_tool`); an
                // uncommitted/absent observation (a PR-2d-1 substrate-only journal,
                // or one still in flight) contributes nothing (fail-safe filter).
                let this_turn = this.turn;
                let mut turns: Vec<&ReactRoundRecord> = Vec::new();
                for r in projection
                    .react_rounds_of(&instance_id, &step_salt)
                    .filter(|r| r.turn < this_turn)
                {
                    match turns.iter_mut().find(|t| t.turn == r.turn) {
                        Some(slot) if r.seq > slot.seq => *slot = r,
                        Some(_) => {}
                        None => turns.push(r),
                    }
                }
                turns.sort_unstable_by_key(|r| r.turn);
                let mut out: Vec<(MoteId, ContentRef)> = Vec::new();
                for r in turns {
                    if let Some(turn_ref) = projection.result_ref_of(&r.turn_mote_id) {
                        out.push((r.turn_mote_id, turn_ref));
                    }
                    // The turn's tool observation(s) interleave right after it: ONE
                    // for a `Tool` branch, N (in call_index order) for a `ToolBatch`
                    // — so the next turn is re-prompted with EVERY tool's result in
                    // the trajectory (T-MULTI-ELEMENT-TOOLCALLS).
                    let batch: Vec<(String, String)> = match &r.branch {
                        ReactBranch::Tool {
                            tool_id,
                            tool_version,
                        } => vec![(tool_id.clone(), tool_version.clone())],
                        ReactBranch::ToolBatch { calls } => calls.clone(),
                        _ => Vec::new(),
                    };
                    for (i, (tool_id, tool_version)) in batch.iter().enumerate() {
                        let obs = crate::react_shape::build_chain_tool(
                            &ModelId(r.model_id.clone()),
                            &ToolName(tool_id.clone()),
                            &ToolVersion(tool_version.clone()),
                            r.turn,
                            &instance_id,
                            step_salt,
                            u32::try_from(i).unwrap_or(u32::MAX),
                            r.turn_mote_id,
                        );
                        if let Some(obs_ref) = projection.result_ref_of(&obs.id) {
                            // RC4c-2b: deliver the RERANKED passage order for a retrieve
                            // observation (when durably reranked); else the base order.
                            // Off-DAG/off-digest — a presentation-only reorder of the
                            // committed observation (the identity/obs.id is unchanged).
                            let delivered =
                                rerank_delivered_ref(projection, store, &instance_id, obs_ref)
                                    .unwrap_or(obs_ref);
                            out.push((obs.id, delivered));
                        }
                    }
                }
                return out;
            }
            return Vec::new(); // a chain marker with no matching turn fact (fail-closed)
        }
        return Vec::new(); // a malformed / absent marker: no context (fail-closed)
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
    /// PR-9b-2b — the admitted-but-PARKED deterministic-agentic launch steps
    /// (`is_agentic_launch`), mapped to their run's `instance_id` (the chain
    /// salt-1 component, captured at submit where it is known — serve's journal
    /// is SHARED across runs, so it cannot be re-derived globally at anchor time).
    /// O(1) populated at submit (`submit_and_capture`) so `settle_agentic_launches`
    /// need not scan all `defs` every drain; an id is dropped once its turn-0
    /// anchor is written (the durable anchor is then the marker). In-memory only —
    /// repopulated on a recovery re-submit (the `defs`/`tracker` precedent); the
    /// skip-if-already-anchored guard prevents a double anchor.
    parked_launches: BTreeMap<MoteId, [u8; INSTANCE_ID_LEN]>,
}

/// The owner-thread loop. Recovers the projection from the journal, then services
/// commands until every sender drops (the channel closes on coordinator shutdown).
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
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
        parked_launches: BTreeMap::new(),
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
    // RC4c-2b: re-insert any in-flight rerank Mote lost on restart (rebuilt from its
    // durable `ReRankRound` anchor), then settle any that committed before the crash.
    // A no-op when no run has anchored a rerank. Runs BEFORE the react recovery so a
    // reranked observation's frozen order is ready when the react chain re-drives.
    recover_rerank_chain(
        journal,
        store,
        &mut projection,
        &mut folded_through,
        &mut dispatch,
    );
    // RC4c-2b: re-anchor + re-stage held chat-rag reranks after a restart (idempotent;
    // a held answer re-inserted into `dispatch.defs` re-drives its rerank). No-op when
    // the flag is off / no eligible answer.
    settle_chat_rag_reranks(
        journal,
        store,
        &mut projection,
        &mut folded_through,
        &mut dispatch,
    );
    let mut react_cache = ReactSettleCache::default();
    recover_react_chain(
        journal,
        store,
        &mut projection,
        &mut folded_through,
        &mut dispatch,
        &mut react_cache,
        tool_registry,
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
        // After this drain's commits + dead-letters fold, run the idempotent settle
        // passes (re-plan rounds · agentic-launch anchoring · react/agentic chain
        // settle). Each is gated to zero cost when its feature is unused.
        run_settle_passes(
            journal,
            store,
            &mut projection,
            &mut folded_through,
            &mut dispatch,
            &mut react_cache,
            tool_registry,
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
                tool_registry,
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
        Command::ListPendingApprovals { reply } => {
            let views = projection
                .pending_approvals()
                .iter()
                .filter_map(|r| match &r.state {
                    ApprovalState::Requested {
                        tool_id,
                        tool_version,
                        intent,
                        deadline_unix_ms,
                        created_unix_ms,
                    } => Some(PendingApprovalView {
                        request_id: r.request_id,
                        instance_id: r.instance_id,
                        mote_id: r.awaiting_mote_id,
                        tool_id: tool_id.clone(),
                        tool_version: tool_version.clone(),
                        intent: intent.clone(),
                        deadline_unix_ms: *deadline_unix_ms,
                        created_unix_ms: *created_unix_ms,
                    }),
                    _ => None,
                })
                .collect();
            let _ = reply.send(views);
        }
        Command::GrantApproval {
            request_id,
            approver_id,
            reason,
            reply,
        } => {
            let decided = decide_approval(
                journal,
                projection,
                folded_through,
                &request_id,
                true,
                approver_id,
                &reason,
            );
            let _ = reply.send(decided);
        }
        Command::DenyApproval {
            request_id,
            denier_id,
            reason,
            reply,
        } => {
            let decided = decide_approval(
                journal,
                projection,
                folded_through,
                &request_id,
                false,
                denier_id,
                &reason,
            );
            let _ = reply.send(decided);
        }
        Command::RunCostCounts { instance_id, reply } => {
            let _ = reply.send(run_cost_counts(projection, &instance_id));
        }
    }
}

/// M11: the run's committed `(turns, tool_calls)`, summed over its react chains. A
/// pure fold over the off-DAG `ReactRound` facts — `turns` per chain = its latest
/// turn + 1; `tool_calls` = the tool-firing branches (the same count the budget gate
/// + the spend gate price). Display-only; never gates anything.
fn run_cost_counts(projection: &Projection, instance_id: &[u8; INSTANCE_ID_LEN]) -> (u64, u64) {
    let mut turns = 0u64;
    let mut tool_calls = 0u64;
    for (inst, salt) in projection.react_chains() {
        if &inst != instance_id {
            continue;
        }
        if let Some(latest) = projection.latest_react_round(&inst, &salt) {
            turns = turns.saturating_add(u64::from(latest.turn).saturating_add(1));
        }
        for r in projection.react_rounds_of(&inst, &salt) {
            tool_calls = tool_calls.saturating_add(match &r.branch {
                ReactBranch::Tool { .. } | ReactBranch::Rejected { .. } => 1,
                ReactBranch::ToolBatch { calls } => calls.len() as u64,
                _ => 0,
            });
        }
    }
    (turns, tool_calls)
}

/// D114: record an operator's GRANT/DENY decision for a pending handshake. Only a
/// STILL-PENDING (`Requested`) request can be decided — a re-grant/deny of a resolved
/// or unknown `request_id` is a no-op (`false`), so the operator action is idempotent.
/// The decision is a durable, server-attributed (SN-8) journal fact the gated react
/// chain reads on its next settle pass (Granted ⇒ the action fires exactly once;
/// Denied ⇒ the chain dead-letters). Returns `true` iff a decision was recorded.
fn decide_approval<J: Journal>(
    journal: &J,
    projection: &mut Projection,
    folded_through: &mut u64,
    request_id: &[u8; kx_journal::APPROVAL_REQUEST_ID_LEN],
    grant: bool,
    operator_id: u64,
    reason: &str,
) -> bool {
    let (instance_id, awaiting_mote_id) = match projection.approval_latest_for(request_id) {
        Some(r) if matches!(r.state, ApprovalState::Requested { .. }) => {
            (r.instance_id, r.awaiting_mote_id)
        }
        _ => return false, // unknown or already decided — idempotent no-op
    };
    let now = approval_now_ms();
    let reason = truncate_for_approval(reason);
    let state = if grant {
        ApprovalState::Granted {
            approver_id: operator_id,
            reason,
            decided_unix_ms: now,
        }
    } else {
        ApprovalState::Denied {
            approver_id: operator_id,
            reason,
            decided_unix_ms: now,
        }
    };
    append_approval(
        journal,
        projection,
        folded_through,
        instance_id,
        *request_id,
        awaiting_mote_id,
        state,
    );
    true
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
    let mut react_caps: Option<(u32, u32)> = None;
    // D114: the run's HITL approval posture (parsed from the seed alongside the caps).
    let mut react_require_approval = false;
    let mut react_chain_salt: Option<[u8; 32]> = None;
    // BUG-35 (+ PR-9d context sibling): the ORIGINAL seed's ANCHOR-BOUND config (the
    // grounding image + grounding-context bundle), captured BEFORE the swap discards the
    // seed (the swapped turn-0 carries only the clean instruction). Both keys are dropped
    // by `build_chain_turn` and re-derived by every successor turn from the anchor, so both
    // MUST be re-injected onto the anchor clone — enumerated in ONE place (`seed_anchor_cfg`)
    // so a future anchor-bound key can never be silently half-carried again.
    let mut react_anchor_cfg: Vec<(ConfigKey, kx_mote::ConfigVal)> = Vec::new();
    let mote = if react_seed {
        if store.is_none() {
            return Err(CoordinatorError::ReactSeedRefused(
                "this coordinator has no content store; the durable ReactRound \
                 anchor (crash recovery) is impossible",
            ));
        }
        // Decode + validate the seed's free params (instruction + budget caps)
        // BEFORE the swap discards the seed — the swapped turn 0 carries only
        // the clean instruction; the caps go to the durable anchor.
        let (instruction, caps, require_approval) = react_seed_params(&mote)?;
        react_caps = Some(caps);
        react_require_approval = require_approval;
        // BUG-35 (+ context sibling): capture the seed's anchor-bound config HERE (`mote`
        // is still the original seed) so the anchor records it — `build_chain_turn` below
        // rebuilds turn-0 from the instruction ONLY, dropping the image AND context bundle.
        react_anchor_cfg = seed_anchor_cfg(&mote);
        // PR-R1 — per-invocation run identity (FINDING-REACT-SHARED-INSTANCE). `kx
        // serve` shares ONE journal / `instance_id` across every Invoke, so a run-
        // level react chain salted by `instance_id` alone collides at turn-0 and the
        // 2nd+ chain DEDUPS to the first. Salt the chain by its SEED MoteId — a
        // content hash of the bound args (the swapped-out seed is NEVER admitted, so
        // its id stays `Pending` in the projection, the run-level/agentic
        // discriminator the settle pass uses): distinct goals ⇒ distinct chains;
        // identical goals ⇒ the SAME chain (Invoke exactly-once preserved). Threaded
        // as the existing per-chain `step_salt`, so every downstream site (index,
        // marker, settle, recovery) is byte-unchanged — the run-level chain simply
        // joins the salt-2 builders an agentic step already uses.
        let chain_salt = *mote.id.as_bytes();
        react_chain_salt = Some(chain_salt);
        crate::react_shape::build_chain_turn(
            &mote.def.model_id,
            &instruction,
            0,
            &instance_id,
            Some(chain_salt),
            warrant.model_route.max_output_tokens,
        )
    } else {
        mote
    };

    // PR-2b: capture the shaper identity BEFORE `handle_submit` consumes `mote`/`warrant`,
    // so a re-submitted-but-already-committed shaper (recovery: the in-memory dispatch.defs
    // + materialized children are gone on restart, but the journal still has the committed
    // shaper fact) can re-materialize its children below.
    // BUG-35 (+ context sibling): carry the original seed's anchor-bound config (image +
    // context bundle) onto the anchor-clone so `write_react_anchor` records both on the
    // turn-0 ReactRound. Every turn (incl. turn 0 — the DISPATCHED swapped mote has neither
    // inline) then re-derives them EDGE-FREE from the anchor via the carried `ContextSink`
    // path. Without this the loop runs BLIND (the model never sees the image / grounding
    // context). A config-only mutation ⇒ the clone's `id` is unchanged (it stays the
    // dispatched turn-0's anchor id).
    let turn0_for_anchor = react_seed.then(|| react_anchor_clone(&mote, &react_anchor_cfg));
    let shaper_def = mote.def.is_topology_shaper.then(|| mote.def.clone());
    let shaper_mote_id = mote.id;
    let shaper_warrant = warrant.clone();
    // PR-9b-2b: a deterministic-AGENTIC launch step is admitted (its def lands in
    // `dispatch.defs` for the eventual terminal launch-commit) but the coordinator
    // PARKS it here, keyed to THIS run's `instance_id` — `settle_agentic_launches`
    // anchors + drives its bounded loop once its DAG parents commit. Captured BEFORE
    // `handle_submit` consumes `mote`. The react seed-swap path is never a launch
    // (the swapped turn-0 carries an empty `tool_contract`).
    let launch_park = (!react_seed && is_agentic_launch(&mote)).then_some(mote.id);

    // Admit through the hosted scheduler (verbatim — the P2 thesis test).
    let warrant_for_capture = warrant.clone();
    let mut outcome = handle_submit(scheduler, projection, dispatch, store, mote, warrant);

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
    // PR-9b-2b: park a freshly-admitted agentic launch keyed to its run (a duplicate
    // re-submit is already parked / anchored — the idempotency guard handles it).
    if let (Some(launch_id), false) = (launch_park, outcome.duplicate) {
        dispatch.parked_launches.insert(launch_id, instance_id);
    }
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
        if let (Some(turn0), Some(store), Some((max_turns, max_tool_calls))) =
            (turn0_for_anchor.as_ref(), store, react_caps)
        {
            write_react_anchor(
                journal,
                store,
                projection,
                folded_through,
                instance_id,
                react_chain_salt, // PR-R1: the per-invocation run-level chain salt (= seed MoteId); was None
                false, // PR-R1: a run-level react chain settles on its own Answer (no launch disposition)
                turn0,
                &shaper_warrant,
                max_turns,
                max_tool_calls,
                react_require_approval,
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
/// Persist an admitted Mote's definition bytes content-addressed (PR-2): the
/// canonical encode's blake3 IS `def.hash()`, so the blob lands at the exact
/// `mote_def_hash` a committed fact carries and `GetMoteDetail` resolves it
/// with no sidecar and no journal write (digest-invariant by construction;
/// rebuildable — a re-submit/recovery re-put is an idempotent no-op).
/// BEST-EFFORT: the blob feeds a display-only read surface, so a store fault
/// must never fail admission (the read side answers `def_found = false`).
fn persist_def(store: Option<&LocalFsContentStore>, def: &MoteDef) {
    let Some(store) = store else {
        return; // storeless coordinator (harness/tests) — nothing to persist
    };
    if let Err(error) = store.put(&def.encode()) {
        tracing::warn!(
            %error,
            def_hash = ?def.hash(),
            "def blob persist failed; GetMoteDetail will answer def_found=false"
        );
    }
}

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
                persist_def(Some(store), &child.mote.def);
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
    store: Option<&LocalFsContentStore>,
    shaper: &Mote,
    warrant_ref: ContentRef,
    warrant: WarrantSpec,
) {
    persist_def(store, &shaper.def);
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
    materialize_replan_shaper(
        projection,
        dispatch,
        Some(store),
        &shaper,
        anchor.warrant_ref,
        warrant,
    );
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
                        Some(store),
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

/// Decode + validate a ReAct SEED's free params (PR-2d-2): the instruction and
/// the per-run budget caps. The instruction comes from `PROMPT_KEY` (the direct
/// submission contract, PR-2d-1) or `REACT_INSTRUCTION_KEY` (the
/// `kx/recipes/react` slot), each tried JSON-string-first then raw UTF-8 — the
/// `prompt_from_config` precedent: a recipe-bound `Str` arrives JSON-quoted, a
/// directly-built seed carries raw bytes. The caps come from
/// `REACT_MAX_TURNS_KEY` / `REACT_MAX_TOOL_CALLS_KEY` (canonical-JSON unsigned
/// ints), defaulting to `8` turns / `6` tool calls, and are validated
/// `0 < max_tool_calls < max_turns ≤ 8` — a violation is a LOUD
/// `ReactSeedRefused` (the flag/recipe is explicit intent; a malformed budget
/// must never silently anchor). Everything is read off the SEED, which is then
/// SWAPPED — none of these keys reach an admitted identity.
/// D114: the serve-wide HITL approval default — `KX_SERVE_REQUIRE_APPROVAL` truthy
/// (`1`/`true`/`yes`/`on`, case-insensitive) ⇒ every NEW react chain gates its
/// irreversible world-mutating tool calls unless the seed explicitly overrides. A
/// host-config read OFF the identity/digest path (the resolved value is frozen on the
/// off-DAG turn-0 anchor; recovery reads the recorded value, never re-reads the env).
/// Default-off ⇒ byte-identical to today.
fn serve_require_approval_default() -> bool {
    std::env::var("KX_SERVE_REQUIRE_APPROVAL")
        .ok()
        .is_some_and(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
}

fn react_seed_params(seed: &Mote) -> Result<(String, (u32, u32), bool), CoordinatorError> {
    let text_param = |key: &str| -> Option<String> {
        let raw = &seed.def.config_subset.get(&ConfigKey(key.to_string()))?.0;
        serde_json::from_slice::<String>(raw)
            .ok()
            .or_else(|| std::str::from_utf8(raw).ok().map(str::to_owned))
    };
    let Some(instruction) = text_param(PROMPT_KEY).or_else(|| text_param(REACT_INSTRUCTION_KEY))
    else {
        return Err(CoordinatorError::ReactSeedRefused(
            "the seed Mote carries no utf-8 instruction prompt",
        ));
    };
    let cap_param = |key: &str, default: u32| -> Result<u32, CoordinatorError> {
        match seed.def.config_subset.get(&ConfigKey(key.to_string())) {
            None => Ok(default),
            Some(v) => serde_json::from_slice::<u32>(&v.0).map_err(|_| {
                CoordinatorError::ReactSeedRefused(
                    "a react budget cap is not a canonical-JSON unsigned integer",
                )
            }),
        }
    };
    let max_turns = cap_param(REACT_MAX_TURNS_KEY, crate::react_shape::REACT_MAX_TURNS)?;
    let max_tool_calls = cap_param(
        REACT_MAX_TOOL_CALLS_KEY,
        crate::react_shape::REACT_DEFAULT_MAX_TOOL_CALLS,
    )?;
    // T-MULTI-ELEMENT-TOOLCALLS: the two caps are now INDEPENDENT. A turn can fire N
    // tools at once (a `ToolBatch`), so the total tool-call budget legitimately
    // exceeds the model-turn budget — the old `max_tool_calls < max_turns` coupling
    // (which assumed ≤1 tool per turn) no longer holds. Each cap is bounded by its own
    // hard ceiling: model turns ≤ REACT_MAX_TURNS (8), total tool calls ≤
    // REACT_MAX_TOOL_CALLS (20). A seed cap above either ceiling is refused LOUDLY.
    if max_turns == 0
        || max_turns > crate::react_shape::REACT_MAX_TURNS
        || max_tool_calls == 0
        || max_tool_calls > crate::react_shape::REACT_MAX_TOOL_CALLS
    {
        return Err(CoordinatorError::ReactSeedRefused(
            "react budget caps must satisfy 0 < max_turns <= 8 AND \
             0 < max_tool_calls <= 20",
        ));
    }
    // D114: the HITL approval posture. An EXPLICIT per-run override (the seed's
    // `REACT_REQUIRE_APPROVAL_KEY`, a canonical-JSON bool — set by an authoring binder)
    // wins; ABSENT ⇒ the serve-wide default `KX_SERVE_REQUIRE_APPROVAL` (off ⇒ `false`,
    // byte-identical to today). Resolving the env default HERE (not by injecting a key
    // into the seed config) keeps the seed MoteId — hence the chain identity — stable;
    // the resolved posture is recorded on the OFF-DAG turn-0 anchor (the digest + the
    // recovery-stable contract are unaffected). A present-but-non-bool override is
    // refused LOUDLY (never silently un-gated).
    let require_approval = match seed
        .def
        .config_subset
        .get(&ConfigKey(REACT_REQUIRE_APPROVAL_KEY.to_string()))
    {
        None => serve_require_approval_default(),
        Some(v) => serde_json::from_slice::<bool>(&v.0).map_err(|_| {
            CoordinatorError::ReactSeedRefused(
                "the react require_approval posture is not a canonical-JSON boolean",
            )
        })?,
    };
    Ok((instruction, (max_turns, max_tool_calls), require_approval))
}

/// The react seed's ANCHOR-BOUND config keys — the grounding image ([`IMAGE_REF_KEY`],
/// AGENTIC-VISION) and the grounding-context bundle ([`CONTEXT_ITEMS_KEY`], PR-9d). BOTH
/// are dropped by the seed-swap (`build_chain_turn` rebuilds turn 0 from the instruction
/// alone) yet are re-derived by every successor turn from the turn-0 anchor — so both MUST
/// ride the anchor clone. Enumerated in ONE place so a future anchor-bound key cannot be
/// silently half-carried again (the BUG-35 class).
const REACT_ANCHOR_BOUND_KEYS: [&str; 2] = [IMAGE_REF_KEY, CONTEXT_ITEMS_KEY];

/// BUG-35 (+ PR-9d context sibling): the ORIGINAL seed's anchor-bound config (image +
/// context bundle), captured from the seed BEFORE the seed-swap discards it (the swapped
/// turn 0 is rebuilt from the instruction alone). Empty for a plain text-only react seed.
fn seed_anchor_cfg(seed: &Mote) -> Vec<(ConfigKey, kx_mote::ConfigVal)> {
    REACT_ANCHOR_BOUND_KEYS
        .iter()
        .filter_map(|k| {
            let key = ConfigKey((*k).to_string());
            seed.def.config_subset.get(&key).map(|v| (key, v.clone()))
        })
        .collect()
}

/// BUG-35 (+ context sibling): build the turn-0 anchor clone for a react seed — the
/// DISPATCHED swapped mote PLUS the ORIGINAL seed's anchor-bound config (image + context
/// bundle) re-injected. The seed-swap rebuilds turn 0 from the instruction alone (dropping
/// both inline keys), so the coordinator captures them BEFORE the swap and re-attaches them
/// here, on the clone `write_react_anchor` records — every successor turn then re-derives
/// them EDGE-FREE from that anchor. A config-only mutation ⇒ the clone's `id` is unchanged
/// (it stays the dispatched turn-0's anchor id). Empty `anchor_cfg` ⇒ a plain clone
/// (byte-identical to the pre-AGENTIC-VISION / pre-PR-9d text path).
fn react_anchor_clone(dispatched: &Mote, anchor_cfg: &[(ConfigKey, kx_mote::ConfigVal)]) -> Mote {
    let mut m = dispatched.clone();
    for (key, val) in anchor_cfg {
        m.def.config_subset.insert(key.clone(), val.clone());
    }
    m
}

/// Write the run's turn-0 ReAct ANCHOR (PR-2d-1): content-store the run-fixed base
/// instruction + warrant and append a durable `ReactRound{turn:0, branch:Pending}`
/// carrying the durable budget caps (validated at the seed-swap, PR-2d-2). This
/// ENABLES the live ReAct chain for the run (the settle pass is inert without
/// folded react facts — the `has_react_turn` sentinel) and makes the chain
/// crash-recoverable from committed facts alone (red-team BLOCKER #2:
/// `Committed` stores only `mote_def_hash`). Idempotent — an existing turn-0
/// anchor for this `instance_id` is a no-op. LOUD on a store or journal fault
/// (unlike replan's fail-safe non-anchor): the client explicitly asked for a
/// react chain, and an un-anchored chain cannot recover.
#[allow(clippy::too_many_arguments)]
fn write_react_anchor<J: Journal>(
    journal: &J,
    store: &LocalFsContentStore,
    projection: &mut Projection,
    folded_through: &mut u64,
    instance_id: [u8; INSTANCE_ID_LEN],
    step_salt: Option<[u8; 32]>,
    is_agentic_launch: bool,
    turn0: &Mote,
    warrant: &WarrantSpec,
    max_turns: u32,
    max_tool_calls: u32,
    require_approval: bool,
) -> Result<(), CoordinatorError> {
    if projection
        .react_rounds_of(&instance_id, &step_salt)
        .any(|r| r.turn == 0)
    {
        return Ok(()); // already anchored (idempotent re-submit / replay / re-park)
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
    // PR-9d: stage the run's grounding-context bundle (the seed's inline
    // `config_subset[CONTEXT_ITEMS_KEY]`) into the content store and record its ref on
    // the anchor, so a recovered coordinator re-derives per-turn context EDGE-FREE for
    // every successor turn (the seed's config_subset is GONE after recovery — only its
    // `mote_def_hash` survives, the same reason base_prompt_ref is anchored here).
    // Absent ⇒ `None`, byte-identical to pre-PR-9d. A store fault is LOUD (like base).
    let context_items_ref = match turn0
        .def
        .config_subset
        .get(&ConfigKey(CONTEXT_ITEMS_KEY.to_string()))
    {
        Some(items) => match store.put(&items.0) {
            Ok(r) => Some(r),
            Err(_) => {
                return Err(CoordinatorError::ReactSeedRefused(
                    "content store fault while staging the react context bundle",
                ))
            }
        },
        None => None,
    };
    // AGENTIC-VISION: record the run's grounding-image ref on the turn-0 anchor (the
    // seed's `config_subset[IMAGE_REF_KEY]`, a JSON string of the uploaded blob's
    // PutContent ref) so a recovered coordinator re-derives the per-turn image EDGE-FREE
    // for every successor turn — the seed's config_subset is GONE after recovery (only its
    // `mote_def_hash` survives), exactly the reason base_prompt_ref / context_items_ref
    // are anchored here. The image BYTES are already content-addressed (a client
    // `PutContent`), so the anchor records the EXISTING ref directly (no fresh `store.put`,
    // unlike context_items). A malformed value fail-closes the seed (the attached image
    // must never be silently dropped — the `model_exec` contract). Absent ⇒ `None`,
    // byte-identical to pre-AGENTIC-VISION.
    let image_ref = match turn0
        .def
        .config_subset
        .get(&ConfigKey(IMAGE_REF_KEY.to_string()))
    {
        // The SHARED tolerant decode (`ContentRef::from_arg`) — accepts the recipe
        // binder's JSON-string OR the chains-DSL params raw hex, and agrees with the
        // executor's read by construction (no drift). Fail-closed on a malformed value.
        Some(v) => match ContentRef::from_arg(&v.0) {
            Some(r) => Some(r),
            None => {
                return Err(CoordinatorError::ReactSeedRefused(
                    "image_ref must be a (JSON or raw) string of 64 hex chars",
                ))
            }
        },
        None => None,
    };
    let entry = JournalEntry::ReactRound {
        turn: 0,
        turn_mote_id: turn0.id,
        instance_id,
        base_prompt_ref: base_ref,
        warrant_ref,
        model_id: turn0.def.model_id.0.clone(),
        branch: ReactBranch::Pending,
        max_turns,
        max_tool_calls,
        // PR-R1: `step_salt` is now `Some` for BOTH a per-invocation run-level chain
        // (= seed MoteId) and an agentic step (= launch MoteId); `is_agentic_launch`
        // is the durable discriminator the settle disposition reads (only an agentic
        // launch disposes its launch mote). `None` step_salt = a legacy run-level
        // chain (old journal), always `is_agentic_launch = false`.
        step_salt,
        is_agentic_launch,
        context_items_ref,
        image_ref,
        // D114: record the run's approval posture on the turn-0 anchor so the gate
        // survives recovery (the seed config is dropped — only mote_def_hash survives).
        require_approval,
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
    store: Option<&LocalFsContentStore>,
    turn: &Mote,
    warrant_ref: ContentRef,
    warrant: WarrantSpec,
) {
    persist_def(store, &turn.def);
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

/// Register + admit a coordinator-materialized ReAct OBSERVATION (PR-2d-2) —
/// the WM tool Mote that fires a frozen `Tool` decision. Unlike a turn it
/// carries its ONE Data edge (to the proposing turn), so the ready-set releases
/// it exactly when the turn is committed — which it already is by the time the
/// settle freezes the `Tool` fact, so the observation is immediately leasable.
/// Idempotent (the `materialize_react_turn` contract): `register_mote`
/// overwrites only the DECLARED info (fold-derived flags — `effect_staged`,
/// failure markers — live separately and survive), and the id-keyed dispatch
/// inserts are no-ops on a repeat, so the settle may call this on every pass
/// until the observation commits, and recovery may call it again after a crash.
fn materialize_react_tool(
    projection: &mut Projection,
    dispatch: &mut Dispatch,
    store: Option<&LocalFsContentStore>,
    obs: &Mote,
    warrant_ref: ContentRef,
    warrant: WarrantSpec,
) {
    persist_def(store, &obs.def);
    projection.register_mote(RegisterMote {
        mote_id: obs.id,
        nd_class: obs.def.nd_class,
        effect_pattern: obs.def.effect_pattern,
        critic_for: None,
        is_topology_shaper: false,
        parents: obs.parents.clone(),
        warrant_ref,
    });
    dispatch.submitted.insert(obs.id);
    dispatch.defs.insert(obs.id, (obs.clone(), warrant));
}

/// PR-9b-2b: the post-drain settle passes — re-plan rounds (PR-2c-2), then
/// agentic-launch anchoring (turn-0 anchor + materialize for any parked launch whose
/// DAG parents committed), then the react/agentic chain settle (decode → freeze the
/// branch → advance under budget; an agentic chain's terminal Answer COMMITS its launch
/// mote, advancing the frozen DAG). Agentic anchoring runs BEFORE the react settle so a
/// freshly-anchored chain drives in the same drain. Each pass is idempotent + gated to
/// zero cost when its feature is unused. Extracted so `core_loop` stays under the line
/// budget (a pure refactor — same call order as the inline block it replaced).
#[allow(clippy::too_many_arguments)]
fn run_settle_passes<J: Journal>(
    journal: &J,
    store: Option<&LocalFsContentStore>,
    projection: &mut Projection,
    folded_through: &mut u64,
    dispatch: &mut Dispatch,
    react_cache: &mut ReactSettleCache,
    tool_registry: &dyn ToolRegistry,
) {
    settle_replan_rounds(journal, store, projection, folded_through, dispatch);
    settle_agentic_launches(journal, store, projection, folded_through, dispatch);
    // RC4c-2b: freeze any COMMITTED rerank BEFORE the react settle, so the react-rag
    // gate sees the frozen outcome and advances in the SAME drain (mirrors the
    // agentic-launch pass running before the react settle). Zero-cost when idle.
    settle_rerank_rounds(journal, store, projection, folded_through);
    // RC4c-2b: anchor + materialize the rerank for any held chat-rag/vision-rag answer,
    // and stage its reordered bundle once settled (runs AFTER the generic settle freezes
    // a committed chat-rag rerank). Zero-cost when the flag is off / no eligible answer.
    settle_chat_rag_reranks(journal, store, projection, folded_through, dispatch);
    settle_react_rounds(
        journal,
        store,
        projection,
        folded_through,
        dispatch,
        react_cache,
        tool_registry,
    );
}

/// PR-9b-2b: the settle/cache CHAIN key — `(instance_id, step_salt)`, not just the
/// run — so a run carrying BOTH a run-level react chain (`step_salt = None`) and one or
/// more agentic-step chains (`Some(launch MoteId)`) tracks each independently.
type ChainKey = ([u8; INSTANCE_ID_LEN], Option<[u8; 32]>);

/// The settle pass's incremental working set (adversarial-review finding,
/// PR-2d-1): `react_rounds` only ever GROWS in serve's shared journal, so a
/// naive per-drain pass over every fact of every chain is O(total-runs²) at the
/// drain rate, forever. This cache keeps the pass proportional to the ACTIVE
/// working set instead:
///
/// - `settled` — chains whose latest branch can never change again (a terminal
///   `Answer`/`DeadLettered`, or a budget-exhausted `Tool` tail). Branches are
///   FROZEN at append and a settled chain accepts no new facts, so membership is
///   monotonic — skipping is always sound. Purely in-memory: recovery starts
///   empty and the first pass re-derives it from the durable facts.
/// - `active` — the distinct unsettled chains, REBUILT only when the folded
///   fact count changes (`seen_facts`); between fact-appends the pass iterates
///   this (usually tiny) list and touches nothing else.
#[derive(Default)]
struct ReactSettleCache {
    seen_facts: usize,
    active: Vec<ChainKey>,
    settled: BTreeSet<ChainKey>,
}

/// Whether one settle pass left a chain able to settle again ([`ReactChainStatus::Active`])
/// or permanently frozen ([`ReactChainStatus::Settled`] — skip on every later pass).
#[derive(PartialEq, Eq)]
enum ReactChainStatus {
    Active,
    Settled,
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
    cache: &mut ReactSettleCache,
    tool_registry: &dyn ToolRegistry,
) {
    if !projection.has_react_turn() {
        return; // the zero-cost sentinel: react-free runs pay one bool read
    }
    let Some(store) = store else {
        return; // anchored chains need a store (the anchor write guaranteed one)
    };
    // Rebuild the active working set only when new facts folded since the last
    // pass (anchors, settles, advances). Between appends the pass touches ONLY
    // the (usually tiny) active list — never the full accumulated fact log.
    if projection.react_rounds().len() != cache.seen_facts {
        cache.active = projection
            .react_chains()
            .filter(|chain| !cache.settled.contains(chain))
            .collect();
        cache.seen_facts = projection.react_rounds().len();
    }
    let mut still_active: Vec<ChainKey> = Vec::with_capacity(cache.active.len());
    for (instance_id, step_salt) in std::mem::take(&mut cache.active) {
        let status = settle_react_chain(
            journal,
            store,
            projection,
            folded_through,
            dispatch,
            instance_id,
            step_salt,
            tool_registry,
        );
        if status == ReactChainStatus::Settled {
            cache.settled.insert((instance_id, step_salt));
        } else {
            still_active.push((instance_id, step_salt));
        }
    }
    cache.active = still_active;
    // The pass itself may have appended facts; refresh so the next drain does
    // not rebuild for our own appends.
    cache.seen_facts = projection.react_rounds().len();
}

/// Settle ONE chain — the `(instance_id, step_salt)` pair (see
/// [`settle_react_rounds`]). For an AGENTIC chain (`step_salt.is_some()`) this
/// wraps the run-level [`drive_react_chain`] with the terminal launch disposition
/// ([`finalize_agentic_launch`]): a terminal `Answer` COMMITS the launch mote (so
/// the frozen DAG advances), a budget-exhausted / dead-lettered chain fail-closed
/// dead-letters it. The run-level chain (`None`) is byte-unchanged (driver only).
#[allow(clippy::too_many_arguments)]
fn settle_react_chain<J: Journal>(
    journal: &J,
    store: &LocalFsContentStore,
    projection: &mut Projection,
    folded_through: &mut u64,
    dispatch: &mut Dispatch,
    instance_id: [u8; INSTANCE_ID_LEN],
    step_salt: Option<[u8; 32]>,
    tool_registry: &dyn ToolRegistry,
) -> ReactChainStatus {
    let status = drive_react_chain(
        journal,
        store,
        projection,
        folded_through,
        dispatch,
        instance_id,
        step_salt,
        tool_registry,
    );
    // PR-9b-2b: an AGENTIC chain that reached a permanent state must dispose of its
    // LAUNCH mote (commit on Answer / dead-letter otherwise) before it is truly
    // settled — the launch-commit is drain-driven + idempotent, so the chain stays
    // ACTIVE until the launch reaches a terminal state (RISK 4: the launch def is
    // lost on a crash and only repopulated by re-submit). The run-level chain
    // (`None`) terminates the run via the seed-swap and needs no launch disposition.
    match step_salt {
        // PR-R1: a per-invocation RUN-LEVEL chain is ALSO salted now (by its
        // swapped-out seed MoteId), so `Some` no longer implies "agentic". The
        // durable `is_agentic_launch` flag on the chain's anchor is the discriminator:
        // only a launched AGENTIC step runs the launch disposition (commit-on-Answer /
        // dead-letter) to advance the frozen DAG; a run-level chain's terminal Answer
        // settles the run directly (the `_` arm). The flag is recovery-stable (read
        // off the committed anchor), so a reaped agentic launch still resumes its
        // disposition while a run-level chain never busy-loops the settle cache.
        Some(salt)
            if status == ReactChainStatus::Settled
                && chain_is_agentic_launch(projection, instance_id, Some(salt)) =>
        {
            finalize_agentic_launch(
                journal,
                store,
                projection,
                folded_through,
                dispatch,
                instance_id,
                salt,
            )
        }
        _ => status,
    }
}

/// PR-R1: is the chain `(instance_id, step_salt)` a launched AGENTIC step (needs the
/// launch disposition) or a run-level react chain (settles on its own Answer)? Reads
/// the durable `is_agentic_launch` flag off the chain's anchor (turn 0) — recovery-
/// stable, so the launch disposition resumes correctly after a crash. A legacy
/// `None`-salted chain (old journal) and a per-invocation run-level chain both report
/// `false`; only an agentic-step anchor reports `true`.
fn chain_is_agentic_launch(
    projection: &Projection,
    instance_id: [u8; INSTANCE_ID_LEN],
    step_salt: Option<[u8; 32]>,
) -> bool {
    projection
        .react_rounds_of(&instance_id, &step_salt)
        .find(|r| r.turn == 0)
        .is_some_and(|r| r.is_agentic_launch)
}

/// RC2 loop hardening: the tool calls fired by PRIOR turns of this chain — the
/// settled `Tool`/`ToolBatch` turns with `turn < this_turn`, re-decoded from their
/// committed output through the ONE authority gate ([`kx_toolcall::parse_tool_calls`]).
/// Pure over committed facts ⇒ recovery-stable; bounded by the per-run turn budget,
/// so the re-decode is cheap. Used to fail-closed a redundant re-proposal (the
/// minimal dedup slice).
fn collect_prior_fired_calls(
    store: &LocalFsContentStore,
    projection: &Projection,
    rounds: &[ReactRoundRecord],
    this_turn: u32,
    warrant: &WarrantSpec,
    max_args: usize,
) -> Vec<kx_toolcall::ToolCall> {
    // The latest fact per prior turn (a turn is `Pending` then a frozen branch;
    // highest seq wins) — the same selection `resolve_parent_context` uses.
    let mut latest: BTreeMap<u32, &ReactRoundRecord> = BTreeMap::new();
    for r in rounds.iter().filter(|r| r.turn < this_turn) {
        latest
            .entry(r.turn)
            .and_modify(|slot| {
                if r.seq > slot.seq {
                    *slot = r;
                }
            })
            .or_insert(r);
    }
    let mut out: Vec<kx_toolcall::ToolCall> = Vec::new();
    for r in latest.values() {
        if !matches!(
            r.branch,
            ReactBranch::Tool { .. } | ReactBranch::ToolBatch { .. }
        ) {
            continue;
        }
        let Some(result_ref) = projection.result_ref_of(&r.turn_mote_id) else {
            continue;
        };
        let Ok(raw) = store.get(&result_ref) else {
            continue;
        };
        if let Ok(calls) = kx_toolcall::parse_tool_calls(raw.as_ref(), warrant, max_args) {
            out.extend(calls);
        }
    }
    out
}

/// Drive ONE chain's reason→tool→observe loop one pass (the PR-2d-1/2d-2 logic,
/// now chain-keyed by `(instance_id, step_salt)`). Bounded: at most one branch fact
/// plus one advance per pass (the next pass continues). Returns whether the chain can
/// ever settle again (the cache classification). The chain's `step_salt` selects
/// the salt-1 (run-level) vs salt-2 (agentic) identity builders via the `anchor`'s
/// own `step_salt` field, so the drive helpers stay chain-shape agnostic.
#[allow(clippy::too_many_lines, clippy::too_many_arguments)]
fn drive_react_chain<J: Journal>(
    journal: &J,
    store: &LocalFsContentStore,
    projection: &mut Projection,
    folded_through: &mut u64,
    dispatch: &mut Dispatch,
    instance_id: [u8; INSTANCE_ID_LEN],
    step_salt: Option<[u8; 32]>,
    tool_registry: &dyn ToolRegistry,
) -> ReactChainStatus {
    let rounds: Vec<ReactRoundRecord> = projection
        .react_rounds_of(&instance_id, &step_salt)
        .cloned()
        .collect();
    // The run-fixed anchor (turn 0) carries base_prompt_ref / warrant_ref /
    // model_id / the durable budget caps.
    let Some(anchor) = rounds.iter().find(|r| r.turn == 0) else {
        // Facts without an anchor (defensive) — nothing recoverable, ever.
        return ReactChainStatus::Settled;
    };
    // The work frontier: the latest fact for the highest turn (a turn's facts are
    // anchor/advance `Pending` then a settled branch; highest seq wins).
    let Some(latest) = rounds
        .iter()
        .max_by(|a, b| a.turn.cmp(&b.turn).then(a.seq.cmp(&b.seq)))
    else {
        return ReactChainStatus::Settled;
    };
    let turn = latest.turn;

    match &latest.branch {
        // Terminal branches: the chain is done (Answer) or dead (DeadLettered).
        // Branches are frozen and a terminal chain accepts no new facts — skip
        // it on every later pass (the cache classification).
        ReactBranch::Answer | ReactBranch::DeadLettered => ReactChainStatus::Settled,
        // PR-3 (A2): a frozen `Rejected` round whose advance was interrupted (a
        // crash between the Rejected fact and the next turn's `Pending` fact) —
        // re-drive the advance idempotently (the `turn + 1` dedup in
        // `advance_react_chain` guards a double-spawn; budget exhaustion freezes
        // the loud terminal). On the live happy path the freeze pass already
        // advanced, so this arm fires only on recovery.
        ReactBranch::Rejected { .. } => advance_react_chain(
            journal,
            store,
            projection,
            folded_through,
            dispatch,
            anchor,
            &rounds,
            turn,
        ),
        // A frozen Tool decision (just decided this pass on an earlier drain, or
        // an advance interrupted by a crash): PR-2d-2 — drive the OBSERVATION
        // lifecycle (materialize → fire via the worker → commit), then advance
        // to the next turn under the budget gate. A single-call `Tool` is a
        // 1-element batch through the SAME drain (call_index 0 ⇒ byte-identical).
        ReactBranch::Tool {
            tool_id,
            tool_version,
        } => progress_tool_batch(
            journal,
            store,
            projection,
            folded_through,
            dispatch,
            anchor,
            &rounds,
            turn,
            latest.turn_mote_id,
            &[(tool_id.clone(), tool_version.clone())],
            tool_registry,
        ),
        // T-MULTI-ELEMENT-TOOLCALLS: a frozen ToolBatch decision — drive ALL N
        // observations (call-indexed), advancing only once EVERY one commits.
        ReactBranch::ToolBatch { calls } => progress_tool_batch(
            journal,
            store,
            projection,
            folded_through,
            dispatch,
            anchor,
            &rounds,
            turn,
            latest.turn_mote_id,
            calls,
            tool_registry,
        ),
        // The in-flight turn: settle it once its Mote reaches a terminal state.
        ReactBranch::Pending => {
            let turn_state = projection.state_of(&latest.turn_mote_id);
            if !is_terminal(turn_state) {
                return ReactChainStatus::Active; // still in flight (Pending/Scheduled)
            }
            if turn_state != MoteState::Committed {
                // Distinguish the failure FLAVORS before freezing an irreversible
                // branch (adversarial-review finding, PR-2d-1):
                //
                // - A pre-commit CRASH flavor (`WorkerCrashed`/`TimedOut`, the
                //   `is_pre_commit_crash` set) is NOT chain-death: the reaped
                //   worker's `ReportCommit` may still be in flight (the fold
                //   deliberately lets a LATER Committed win over
                //   `failed_pending_reattempt` for exactly this race), and a
                //   genuinely dead worker leaves the turn STUCK-but-operator-
                //   recoverable — the standing non-PURE crash semantics
                //   (`redispatch_admissible`), never an auto-dead-letter. So we
                //   LEAVE the frontier Pending: a late commit settles normally on
                //   a later pass; a stuck turn is visible via `ListReactTurns`.
                // - A TERMINAL failure (F4 `ReportFailure` dead-letter /
                //   validator-rejected / …), a repudiation, or an inconsistency
                //   IS chain-death ⇒ freeze `DeadLettered`.
                // The fold records `failure_reason` ONLY for terminal flavors
                // (`is_pre_commit_crash` reasons leave it `None`), so a Failed
                // state with no recorded reason IS the crash flavor; a recorded
                // reason is re-checked defensively against the classifier.
                let crash_retryable = turn_state == MoteState::Failed
                    && projection
                        .failure_reason_of(&latest.turn_mote_id)
                        .is_none_or(kx_journal::is_pre_commit_crash);
                if crash_retryable {
                    // The commit may still land; never discard it. Stays ACTIVE
                    // so a later pass re-examines the frontier.
                    return ReactChainStatus::Active;
                }
                append_react_branch(
                    journal,
                    projection,
                    folded_through,
                    anchor,
                    latest.turn_mote_id,
                    turn,
                    ReactBranch::DeadLettered,
                );
                return ReactChainStatus::Settled;
            }
            // Committed: decode the RAW output via the ONE authority gate.
            let Some(result_ref) = projection.result_ref_of(&latest.turn_mote_id) else {
                return ReactChainStatus::Active; // defensive: committed without a result_ref
            };
            let Ok(raw) = store.get(&result_ref) else {
                return ReactChainStatus::Active; // store fault — retry next pass (fail-safe)
            };
            let Ok(warrant_bytes) = store.get(&anchor.warrant_ref) else {
                return ReactChainStatus::Active;
            };
            let Ok(warrant) = decode_warrant(warrant_bytes.as_ref()) else {
                return ReactChainStatus::Active;
            };
            let max_args = kx_toolcall::max_args_bytes(&warrant);
            // T-MULTI-ELEMENT-TOOLCALLS: decode ALL proposed calls (the plural gate).
            // A normal completion IS the final answer; ≥1 grant-checked, schema-valid
            // call freezes `Tool` (one) or `ToolBatch` (N — fire all N, no silent cap);
            // any call that fails validation (or a malformed/oversize/ungranted
            // envelope) freezes a `Rejected` round (the next turn self-corrects under
            // the budget). The SINGLE path is byte-identical to PR-2d-2's `Tool` arm.
            let decoded = kx_toolcall::parse_tool_calls(raw.as_ref(), &warrant, max_args);
            // RC2 loop hardening (minimal dedup slice): a SINGLE `Tool` proposal that
            // EXACTLY repeats a call already fired this run is frozen `Rejected` — the
            // re-prompt steers the model to use the result it already has (bounded loop
            // progress) instead of re-firing the identical effect. Pure over committed
            // facts (a re-decode of prior `Tool`/`ToolBatch` turns) ⇒ recovery-stable;
            // the reason is the SHARED `kx_toolcall` twin string. Only the lone-`Tool`
            // case is deduped (the dominant loop-waste); a `ToolBatch` rides the budget.
            let dup_reason: Option<String> = match &decoded {
                Ok(calls) if calls.len() == 1 => {
                    let prior = collect_prior_fired_calls(
                        store, projection, &rounds, turn, &warrant, max_args,
                    );
                    kx_toolcall::is_duplicate_call(&calls[0], &prior).then(|| {
                        crate::react_shape::bounded_reason(kx_toolcall::duplicate_call_reason(
                            &calls[0],
                        ))
                    })
                }
                _ => None,
            };
            let branch = match (decoded, dup_reason) {
                // A duplicate single call overrides the would-be `Tool` with `Rejected`.
                (Ok(_), Some(reason)) => ReactBranch::Rejected { reason },
                (Ok(calls), None) => settle_calls_to_branch(&calls, tool_registry),
                // Malformed / oversize / a name that decoded to no grant ⇒ a
                // `Rejected` round (the committed turn fact remains; the model
                // gets a chance to re-propose under the budget).
                (Err(error), _) => ReactBranch::Rejected {
                    // Twin of `kx_model_harness::react_reason::decode_error_reason`
                    // (byte-identical — pinned across the dep wall so the re-prompted
                    // turn's MoteId matches on a cold re-fold).
                    reason: crate::react_shape::bounded_reason(match error {
                        kx_toolcall::DecodeError::Malformed { diagnostic } => {
                            format!("the tool proposal was malformed: {diagnostic}")
                        }
                        kx_toolcall::DecodeError::UngrantedTool { name, version } => format!(
                            "the proposed tool `{}@{}` is not granted to this run",
                            name.0, version.0
                        ),
                        kx_toolcall::DecodeError::Ambiguous { name, candidates } => format!(
                            "the tool name `{}` is ambiguous — use the full id: {}",
                            name.0,
                            candidates
                                .iter()
                                .map(|c| c.0.as_str())
                                .collect::<Vec<_>>()
                                .join(" OR ")
                        ),
                        kx_toolcall::DecodeError::Oversize { got, max } => format!(
                            "the proposed tool arguments are too large \
                             ({got} bytes > {max} max)"
                        ),
                    }),
                },
            };
            // The calls this branch will drive (1 for `Tool`, N for `ToolBatch`, none
            // otherwise) — used to start the observation drain in the same pass.
            let advanced: Option<Vec<(String, String)>> = match &branch {
                ReactBranch::Tool {
                    tool_id,
                    tool_version,
                } => Some(vec![(tool_id.clone(), tool_version.clone())]),
                ReactBranch::ToolBatch { calls } => Some(calls.clone()),
                _ => None,
            };
            let rejected = matches!(branch, ReactBranch::Rejected { .. });
            append_react_branch(
                journal,
                projection,
                folded_through,
                anchor,
                latest.turn_mote_id,
                turn,
                branch,
            );
            if let Some(calls) = advanced {
                // Re-read the folded rounds (the Tool/ToolBatch fact just folded) and
                // start the observation drain in the same pass (no wasted drain) —
                // materializes the N observations; the advance to the next turn
                // happens on a later pass, once EVERY observation commits (the
                // back-pressure gate in `progress_tool_batch`).
                let rounds: Vec<ReactRoundRecord> = projection
                    .react_rounds_of(&instance_id, &step_salt)
                    .cloned()
                    .collect();
                progress_tool_batch(
                    journal,
                    store,
                    projection,
                    folded_through,
                    dispatch,
                    anchor,
                    &rounds,
                    turn,
                    latest.turn_mote_id,
                    &calls,
                    tool_registry,
                )
            } else if rejected {
                // PR-3 (A2): a refused proposal just froze a `Rejected` round —
                // advance to the next (re-prompted) turn under the budget gate.
                // At budget exhaustion `advance_react_chain` freezes the loud
                // terminal `DeadLettered` (never a silent wedge).
                let rounds: Vec<ReactRoundRecord> = projection
                    .react_rounds_of(&instance_id, &step_salt)
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
                )
            } else {
                // Answer just froze — the chain is done.
                ReactChainStatus::Settled
            }
        }
    }
}

/// T-MULTI-ELEMENT-TOOLCALLS — the settle authority's branch decision for the
/// decoded call list. This is the ONE decode/validate gate (PR-2d-2): resolve EACH
/// proposed tool and validate its args against the typed `inputSchema` FAIL-CLOSED
/// BEFORE freezing the irreversible branch, so a frozen `Tool`/`ToolBatch` fact
/// GUARANTEES registered, schema-valid args (the lease-time re-derivation can then
/// only fail on I/O). ALL-OR-NOTHING: any one call that fails resolution/validation
/// freezes the WHOLE turn `Rejected` (one frozen branch/turn, no partial fire) — the
/// next turn self-corrects under the budget (PR-3/A2). `[]` ⇒ `Answer`; one valid
/// call ⇒ `Tool` (byte-identical to PR-2d-2); N≥2 valid calls ⇒ `ToolBatch` (fire all
/// N), bounded by [`kx_journal::MAX_TOOL_BATCH_CALLS`] (a batch over the cap is a LOUD
/// `Rejected`, never a silent truncation — BUG-27).
fn settle_calls_to_branch(
    calls: &[kx_toolcall::ToolCall],
    tool_registry: &dyn ToolRegistry,
) -> ReactBranch {
    if calls.is_empty() {
        // A normal completion IS the final answer (the harness two-fact contract).
        return ReactBranch::Answer;
    }
    let mut validated: Vec<(String, String)> = Vec::with_capacity(calls.len());
    for call in calls {
        match tool_registry.lookup(&call.name, &call.version) {
            None => {
                return ReactBranch::Rejected {
                    reason: crate::react_shape::bounded_reason(format!(
                        "the proposed tool `{}@{}` is not granted to this run \
                         or is no longer registered",
                        call.name.0, call.version.0
                    )),
                };
            }
            Some(def) => {
                if let Some(schema) = def.input_schema.as_ref() {
                    if let Err(error) = kx_tool_registry::validate_args(schema, &call.args_bytes) {
                        return ReactBranch::Rejected {
                            reason: crate::react_shape::bounded_reason(format!(
                                "the arguments for `{}@{}` do not match its \
                                 inputSchema: {error}",
                                call.name.0, call.version.0
                            )),
                        };
                    }
                }
            }
        }
        validated.push((call.name.0.clone(), call.version.0.clone()));
    }
    if validated.len() == 1 {
        // swap_remove(0) on a len-1 vec returns the sole element (no panic, no expect).
        let (tool_id, tool_version) = validated.swap_remove(0);
        ReactBranch::Tool {
            tool_id,
            tool_version,
        }
    } else if validated.len() > kx_journal::MAX_TOOL_BATCH_CALLS {
        // A per-turn batch over the cap dead-letters loudly rather than firing a
        // silent prefix (no silent cap; the journal could not encode it either).
        ReactBranch::Rejected {
            reason: crate::react_shape::bounded_reason(format!(
                "the model proposed {} tool calls in one turn, exceeding the \
                 per-turn batch cap of {}",
                validated.len(),
                kx_journal::MAX_TOOL_BATCH_CALLS
            )),
        }
    } else {
        ReactBranch::ToolBatch { calls: validated }
    }
}

/// D114: the gate's decision for ONE world-mutating tool call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ApprovalGate {
    /// An operator `Granted` is folded (or no gate applies) — materialize + fire.
    Proceed,
    /// `Requested` (just appended, or already pending) — withhold; re-check next pass.
    Wait,
    /// `Denied`/`Expired` — the chain dead-letters loudly (fail-closed).
    DeadLetter,
}

/// D114: does an idempotency class denote an IRREVERSIBLE world-mutating action that
/// the HITL gate must hold for operator approval? `Staged`/`AtLeastOnce` have no
/// self-closing dedup mechanism (a double-fire is a real-world side effect), so they
/// are gated; `Token`/`Readback` self-close (and read-only/Pure tools never reach
/// here), so they auto-proceed — matching the corpus posture "auto for read/diagnose,
/// human-OK for email/call/DB-write/prod-change".
fn tool_needs_approval(class: Option<IdempotencyClass>) -> bool {
    matches!(
        class,
        Some(IdempotencyClass::Staged | IdempotencyClass::AtLeastOnce)
    )
}

/// Wall-clock millis since the Unix epoch for an approval fact's AUDIT timestamps
/// (off-DAG — never hashed, never an identity input; SN-8). A pre-epoch/overflow
/// reading collapses to `0` (panic-free), exactly like [`crate::clock::SystemClock`].
fn approval_now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| u64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

/// Append one off-DAG [`JournalEntry::Approval`] handshake step + fold it. Idempotent
/// by the caller (the gate only requests once per `request_id`); a fold/append fault
/// is LOUD (the chain re-checks next pass). Returns whether the entry folded.
fn append_approval<J: Journal>(
    journal: &J,
    projection: &mut Projection,
    folded_through: &mut u64,
    instance_id: [u8; INSTANCE_ID_LEN],
    request_id: [u8; kx_journal::APPROVAL_REQUEST_ID_LEN],
    awaiting_mote_id: MoteId,
    state: ApprovalState,
) {
    let entry = JournalEntry::Approval {
        instance_id,
        request_id,
        awaiting_mote_id,
        state,
        seq: 0,
    };
    match journal.append(entry) {
        Ok(durable) => {
            let seq = durable.seq();
            if seq > *folded_through && projection.fold(&durable).is_ok() {
                *folded_through = seq;
            }
        }
        Err(error) => tracing::error!(%error, "failed to append Approval handshake fact"),
    }
}

/// D114: the gate decision for a gated world-mutating observation `obs`. On the FIRST
/// encounter (no folded handshake) it appends `Requested` and WAITs (the action is
/// withheld staged-not-committed). A folded `Granted` PROCEEDS (the authorized action
/// fires exactly once via the existing StageThenCommit fence + idempotent
/// `materialize_react_tool`); `Denied`/`Expired` DEAD-LETTERS. The `request_id` is
/// deterministic (`instance_id ‖ obs.id`), so a cold recovery re-derives the SAME id
/// and reads the committed decision — never re-asks, never double-fires.
fn approval_gate_decision<J: Journal>(
    journal: &J,
    projection: &mut Projection,
    folded_through: &mut u64,
    anchor: &ReactRoundRecord,
    obs_id: MoteId,
    tool_id: &str,
    tool_version: &str,
) -> ApprovalGate {
    let request_id = approval_request_id(&anchor.instance_id, &obs_id);
    match projection
        .approval_latest_for(&request_id)
        .map(|r| &r.state)
    {
        Some(ApprovalState::Granted { .. }) => ApprovalGate::Proceed,
        Some(ApprovalState::Denied { .. } | ApprovalState::Expired { .. }) => {
            ApprovalGate::DeadLetter
        }
        Some(ApprovalState::Requested { .. }) => ApprovalGate::Wait, // already pending
        None => {
            // First encounter: request operator approval, then withhold the action.
            let now = approval_now_ms();
            let intent = format!("world-mutating tool call: {tool_id}@{tool_version}");
            append_approval(
                journal,
                projection,
                folded_through,
                anchor.instance_id,
                request_id,
                obs_id,
                ApprovalState::Requested {
                    tool_id: tool_id.to_string(),
                    tool_version: tool_version.to_string(),
                    intent: truncate_for_approval(&intent),
                    deadline_unix_ms: 0, // 0 = operator-driven (no auto-expiry this increment)
                    created_unix_ms: now,
                },
            );
            tracing::info!(
                observation = ?obs_id,
                tool = %tool_id,
                "world-mutating action withheld — awaiting operator approval (D114)"
            );
            ApprovalGate::Wait
        }
    }
}

/// Truncate an approval display string at a char boundary to the journal cap.
fn truncate_for_approval(s: &str) -> String {
    let cap = kx_journal::MAX_APPROVAL_TEXT_LEN;
    if s.chars().count() <= cap {
        return s.to_string();
    }
    s.chars().take(cap).collect()
}

/// M11/D115: the deterministic spend ESTIMATE (micro-USD) of a react chain that has
/// run `turns_used` model turns + `committed_tool_calls` tool calls, at the host's
/// operator-priced rates. A pure fold mirror of the budget counters (state.rs ~3605);
/// re-derived per pass, never a live counter (D115.2). `pending_calls` lets the
/// pre-stage gate price the calls ABOUT to fire so the ceiling is enforced BEFORE the
/// world-mutating dispatch (fail-closed), not after.
fn react_projected_spend_micro_usd(
    rounds: &[ReactRoundRecord],
    turn: u32,
    pending_calls: u32,
) -> u64 {
    let committed_tool_calls: u32 = u32::try_from(
        rounds
            .iter()
            .map(|r| match &r.branch {
                ReactBranch::Tool { .. } | ReactBranch::Rejected { .. } => 1,
                ReactBranch::ToolBatch { calls } => calls.len(),
                _ => 0,
            })
            .sum::<usize>(),
    )
    .unwrap_or(u32::MAX);
    let turns_used = turn.saturating_add(1);
    let tool_calls = committed_tool_calls.saturating_add(pending_calls);
    kx_pricing::PriceBook::default()
        .with_env_overrides()
        .estimate_spend(u64::from(turns_used), u64::from(tool_calls))
}

/// T-MULTI-ELEMENT-TOOLCALLS — drive a frozen tool decision past its OBSERVATION
/// lifecycle: ONE observation for a `Tool` branch, N CALL-INDEXED observations for a
/// `ToolBatch`. The chain advances to the next (re-prompted) turn ONLY once EVERY
/// observation has committed (BACK-PRESSURE). Each observation Mote is a PURE function
/// of the durable facts, so this is idempotent across passes, crashes, and recovery.
/// D114/M11: the per-call HITL approval barrier + the per-batch cost-ceiling pre-stage
/// check interpose before each world-mutating dispatch (the helpers above).
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn progress_tool_batch<J: Journal>(
    journal: &J,
    store: &LocalFsContentStore,
    projection: &mut Projection,
    folded_through: &mut u64,
    dispatch: &mut Dispatch,
    anchor: &ReactRoundRecord,
    rounds: &[ReactRoundRecord],
    turn: u32,
    turn_mote_id: MoteId,
    calls: &[(String, String)],
    tool_registry: &dyn ToolRegistry,
) -> ReactChainStatus {
    // The chain warrant — load once for the whole batch (store fault ⇒ retry next pass).
    let Ok(warrant_bytes) = store.get(&anchor.warrant_ref) else {
        return ReactChainStatus::Active;
    };
    let Ok(warrant) = decode_warrant(warrant_bytes.as_ref()) else {
        return ReactChainStatus::Active;
    };
    // M11/D115: cost-ceiling pre-stage enforcement (the runaway-agent kill-switch +
    // FinOps ceiling). When the run warrant set a POSITIVE ceiling (0 = unset/OFF),
    // price the chain's committed turns/tool-calls (this batch already folded into
    // `rounds`) and dead-letter BEFORE the batch's observations dispatch when the
    // estimate exceeds the ceiling. A pure fold over committed facts (D115.2); the
    // broker precheck is the SN-8 backstop for non-react WM dispatch.
    let ceiling = warrant.cost_ceiling.micro_usd;
    if ceiling > 0 {
        let projected = react_projected_spend_micro_usd(rounds, turn, 0);
        if projected > ceiling {
            tracing::warn!(
                turn,
                projected_micro_usd = projected,
                ceiling_micro_usd = ceiling,
                "react chain spend exceeds cost_ceiling — dead-lettering (M11/D115)"
            );
            append_react_branch(
                journal,
                projection,
                folded_through,
                anchor,
                turn_mote_id,
                turn,
                ReactBranch::DeadLettered,
            );
            return ReactChainStatus::Settled;
        }
    }
    let mut all_committed = true;
    for (i, (tool_id, tool_version)) in calls.iter().enumerate() {
        let call_index = u32::try_from(i).unwrap_or(u32::MAX);
        let obs = crate::react_shape::build_chain_tool(
            &ModelId(anchor.model_id.clone()),
            &ToolName(tool_id.clone()),
            &ToolVersion(tool_version.clone()),
            turn,
            &anchor.instance_id,
            anchor.step_salt,
            call_index,
            turn_mote_id,
        );
        let obs_state = projection.state_of(&obs.id);
        if obs_state == MoteState::Committed {
            continue; // this call's observation has landed — keep checking the rest
        }
        if is_terminal(obs_state) {
            // The flavor guard, line-for-line the turn arm's: only a recorded
            // TERMINAL failure reason (or a non-Failed anomaly) kills the chain.
            let crash_retryable = obs_state == MoteState::Failed
                && projection
                    .failure_reason_of(&obs.id)
                    .is_none_or(kx_journal::is_pre_commit_crash);
            if crash_retryable {
                return ReactChainStatus::Active;
            }
            // One hard-failed call dead-letters the WHOLE chain (a batch with a
            // non-existent observation is never fed into a next turn's assemble).
            append_react_branch(
                journal,
                projection,
                folded_through,
                anchor,
                turn_mote_id,
                turn,
                ReactBranch::DeadLettered,
            );
            return ReactChainStatus::Settled;
        }
        // Pending / Scheduled / never-materialized ⇒ (re-)materialize idempotently.
        // PR-9a (BUG-27): an observation whose args can never resolve (the granted
        // tool was DEREGISTERED / re-schema'd since the branch froze) would otherwise
        // re-materialize forever — re-derive on the sole writer and dead-letter on a
        // PERMANENT fault. `resolve_tool_args` self-recovers THIS observation's
        // call_index from the frozen branch fact, so the right call's args are checked.
        if !dispatch.tracker.is_leased(obs.id) {
            if let ArgResolution::Permanent { reason } =
                resolve_tool_args(&obs, &warrant, projection, Some(store), tool_registry)
            {
                tracing::warn!(
                    turn,
                    call_index,
                    observation = ?obs.id,
                    tool = %tool_id,
                    %reason,
                    "react observation can never resolve its args — dead-lettering the chain (BUG-27)"
                );
                append_react_branch(
                    journal,
                    projection,
                    folded_through,
                    anchor,
                    turn_mote_id,
                    turn,
                    ReactBranch::DeadLettered,
                );
                dispatch.submitted.remove(&obs.id);
                dispatch.defs.remove(&obs.id);
                dispatch.tracker.resolve_committed(obs.id);
                return ReactChainStatus::Settled;
            }
        }
        // D114: HITL approval barrier. If the chain requires approval AND this tool is
        // an irreversible world-mutating action (idempotency class Staged/AtLeastOnce),
        // hold the observation staged-not-committed until an operator decision folds.
        // The gate is a PURE function of the deterministic obs id + folded handshake
        // facts, so recovery re-derives it identically (a committed `Granted` proceeds
        // exactly once; a crash before any decision simply re-requests idempotently).
        if anchor.require_approval {
            let class = tool_registry
                .lookup(
                    &ToolName(tool_id.clone()),
                    &ToolVersion(tool_version.clone()),
                )
                .map(|d| d.idempotency_class);
            if tool_needs_approval(class) {
                match approval_gate_decision(
                    journal,
                    projection,
                    folded_through,
                    anchor,
                    obs.id,
                    tool_id,
                    tool_version,
                ) {
                    ApprovalGate::Proceed => {} // granted — fall through to dispatch
                    ApprovalGate::Wait => {
                        all_committed = false; // withheld — chain stays Active, re-check next pass
                        continue;
                    }
                    ApprovalGate::DeadLetter => {
                        append_react_branch(
                            journal,
                            projection,
                            folded_through,
                            anchor,
                            turn_mote_id,
                            turn,
                            ReactBranch::DeadLettered,
                        );
                        return ReactChainStatus::Settled;
                    }
                }
            }
        }
        materialize_react_tool(
            projection,
            dispatch,
            Some(store),
            &obs,
            anchor.warrant_ref,
            warrant.clone(),
        );
        tracing::info!(
            turn,
            call_index,
            observation = ?obs.id,
            tool = %tool_id,
            "react observation materialized"
        );
        all_committed = false;
    }
    if all_committed {
        // RC4c-2b react-rag gate: durably rerank the retrieved passages (if enabled)
        // BEFORE advancing, so the next turn reasons over the reranked order. In flight
        // ⇒ Active (re-check next pass); settled / not-applicable ⇒ fall through. The
        // rerank is OFF-BUDGET (a `ReRankRound` fact, never a `ReactRound`).
        if let Some(status) = maybe_gate_on_rerank(
            journal,
            store,
            projection,
            folded_through,
            dispatch,
            anchor,
            turn,
            turn_mote_id,
            calls,
        ) {
            return status;
        }
        // BACK-PRESSURE: every observation in the batch has committed ⇒ advance to
        // the next turn under the budget gate (re-prompt ONCE with all N results).
        return advance_react_chain(
            journal,
            store,
            projection,
            folded_through,
            dispatch,
            anchor,
            rounds,
            turn,
        );
    }
    ReactChainStatus::Active
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
        .react_rounds_of(&anchor.instance_id, &anchor.step_salt)
        .any(|r| r.turn == turn && r.branch == branch)
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
        // v9 (PR-9b-2b): a settled branch joins the SAME chain as its anchor —
        // `None` for a legacy run-level chain, `Some(salt)` otherwise.
        step_salt: anchor.step_salt,
        // PR-R1: every round of a chain inherits its anchor's launch discriminator,
        // so any round (not just turn 0) reports the chain's true kind.
        is_agentic_launch: anchor.is_agentic_launch,
        // PR-9d: every round inherits the anchor's grounding-context ref too, so the
        // resolver finds it on ANY turn-0 record (robust to fold/find order).
        context_items_ref: anchor.context_items_ref,
        // AGENTIC-VISION: every round inherits the anchor's grounding-image ref the same
        // way, so the resolver finds the image on ANY turn-0 record.
        image_ref: anchor.image_ref,
        // D114: every round inherits the anchor's approval posture so the gate finds it
        // on ANY record of the chain (robust to fold/find order).
        require_approval: anchor.require_approval,
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
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn advance_react_chain<J: Journal>(
    journal: &J,
    store: &LocalFsContentStore,
    projection: &mut Projection,
    folded_through: &mut u64,
    dispatch: &mut Dispatch,
    anchor: &ReactRoundRecord,
    rounds: &[ReactRoundRecord],
    turn: u32,
) -> ReactChainStatus {
    // Dedup: the successor turn already exists (live double-settle or recovery).
    // CROSS-VERSION STABILITY (load-bearing for the A2 re-prompt + the W2 nudge): a
    // turn whose `Pending` fact was already appended by an earlier binary is NEVER
    // rebuilt here, so an old binary's base-instruction turn stands even if a newer
    // binary would re-derive a re-prompted/nudged instruction (a different MoteId)
    // for the same `(instance_id, turn)`. The re-prompt/nudge only ever shape a turn
    // THIS pass builds from scratch — keep this dedup check first.
    if rounds.iter().any(|r| r.turn == turn + 1) {
        return ReactChainStatus::Active;
    }
    // FOLD-RE-DERIVED counters (BLOCKER #4): tool_calls = the per-call total across
    // the recorded branches — each `Tool` fact is ONE call, each `Rejected` fact is
    // one spent attempt (PR-3/A2: a model that only ever emits bad calls is bounded by
    // `max_tool_calls` exactly like one that fires real tools), and a `ToolBatch` fact
    // is ALL N of its calls (T-MULTI-ELEMENT-TOOLCALLS: each fired call counts against
    // the budget — fire-all + back-pressure means a batch is bounded by the per-turn
    // cap, and the chain dead-letters loudly once the cumulative total reaches the
    // ceiling). This turn's branch is included (it folded before this call);
    // turns_used = turns 0..=turn ran (a batch is ONE model turn). Then the harness gate.
    let tool_calls = u32::try_from(
        rounds
            .iter()
            .map(|r| match &r.branch {
                ReactBranch::Tool { .. } | ReactBranch::Rejected { .. } => 1,
                ReactBranch::ToolBatch { calls } => calls.len(),
                _ => 0,
            })
            .sum::<usize>(),
    )
    .unwrap_or(u32::MAX);
    let turns_used = turn.saturating_add(1);
    // PR-3 (A2): the just-settled turn (max-seq round at `turn`) — its branch
    // drives both the budget-exhaustion terminal flavor and the next turn's
    // re-prompt/nudge. A pure function of frozen facts.
    let prev_round = rounds
        .iter()
        .filter(|r| r.turn == turn)
        .max_by_key(|r| r.seq);
    // PR-3 (A2): a REJECTED tail carries a durable reason for the re-prompt.
    let prev_reject: Option<(MoteId, String)> = prev_round.and_then(|r| match &r.branch {
        ReactBranch::Rejected { reason } => Some((r.turn_mote_id, reason.clone())),
        _ => None,
    });
    if tool_calls >= anchor.max_tool_calls || turns_used >= anchor.max_turns {
        // BudgetExhausted (the harness ReactStop semantics). The gate is a pure
        // function of frozen facts — it fires identically on every later pass,
        // so the chain is permanently done: skip it (the cache classification).
        // The model spent its whole budget without ever producing an answer →
        // freeze the LOUD terminal `DeadLettered` (BUG-27: terminal, never silent;
        // never a fabricated answer, GR15). Two no-answer tails dead-letter:
        //   - a REJECTED tail (PR-3/A2): every proposal was refused; logs the reason.
        //   - a TOOL tail (W2, this PR): the model kept calling tools and never
        //     settled. Previously this QUIESCED with no terminal — so a run-level
        //     chain exposed neither `answer` nor `dead_lettered`, the client wait
        //     timed out, and `kx agent run` exited 3 (a resumable timeout) for a
        //     PERMANENT failure. Dead-lettering it (the existing tag, NO schema
        //     change) makes the terminal honest (→ exit 1) and aligns the run-level
        //     chain with `finalize_agentic_launch`, which ALREADY dead-letters a
        //     no-answer tail. An `Answer`/`Pending`/already-`DeadLettered` tail is a
        //     no-op. Idempotent: `append_react_branch` dedups `(turn, DeadLettered)`,
        //     and a `DeadLettered`/`Answer` tail matches neither arm on a re-drive.
        let dead_letter_tail: Option<(MoteId, Option<&str>)> = match prev_round.map(|r| &r.branch) {
            Some(ReactBranch::Rejected { .. }) => prev_reject
                .as_ref()
                .map(|(id, reason)| (*id, Some(reason.as_str()))),
            // A `Tool` OR `ToolBatch` tail that exhausted the budget without ever
            // answering dead-letters identically (T-MULTI-ELEMENT-TOOLCALLS).
            Some(ReactBranch::Tool { .. } | ReactBranch::ToolBatch { .. }) => {
                prev_round.map(|r| (r.turn_mote_id, None))
            }
            _ => None,
        };
        if let Some((turn_mote_id, reason)) = dead_letter_tail {
            if let Some(reason) = reason {
                tracing::warn!(
                    turn,
                    %reason,
                    "react chain exhausted its budget on refused tool proposals — dead-lettering"
                );
            } else {
                tracing::warn!(
                    turn,
                    "react chain exhausted its budget calling tools without ever answering — dead-lettering"
                );
            }
            append_react_branch(
                journal,
                projection,
                folded_through,
                anchor,
                turn_mote_id,
                turn,
                ReactBranch::DeadLettered,
            );
        }
        return ReactChainStatus::Settled;
    }
    // Build the next turn from the run-fixed anchor. Any I/O fault fails safe
    // (the chain simply doesn't advance this pass; a later pass retries).
    let Ok(base_bytes) = store.get(&anchor.base_prompt_ref) else {
        return ReactChainStatus::Active;
    };
    let Ok(instruction) = std::str::from_utf8(base_bytes.as_ref()) else {
        // Non-UTF-8 anchor prompt can never become valid — permanently stuck.
        return ReactChainStatus::Settled;
    };
    let Ok(warrant_bytes) = store.get(&anchor.warrant_ref) else {
        return ReactChainStatus::Active;
    };
    let Ok(warrant) = decode_warrant(warrant_bytes.as_ref()) else {
        return ReactChainStatus::Active;
    };
    // W2 (settle-nudge): the turn we are about to build (turn + 1) is the LAST one
    // that can fire a tool before the budget gate above closes — we just passed
    // `tool_calls < max_tool_calls` AND `turns_used < max_turns`, so `+1 >= cap`
    // means "one more tool round exhausts the budget". A `Tool` tail (prev_reject
    // None ⇒ the model has ≥1 real observation) that keeps proposing tools would
    // quiesce answerless → the chain dead-letters (the W2 finding). Nudge the model
    // to settle on a final answer on this last useful turn. A `Rejected` tail
    // already gets the A2 re-prompt (which itself says "answer directly if you
    // cannot"), so the reject arm takes precedence and the nudge requires
    // `prev_reject.is_none()`. PURE over frozen facts (counters fold-re-derived,
    // caps anchor-durable) ⇒ recovery-stable, no new durable state (the A2
    // precedent). `saturating_add` matches the house style (cf. `turns_used` above).
    let nudge = prev_reject.is_none()
        && (tool_calls.saturating_add(1) >= anchor.max_tool_calls
            || turns_used.saturating_add(1) >= anchor.max_turns);
    // PR-3 (A2) / W2: build the next turn's instruction. A REJECTED tail re-prompts
    // with the durable reason so the model self-corrects; a TOOL tail one round from
    // exhaustion gets the settle-nudge. Both are deterministic (pure functions of
    // frozen facts + the anchor's immutable base prompt), so a recovery re-fold
    // re-derives the byte-identical turn Mote (the instruction rides PROMPT_KEY).
    let reprompt;
    let nudged;
    let turn_instruction: &str = match &prev_reject {
        Some((_, reason)) => {
            reprompt = crate::react_shape::render_reprompt(instruction, reason);
            &reprompt
        }
        None if nudge => {
            nudged = crate::react_shape::render_settle_nudge(instruction);
            &nudged
        }
        None => instruction,
    };
    let next_turn = turn + 1;
    let model_id = ModelId(anchor.model_id.clone());
    let next = crate::react_shape::build_chain_turn(
        &model_id,
        turn_instruction,
        next_turn,
        &anchor.instance_id,
        anchor.step_salt,
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
    materialize_react_turn(
        projection,
        dispatch,
        Some(store),
        &next,
        anchor.warrant_ref,
        warrant,
    );
    tracing::info!(turn = next_turn, mote = ?next.id, "react turn materialized");
    ReactChainStatus::Active
}

/// PR-9b-2b: dispose of an AGENTIC chain's LAUNCH mote once the bounded loop has
/// permanently settled — the contrast with the run-level chain (which terminates
/// the run on its final Answer). A terminal `Answer` ⇒ COMMIT the launch mote with
/// the answer turn's `result_ref` + the launch's DECLARED DAG parents (advancing the
/// frozen DAG so its consumers become ready); any other terminal shape (a
/// budget-exhausted `Tool` tail, a `DeadLettered` branch) ⇒ fail-closed dead-letter
/// (the step fails honestly — never fabricate an answer, GR15).
///
/// **Drain-driven + idempotent** (RISK 4): the launch's def (declared parents /
/// `mote_def_hash` / warrant) lives only in the in-memory `dispatch.defs`, lost on a
/// coordinator restart and repopulated by the run's re-submit (the shaper-recovery
/// precedent). When absent (the recovery PROLOGUE, before re-submit) this returns
/// `Active` (dormant) so the FIRST drain after re-submit completes the commit; once
/// the launch is terminal it is a no-op `Settled`.
#[allow(clippy::too_many_arguments)]
fn finalize_agentic_launch<J: Journal>(
    journal: &J,
    store: &LocalFsContentStore,
    projection: &mut Projection,
    folded_through: &mut u64,
    dispatch: &mut Dispatch,
    instance_id: [u8; INSTANCE_ID_LEN],
    step_salt: [u8; 32],
) -> ReactChainStatus {
    let launch_id = MoteId::from_bytes(step_salt);
    // Idempotent: the launch already committed (Answer) or dead-lettered on a prior
    // pass / a recovery re-derive.
    if is_terminal(projection.state_of(&launch_id)) {
        return ReactChainStatus::Settled;
    }
    let Some((launch_mote, launch_warrant)) = dispatch.defs.get(&launch_id).cloned() else {
        // Recovery prologue (def lost, not yet re-submitted): stay dormant.
        return ReactChainStatus::Active;
    };
    // The chain's terminal decision: a frozen `Answer` ⇒ commit; otherwise (budget
    // exhausted without an answer, or a dead-lettered branch) ⇒ dead-letter.
    let answer_turn = projection
        .react_rounds_of(&instance_id, &Some(step_salt))
        .filter(|r| r.branch == ReactBranch::Answer)
        .max_by_key(|r| r.turn)
        .map(|r| r.turn_mote_id);
    let Some(turn_mote_id) = answer_turn else {
        // No frozen Answer — budget exhausted / dead-lettered. Fail-closed.
        dead_letter_agentic_launch(journal, projection, folded_through, dispatch, launch_id);
        return ReactChainStatus::Settled;
    };
    let Some(result_ref) = projection.result_ref_of(&turn_mote_id) else {
        return ReactChainStatus::Active; // committed turn, result_ref not folded yet
    };
    // RISK 3: this commit is a DIRECT append (it bypasses `flush_commits`' admission
    // + phantom-ref guards), so re-verify the answer payload is present in the store
    // before committing — a store fault retries next pass.
    let Ok(raw) = store.get(&result_ref) else {
        return ReactChainStatus::Active;
    };
    // gemma3 connector-tool-fire: under the Ollama non-strict UNION `format` the model
    // settles by emitting `{"answer":"…"}` instead of free prose; unwrap it to the plain
    // text a consumer/CLI expects. `extract_answer` is a byte-identical NO-OP for prose /
    // llama.cpp / non-union answers (`Cow::Borrowed` ⇒ the SAME `result_ref` commits, so
    // those launches stay byte-invariant + recovery-stable). Only a union answer re-`put`s
    // a clean fact — deterministic over the committed turn bytes ⇒ the same ref on a cold
    // re-fold. Presentation only (SN-8); the launch idempotency key stays the launch MoteId
    // (`commit_agentic_launch`), never the ref, so recovery identity is unaffected.
    let served_ref = match kx_toolcall::extract_answer(raw.as_ref()) {
        std::borrow::Cow::Borrowed(_) => result_ref,
        std::borrow::Cow::Owned(unwrapped) => store.put(&unwrapped).unwrap_or(result_ref),
    };
    commit_agentic_launch(
        journal,
        projection,
        folded_through,
        dispatch,
        &launch_mote,
        &launch_warrant,
        served_ref,
    );
    ReactChainStatus::Settled
}

/// PR-9b-2b: COMMIT the launch mote of an agentic step with its loop's final answer.
/// A coordinator-SYNTHESIZED `Committed` (the launch never ran on a worker — the
/// chain did), so the `idempotency_key` is the launch `MoteId` itself
/// (`idempotency.md`: key == derived `MoteId`, the `failed_worker_crashed_entry`
/// precedent). Carries the launch's DECLARED DAG parents so the projection's
/// children-index releases the launch's consumers on the next ready-set pass. Guarded
/// BEFORE the append by the caller's `state_of==terminal` check; the fold's
/// `DuplicateCommitted` is the backstop. Frees the def (a committed mote never re-leases).
fn commit_agentic_launch<J: Journal>(
    journal: &J,
    projection: &mut Projection,
    folded_through: &mut u64,
    dispatch: &mut Dispatch,
    launch_mote: &Mote,
    launch_warrant: &WarrantSpec,
    result_ref: ContentRef,
) {
    let parents: SmallVec<[kx_journal::ParentEntry; 4]> = launch_mote
        .parents
        .iter()
        .map(kx_journal::ParentEntry::from_parent_ref)
        .collect();
    let entry = JournalEntry::Committed {
        mote_id: launch_mote.id,
        idempotency_key: *launch_mote.id.as_bytes(),
        seq: 0,
        nondeterminism: launch_mote.def.nd_class,
        result_ref,
        parents,
        warrant_ref: warrant_ref_of(launch_warrant),
        mote_def_hash: launch_mote.def.hash(),
    };
    match journal.append(entry) {
        Ok(durable) => {
            let seq = durable.seq();
            if seq > *folded_through && projection.fold(&durable).is_ok() {
                *folded_through = seq;
            }
            dispatch.submitted.remove(&launch_mote.id);
            dispatch.defs.remove(&launch_mote.id);
            dispatch.parked_launches.remove(&launch_mote.id);
            dispatch.tracker.resolve_committed(launch_mote.id);
            tracing::info!(launch = ?launch_mote.id, "agentic launch step committed — frozen DAG advanced");
        }
        Err(error) => tracing::error!(%error, "failed to append agentic launch Committed"),
    }
}

/// PR-9b-2b: fail-closed dead-letter an agentic launch whose bounded loop exhausted
/// its budget (or dead-lettered) without a terminal answer — the step FAILS honestly
/// rather than fabricate one (GR15). A terminal coordinator-reported
/// `FailureReason::DeadLettered` (classified terminal by `is_pre_commit_crash` ⇒
/// `terminal_failure_observed` ⇒ `state_of == Failed`, never re-dispatched). The
/// launch leaves the ready-set; its DAG consumers stay `Pending` exactly as for ANY
/// terminally-failed DAG mote (the standing ready-set semantic — a `Failed` parent is
/// not `Committed`). Visible via the alerts inbox + `ListReactTurns`. Idempotent (the
/// caller's `state_of==terminal` guard).
fn dead_letter_agentic_launch<J: Journal>(
    journal: &J,
    projection: &mut Projection,
    folded_through: &mut u64,
    dispatch: &mut Dispatch,
    launch_id: MoteId,
) {
    let entry = JournalEntry::Failed {
        mote_id: launch_id,
        idempotency_key: *launch_id.as_bytes(),
        seq: 0,
        reason_class: FailureReason::DeadLettered,
        reporter_id: COORDINATOR_REPORTER_ID,
    };
    match journal.append(entry) {
        Ok(durable) => {
            let seq = durable.seq();
            if seq > *folded_through && projection.fold(&durable).is_ok() {
                *folded_through = seq;
            }
            dispatch.submitted.remove(&launch_id);
            dispatch.defs.remove(&launch_id);
            dispatch.parked_launches.remove(&launch_id);
            dispatch.tracker.resolve_committed(launch_id);
            tracing::warn!(launch = ?launch_id, "agentic launch step dead-lettered (budget exhausted without an answer)");
        }
        Err(error) => tracing::error!(%error, "failed to append agentic launch Failed"),
    }
}

/// PR-9b-2b: anchor every PARKED agentic launch whose DAG parents have all committed
/// (the launch became "ready" but `lease_ready` parked it). For each: validate the
/// declared budget + prompt (`react_seed_params`, the `0 < tc < turns ≤ 8` gate),
/// build the salt-2 turn-0 (`build_agentic_turn` keyed by `step_salt = launch MoteId`),
/// write the durable turn-0 `ReactRound` anchor, and materialize turn-0 into dispatch
/// so a worker leases it — `settle_react_rounds` then drives the loop. Idempotent:
/// `write_react_anchor` no-ops on an existing turn-0 (a recovery re-submit re-parks).
/// Gated O(1) on the parked set being non-empty (zero cost for a launch-free run).
///
/// **KNOWN LIMITATION (turn-0 reasons over the static prompt only).** turn-0 is built
/// from the launch's `PROMPT_KEY` instruction (via `react_seed_params`) + the chain's
/// own tool observations; the launch's committed DAG-PARENT results are carried into
/// the terminal launch-commit (`commit_agentic_launch`) so the launch's CONSUMERS
/// release, but they are NOT injected into the launch's own model context (turn-0 is
/// edge-free; `resolve_parent_context` serves only the chain trajectory). So an agentic
/// step wired DOWNSTREAM of producers (`producer > plan@tool`) reasons only over its
/// prompt — the headline use is the agentic step as a generator/root, which is correct.
/// Wiring upstream context into the agentic loop is the PR-9d context-carry follow-up
/// (it re-baselines the salt-2 golden + must keep recovery's rebuild byte-identical).
#[allow(clippy::too_many_lines)] // D114: + the require_approval threading.
fn settle_agentic_launches<J: Journal>(
    journal: &J,
    store: Option<&LocalFsContentStore>,
    projection: &mut Projection,
    folded_through: &mut u64,
    dispatch: &mut Dispatch,
) {
    if dispatch.parked_launches.is_empty() {
        return;
    }
    let Some(store) = store else {
        return; // an agentic chain needs a store for its durable anchor
    };
    let parked: Vec<(MoteId, [u8; INSTANCE_ID_LEN])> = dispatch
        .parked_launches
        .iter()
        .map(|(id, instance)| (*id, *instance))
        .collect();
    for (launch_id, instance_id) in parked {
        let Some((launch_mote, launch_warrant)) = dispatch.defs.get(&launch_id).cloned() else {
            dispatch.parked_launches.remove(&launch_id); // def gone — drop the stale park
            continue;
        };
        // Not ready until still-Pending AND every declared DAG parent committed.
        if projection.state_of(&launch_id) != MoteState::Pending {
            dispatch.parked_launches.remove(&launch_id);
            continue;
        }
        // A launch becomes ready when EVERY declared DAG parent commits. Distinguish
        // "still pending" (re-check next drain) from "permanently unsatisfiable" (a
        // parent TERMINALLY failed): a `Failed` parent never cascades the launch to a
        // terminal state (the standing ready-set semantic — a `Failed` parent is not
        // `Committed`), so without this the launch would sit `Pending` forever and its
        // `parked_launches` + `dispatch.defs` entries would never be reclaimed — a slow
        // in-memory leak in a long-lived serve. Fail-closed: dead-letter the launch (it
        // can never run) and reclaim the park, mirroring "a Failed parent strands its
        // consumers" while bounding the in-memory set.
        let mut all_committed = true;
        let mut parent_terminally_failed = false;
        for p in &launch_mote.parents {
            match projection.state_of(&p.parent_id) {
                MoteState::Committed => {}
                MoteState::Failed | MoteState::Inconsistent | MoteState::Repudiated => {
                    parent_terminally_failed = true;
                    all_committed = false;
                    break;
                }
                _ => all_committed = false,
            }
        }
        if parent_terminally_failed {
            tracing::warn!(launch = ?launch_id, "agentic launch parent terminally failed — dead-lettering (can never become ready)");
            dead_letter_agentic_launch(journal, projection, folded_through, dispatch, launch_id);
            dispatch.parked_launches.remove(&launch_id);
            continue;
        }
        if !all_committed {
            continue; // parents still pending — re-check next drain
        }
        let step_salt = *launch_id.as_bytes();
        // Already anchored (recovery re-submit / a prior drain): idempotent — unpark.
        if projection
            .react_rounds_of(&instance_id, &Some(step_salt))
            .any(|r| r.turn == 0)
        {
            dispatch.parked_launches.remove(&launch_id);
            continue;
        }
        // Validate the declared budget + prompt (server-vetted at authoring; a
        // defensive failure here dead-letters rather than wedge).
        let (instruction, (max_turns, max_tool_calls), require_approval) = match react_seed_params(
            &launch_mote,
        ) {
            Ok(decoded) => decoded,
            Err(reason) => {
                tracing::error!(launch = ?launch_id, %reason, "agentic launch budget/prompt invalid — dead-lettering");
                dead_letter_agentic_launch(
                    journal,
                    projection,
                    folded_through,
                    dispatch,
                    launch_id,
                );
                dispatch.parked_launches.remove(&launch_id);
                continue;
            }
        };
        let turn0 = crate::react_shape::build_agentic_turn(
            &launch_mote.def.model_id,
            &instruction,
            0,
            &instance_id,
            &step_salt,
            launch_warrant.model_route.max_output_tokens,
        );
        if let Err(error) = write_react_anchor(
            journal,
            store,
            projection,
            folded_through,
            instance_id,
            Some(step_salt),
            true, // PR-R1: a launched deterministic-agentic step (disposes its launch mote)
            &turn0,
            &launch_warrant,
            max_turns,
            max_tool_calls,
            require_approval,
        ) {
            tracing::error!(launch = ?launch_id, %error, "failed to anchor agentic launch chain");
            continue; // transient — retry next drain (still parked)
        }
        materialize_react_turn(
            projection,
            dispatch,
            Some(store),
            &turn0,
            warrant_ref_of(&launch_warrant),
            launch_warrant.clone(),
        );
        dispatch.parked_launches.remove(&launch_id);
        tracing::info!(launch = ?launch_id, turn0 = ?turn0.id, "agentic launch chain anchored");
    }
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
    cache: &mut ReactSettleCache,
    tool_registry: &dyn ToolRegistry,
) {
    if !projection.has_react_turn() {
        return;
    }
    let Some(store) = store else {
        return;
    };
    // Phase B — re-insert each CHAIN's in-flight turn (its fact is Pending and its
    // Mote is not yet terminal) so a worker can re-lease it. Per-chain reads off the
    // projection's derived nested index (PR-2d-2 / PR-9b-2b): a run may carry the
    // run-level chain (`step_salt = None`) AND agentic-step chains (`Some(..)`); the
    // builder is selected by the chain's `step_salt` (`build_chain_turn`).
    let chains: Vec<ChainKey> = projection.react_chains().collect();
    for (instance_id, step_salt) in chains {
        let Some(latest) = projection
            .latest_react_round(&instance_id, &step_salt)
            .cloned()
        else {
            continue;
        };
        if !matches!(latest.branch, ReactBranch::Pending) {
            continue; // settled tail — Phase C re-drives it (incl. an agentic launch-commit)
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
        let rebuilt = crate::react_shape::build_chain_turn(
            &model_id,
            instruction,
            latest.turn,
            &instance_id,
            step_salt,
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
        materialize_react_turn(
            projection,
            dispatch,
            Some(store),
            &rebuilt,
            latest.warrant_ref,
            warrant,
        );
    }
    // Phase C — complete any interrupted settle/advance (idempotent; dedups on
    // the durable facts; re-decodes the committed tail; PR-2d-2: re-enters the
    // tool round, re-materializing an in-flight OBSERVATION from its frozen
    // `Tool` fact — the deterministic re-derivation IS the durable marker).
    // The cache starts empty at recovery; this first pass re-derives the
    // settled set from the facts.
    settle_react_rounds(
        journal,
        Some(store),
        projection,
        folded_through,
        dispatch,
        cache,
        tool_registry,
    );
}

// ===========================================================================
// RC4c-2b — the live LLM RERANK-turn coordinator drivers.
//
// A faithful, OFF-BUDGET mirror of the ReAct-turn sole-writer lifecycle for a
// SINGLE bounded rerank per retrieval: write a durable `ReRankRound` anchor
// (Pending) BEFORE materializing the edge-free rerank Mote (fact-before-materialize
// crash order), settle it by re-decoding the COMMITTED permutation ON THE SOLE
// WRITER (the `parse_permutation` authority), and recover an in-flight rerank by
// re-inserting the byte-identical Mote from its anchor (R49). The rerank never
// touches a `ReactRound` fact ⇒ it consumes NO `max_turns`/`max_tool_calls`, and its
// distinct `RERANK_TURN_KEY` namespace keeps it off the react settle/recover paths.
// ===========================================================================

/// Write the durable turn-0 `ReRankRound` anchor (`Pending`) for a rerank Mote —
/// fact-BEFORE-materialize, so a crash between anchor and commit re-derives the
/// in-flight Mote from committed facts on recovery. Idempotent: an existing anchor
/// (any outcome) for this `rerank_mote_id` is a no-op (replay / re-drive). The
/// `base_results_ref` / `query_ref` / `warrant_ref` are already content-store refs
/// (the caller staged them); `candidate_count` is the `Permutation(n)` bound.
#[allow(clippy::too_many_arguments)]
fn write_rerank_anchor<J: Journal>(
    journal: &J,
    projection: &mut Projection,
    folded_through: &mut u64,
    rerank_mote: &Mote,
    instance_id: [u8; INSTANCE_ID_LEN],
    base_results_ref: ContentRef,
    query_ref: ContentRef,
    warrant_ref: ContentRef,
    candidate_count: u32,
) -> Result<(), CoordinatorError> {
    if projection.latest_rerank_round(&rerank_mote.id).is_some() {
        return Ok(()); // already anchored (idempotent re-drive / replay)
    }
    let entry = JournalEntry::ReRankRound {
        round: 0,
        rerank_mote_id: rerank_mote.id,
        instance_id,
        base_results_ref,
        query_ref,
        warrant_ref,
        model_id: rerank_mote.def.model_id.0.clone(),
        candidate_count,
        outcome: ReRankOutcome::Pending,
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

/// Append the FROZEN settled outcome for a rerank round (idempotent: a non-`Pending`
/// outcome already recorded for this `rerank_mote_id` is a no-op — a recovery
/// re-drive re-reads the decision, never re-samples). The settle is the SOLE
/// authority: the worker committed the RAW permutation; the coordinator re-decodes.
fn append_rerank_outcome<J: Journal>(
    journal: &J,
    projection: &mut Projection,
    folded_through: &mut u64,
    anchor: &ReRankRoundRecord,
    outcome: ReRankOutcome,
) {
    if projection
        .latest_rerank_round(&anchor.rerank_mote_id)
        .is_some_and(|r| !matches!(r.outcome, ReRankOutcome::Pending))
    {
        return; // already settled (recovery re-drive)
    }
    let entry = JournalEntry::ReRankRound {
        round: anchor.round,
        rerank_mote_id: anchor.rerank_mote_id,
        instance_id: anchor.instance_id,
        base_results_ref: anchor.base_results_ref,
        query_ref: anchor.query_ref,
        warrant_ref: anchor.warrant_ref,
        model_id: anchor.model_id.clone(),
        candidate_count: anchor.candidate_count,
        outcome,
        seq: 0,
    };
    match journal.append(entry) {
        Ok(durable) => {
            let seq = durable.seq();
            if seq > *folded_through && projection.fold(&durable).is_ok() {
                *folded_through = seq;
            }
        }
        Err(error) => tracing::error!(%error, "failed to append ReRankRound outcome fact"),
    }
}

/// The distinct Pending frontier: one `ReRankRoundRecord` per `rerank_mote_id` whose
/// LATEST outcome is still `Pending` (the work not yet settled). Bounded + cheap
/// (reranks are opt-in + off by default; a run writes at most a few).
fn pending_rerank_frontier(projection: &Projection) -> Vec<ReRankRoundRecord> {
    let mut seen: BTreeSet<MoteId> = BTreeSet::new();
    let mut out = Vec::new();
    for r in projection.rerank_rounds() {
        if !seen.insert(r.rerank_mote_id) {
            continue;
        }
        if let Some(latest) = projection.latest_rerank_round(&r.rerank_mote_id) {
            if matches!(latest.outcome, ReRankOutcome::Pending) {
                out.push(latest.clone());
            }
        }
    }
    out
}

/// Settle every Pending rerank round whose Mote reached a TERMINAL state: decode a
/// COMMITTED permutation on the sole writer (`parse_permutation`) → freeze
/// `Reranked`/`FailedClosed`; a FAILED Mote (crash OR permanent) freezes
/// `FailedClosed` (best-effort — a rerank is never worth wedging an answerable RAG
/// turn, unlike a react observation whose permanent failure dead-letters the chain).
/// In-flight Motes stay `Pending` (re-checked next pass; recovery re-inserts them).
/// Zero-cost when no rerank is pending.
fn settle_rerank_rounds<J: Journal>(
    journal: &J,
    store: Option<&LocalFsContentStore>,
    projection: &mut Projection,
    folded_through: &mut u64,
) {
    let pending = pending_rerank_frontier(projection);
    if pending.is_empty() {
        return;
    }
    let Some(store) = store else {
        return;
    };
    for anchor in pending {
        let state = projection.state_of(&anchor.rerank_mote_id);
        let outcome = if state == MoteState::Committed {
            match projection
                .result_ref_of(&anchor.rerank_mote_id)
                .and_then(|r| store.get(&r).ok())
            {
                Some(bytes) => {
                    let text = String::from_utf8_lossy(bytes.as_ref());
                    match kx_toolcall::parse_permutation(&text, anchor.candidate_count as usize) {
                        Some(perm) => ReRankOutcome::Reranked {
                            permutation: perm
                                .into_iter()
                                .map(|i| u32::try_from(i).unwrap_or(u32::MAX))
                                .collect(),
                        },
                        None => ReRankOutcome::FailedClosed, // fail-closed to upstream order
                    }
                }
                None => continue, // transient store fault ⇒ re-check next pass
            }
        } else if is_terminal(state) {
            // A failed rerank Mote (crash flavor OR permanent) ⇒ fail-closed. A rerank
            // is best-effort + off-budget, so it never dead-letters the RAG chain.
            ReRankOutcome::FailedClosed
        } else {
            continue; // Pending / Scheduled / in-flight ⇒ re-check next pass
        };
        // On a successful rerank, PRE-STAGE the reordered delivery blob so the read-path
        // `resolve_parent_context` (which recomputes the SAME deterministic
        // content-addressed ref) finds the content. `base_results_ref` = the retrieve
        // observation itself; a shape/parse fault silently leaves the base order. Pure
        // content-store put (off-DAG, off-digest); idempotent (content-addressed).
        if let ReRankOutcome::Reranked { permutation } = &outcome {
            if let Ok(obs_bytes) = store.get(&anchor.base_results_ref) {
                if let Some(reordered) =
                    reorder_retrieval_observation(obs_bytes.as_ref(), permutation)
                {
                    let _ = store.put(&reordered);
                }
            }
        }
        append_rerank_outcome(journal, projection, folded_through, &anchor, outcome);
    }
}

/// Recover the live rerank chains after a restart: re-insert each in-flight rerank
/// Mote (its anchor is `Pending` and its Mote is not yet terminal) so a worker
/// re-leases it, rebuilt byte-identically from the anchor (R49 — the id is derived
/// from the committed `(instance_id, base_results_ref, query_ref)`; a divergence
/// fail-closes). Then settle any that committed before the crash. Zero-cost when no
/// rerank is pending. Reuses [`materialize_react_turn`] — a generic edge-free
/// register+admit (the rerank Mote is edge-free like a react turn).
fn recover_rerank_chain<J: Journal>(
    journal: &J,
    store: Option<&LocalFsContentStore>,
    projection: &mut Projection,
    folded_through: &mut u64,
    dispatch: &mut Dispatch,
) {
    let Some(store) = store else {
        return;
    };
    let pending = pending_rerank_frontier(projection);
    if pending.is_empty() {
        return;
    }
    for anchor in &pending {
        if is_terminal(projection.state_of(&anchor.rerank_mote_id)) {
            continue; // committed/failed ⇒ settle (below) decodes/freezes it
        }
        let Ok(warrant_bytes) = store.get(&anchor.warrant_ref) else {
            continue;
        };
        let Ok(warrant) = decode_warrant(warrant_bytes.as_ref()) else {
            continue;
        };
        let rebuilt = crate::react_shape::build_rerank_turn(
            &ModelId(anchor.model_id.clone()),
            &anchor.instance_id,
            &anchor.base_results_ref,
            &anchor.query_ref,
            warrant.model_route.max_output_tokens,
        );
        if rebuilt.id != anchor.rerank_mote_id {
            tracing::error!(
                expected = ?anchor.rerank_mote_id,
                rebuilt = ?rebuilt.id,
                "rerank turn rebuild diverged from the durable fact — not re-inserting (fail-closed)"
            );
            continue;
        }
        materialize_react_turn(
            projection,
            dispatch,
            Some(store),
            &rebuilt,
            anchor.warrant_ref,
            warrant,
        );
    }
    // Complete any rerank that committed before the crash (idempotent).
    settle_rerank_rounds(journal, Some(store), projection, folded_through);
}

/// RC4c-2b: the serve-wide LLM-rerank enable flag — `KX_SERVE_RAG_LLM_RERANK` truthy
/// (`1`/`true`/`yes`/`on`, case-insensitive) ⇒ the react-rag / chat-rag paths insert a
/// DURABLE LLM rerank of the retrieved passages before the model reasons. A host-config
/// read OFF the identity/digest path (checked ONLY when a rerank is first anchored; a
/// committed rerank always completes on recovery regardless of the flag). Default-off
/// ⇒ byte-identical to today (the canonical demo writes no rerank). Mirrors
/// [`serve_require_approval_default`].
fn serve_llm_rerank_enabled() -> bool {
    std::env::var("KX_SERVE_RAG_LLM_RERANK")
        .ok()
        .is_some_and(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
}

/// Parse a committed `retrieve@1` observation JSON for the rerank inputs: the query
/// text + the candidate count. The ONE coupling to the retrieve observation's JSON
/// shape (gated strictly on the `retrieve@1` tool id at the call site); generic
/// `serde_json` so it never depends on the gateway's private `Observation` struct.
/// `None` on any parse fault (the caller fails open to a base-order advance).
fn parse_retrieve_query_and_count(obs_bytes: &[u8]) -> Option<(String, usize)> {
    let v: serde_json::Value = serde_json::from_slice(obs_bytes).ok()?;
    let query = v.get("query")?.as_str()?.to_string();
    let n = v.get("passages")?.as_array()?.len();
    Some((query, n))
}

/// Apply a rerank `permutation` to a `retrieve@1` observation's `passages` array,
/// re-serialized deterministically. Produced by `settle_rerank_rounds` (sole writer,
/// stores the bytes) and re-derived byte-identically by the read-path
/// `resolve_parent_context` (content-addressed, so the same ref resolves). `None` on
/// a shape drift / parse fault (the caller falls back to the base order).
fn reorder_retrieval_observation(obs_bytes: &[u8], permutation: &[u32]) -> Option<Vec<u8>> {
    let mut v: serde_json::Value = serde_json::from_slice(obs_bytes).ok()?;
    let passages = v.get("passages")?.as_array()?;
    if passages.len() != permutation.len() {
        return None; // shape drift ⇒ leave the base order
    }
    let reordered: Vec<serde_json::Value> = permutation
        .iter()
        .map(|&i| passages.get(i as usize).cloned())
        .collect::<Option<Vec<_>>>()?;
    v["passages"] = serde_json::Value::Array(reordered);
    serde_json::to_vec(&v).ok()
}

/// The reranked delivery ref for a retrieve observation (read path): if a `Reranked`
/// `ReRankRound` exists for `obs_ref` (= its `base_results_ref`), the content-addressed
/// ref of its reordered passages (already staged by `settle_rerank_rounds`), else
/// `None` (base order). Cheap early-out when the run wrote no rerank.
fn rerank_delivered_ref(
    projection: &Projection,
    store: Option<&LocalFsContentStore>,
    instance_id: &[u8; INSTANCE_ID_LEN],
    obs_ref: ContentRef,
) -> Option<ContentRef> {
    let store = store?;
    // Zero-cost early-out: a run that wrote no rerank fact returns immediately.
    projection.rerank_rounds_of(instance_id).next()?;
    let rr = projection.latest_rerank_round_by_base(instance_id, &obs_ref)?;
    let ReRankOutcome::Reranked { permutation } = &rr.outcome else {
        return None; // Pending / FailedClosed ⇒ base order
    };
    let obs_bytes = store.get(&obs_ref).ok()?;
    let reordered = reorder_retrieval_observation(obs_bytes.as_ref(), permutation)?;
    Some(ContentRef::of(&reordered))
}

/// RC4c-2b react-rag gate: if LLM rerank is enabled and this batch committed a
/// `retrieve@1` observation, ensure its passages are DURABLY reranked before the chain
/// advances. Returns `Some(Active)` while the rerank is in flight (re-check next pass),
/// or `None` when no rerank applies OR it has settled (proceed to advance — a settled
/// `Reranked` reorders the next turn's trajectory via `resolve_parent_context`; a
/// `FailedClosed` leaves the base order). OFF-BUDGET + best-effort: every error path
/// falls through to `None` (advance with the base order — never wedge an answerable
/// RAG turn). Only the FIRST retrieve call in a batch is reranked (react-rag proposes
/// `retrieve` alone; a rare multi-retrieve batch reranks the first, the rest base order).
#[allow(clippy::too_many_arguments)]
fn maybe_gate_on_rerank<J: Journal>(
    journal: &J,
    store: &LocalFsContentStore,
    projection: &mut Projection,
    folded_through: &mut u64,
    dispatch: &mut Dispatch,
    anchor: &ReactRoundRecord,
    turn: u32,
    turn_mote_id: MoteId,
    calls: &[(String, String)],
) -> Option<ReactChainStatus> {
    if !serve_llm_rerank_enabled() {
        return None;
    }
    let call_index = calls
        .iter()
        .position(|(id, ver)| id == "retrieve" && ver == "1")?;
    let obs = crate::react_shape::build_chain_tool(
        &ModelId(anchor.model_id.clone()),
        &ToolName("retrieve".to_string()),
        &ToolVersion("1".to_string()),
        turn,
        &anchor.instance_id,
        anchor.step_salt,
        u32::try_from(call_index).unwrap_or(u32::MAX),
        turn_mote_id,
    );
    let obs_ref = projection.result_ref_of(&obs.id)?; // committed retrieve observation
    let obs_bytes = store.get(&obs_ref).ok()?;
    let (query, n) = parse_retrieve_query_and_count(obs_bytes.as_ref())?;
    if n < 2 {
        return None; // 0/1 passages ⇒ nothing to reorder
    }
    let Ok(query_ref) = store.put(query.as_bytes()) else {
        return None; // store fault ⇒ fail-open to base-order advance
    };
    let warrant = {
        let bytes = store.get(&anchor.warrant_ref).ok()?;
        decode_warrant(bytes.as_ref()).ok()?
    };
    // base_results_ref = the committed retrieve observation itself (the passages the
    // model reorders — no scores, SN-8); the worker + settle read it back.
    let rerank_mote = crate::react_shape::build_rerank_turn(
        &ModelId(anchor.model_id.clone()),
        &anchor.instance_id,
        &obs_ref,
        &query_ref,
        warrant.model_route.max_output_tokens,
    );
    match projection
        .latest_rerank_round(&rerank_mote.id)
        .map(|r| r.outcome.clone())
    {
        None => {
            // First sighting: fact BEFORE materialize (crash-safety order). A fault ⇒
            // fail-open (advance with the base order — the rerank is best-effort).
            if write_rerank_anchor(
                journal,
                projection,
                folded_through,
                &rerank_mote,
                anchor.instance_id,
                obs_ref,
                query_ref,
                anchor.warrant_ref,
                u32::try_from(n).unwrap_or(u32::MAX),
            )
            .is_err()
            {
                return None;
            }
            materialize_react_turn(
                projection,
                dispatch,
                Some(store),
                &rerank_mote,
                anchor.warrant_ref,
                warrant,
            );
            Some(ReactChainStatus::Active) // wait for the rerank to commit + settle
        }
        Some(ReRankOutcome::Pending) => Some(ReactChainStatus::Active), // in flight
        Some(ReRankOutcome::Reranked { .. } | ReRankOutcome::FailedClosed) => None, // settled ⇒ advance
    }
}

// ===========================================================================
// RC4c-2b — the chat-rag / vision-rag SUPPRESSION GATE (durable LLM rerank of a
// grounded single-step answer's context bundle, BEFORE it dispatches).
//
// A chat-rag / vision-rag answer is an EDGE-FREE model step grounded INLINE at bind
// (`config_subset[CONTEXT_ITEMS_KEY]`) — ready-at-submit, no react chain to gate on.
// So the coordinator HOLDS it from lease (mirroring the agentic-launch park) until a
// durable `ReRankRound` over its grounded passages settles, then delivers the
// reranked bundle OUT-OF-BAND (`WorkItem.context_items`, the successor-turn rail) —
// the answer Mote's IDENTITY is untouched (its inline base-order bundle is the
// fallback), so this is fully digest-invariant even for the answer. Reuses the SAME
// `build_rerank_turn` + `settle_rerank_rounds` machinery; the query is the answer's
// `PROMPT`, the candidates its grounded context items.
// ===========================================================================

/// `true` iff `mote` is a rerank-eligible grounded answer — an EDGE-FREE model step
/// (chat-rag / vision-rag) carrying a grounding bundle + a prompt, and NOT a
/// react / rerank / authored-tool / critic / shaper Mote. (The serve rerank flag +
/// the settled-outcome check are applied by the callers.)
fn is_chat_rag_rerank_answer(mote: &Mote) -> bool {
    mote.parents.is_empty()
        && mote.def.critic_check.is_none()
        && !mote.def.is_topology_shaper
        && mote.def.tool_contract.is_empty()
        && mote
            .def
            .config_subset
            .contains_key(&ConfigKey(PROMPT_KEY.to_string()))
        && mote
            .def
            .config_subset
            .contains_key(&ConfigKey(CONTEXT_ITEMS_KEY.to_string()))
        && !mote
            .def
            .config_subset
            .contains_key(&ConfigKey(REACT_TURN_KEY.to_string()))
        && !mote
            .def
            .config_subset
            .contains_key(&ConfigKey(RERANK_TURN_KEY.to_string()))
        && !mote
            .def
            .config_subset
            .contains_key(&ConfigKey(RERANK_CANDIDATES_KEY.to_string()))
}

/// Prepare a chat-rag answer's rerank inputs from its INLINE grounding bundle: build a
/// synthetic `{query, passages:[{ref,text}]}` observation (so the worker + settle reuse
/// the react-rag rerank path VERBATIM) + the query ref. `None` when the bundle has < 2
/// items (nothing to reorder) or a store fault. `query` = the answer's `PROMPT`.
fn chat_rag_rerank_prep(
    mote: &Mote,
    store: &LocalFsContentStore,
) -> Option<(ContentRef, ContentRef, u32)> {
    let bundle = mote
        .def
        .config_subset
        .get(&ConfigKey(CONTEXT_ITEMS_KEY.to_string()))?;
    let items = kx_mote::decode_context_items(&bundle.0);
    if items.len() < 2 {
        return None;
    }
    let query = mote
        .def
        .config_subset
        .get(&ConfigKey(PROMPT_KEY.to_string()))
        .map(|v| String::from_utf8_lossy(&v.0).into_owned())?;
    let passages: Vec<serde_json::Value> = items
        .iter()
        .map(|it| {
            let cref = ContentRef::from_bytes(it.content_ref);
            let text = store
                .get(&cref)
                .ok()
                .map(|b| String::from_utf8_lossy(b.as_ref()).into_owned())
                .unwrap_or_default();
            serde_json::json!({ "ref": cref.to_hex(), "text": text })
        })
        .collect();
    let obs = serde_json::json!({ "query": query, "passages": passages });
    let obs_bytes = serde_json::to_vec(&obs).ok()?;
    let base_results_ref = store.put(&obs_bytes).ok()?;
    let query_ref = store.put(query.as_bytes()).ok()?;
    Some((
        base_results_ref,
        query_ref,
        u32::try_from(items.len()).unwrap_or(u32::MAX),
    ))
}

/// Apply a rerank `permutation` to a grounding `CONTEXT_ITEMS` bundle — decode →
/// reorder the items → re-encode. Produced by `settle_chat_rag_reranks` (stores the
/// bytes) + re-derived byte-identically on the read path (content-addressed). `None`
/// on a length mismatch (the caller falls back to the base order).
fn reorder_context_items_bundle(bundle_bytes: &[u8], permutation: &[u32]) -> Option<Vec<u8>> {
    let items = kx_mote::decode_context_items(bundle_bytes);
    if items.len() != permutation.len() {
        return None;
    }
    let reordered: Vec<kx_mote::ContextItemRef> = permutation
        .iter()
        .map(|&i| items.get(i as usize).cloned())
        .collect::<Option<Vec<_>>>()?;
    // ORDER-PRESERVING encode: `encode_context_items` canonically SORTS (which would
    // discard the rerank); the reranked bundle is off-digest out-of-band delivery, so
    // it uses the ordered variant, and the serve assembler renders in decoded order.
    Some(kx_mote::encode_context_items_ordered(&reordered))
}

/// The rerank `MoteId` for a chat-rag answer (its salt = the run instance ‖ its
/// synthetic base-results ref ‖ its query ref). `None` when not eligible / no run
/// registered / < 2 items. Shared by the lease hold, the delivery, and the settle pass
/// so all three agree by construction (the `max_output_tokens` MUST be the answer's
/// warrant cap in every caller).
fn chat_rag_rerank_id(
    mote: &Mote,
    warrant: &WarrantSpec,
    projection: &Projection,
    store: &LocalFsContentStore,
) -> Option<(MoteId, ContentRef, ContentRef, u32, [u8; INSTANCE_ID_LEN])> {
    let (instance_id, _) = projection.run_registration()?;
    let (base_ref, query_ref, n) = chat_rag_rerank_prep(mote, store)?;
    let id = crate::react_shape::build_rerank_turn(
        &mote.def.model_id,
        &instance_id,
        &base_ref,
        &query_ref,
        warrant.model_route.max_output_tokens,
    )
    .id;
    Some((id, base_ref, query_ref, n, instance_id))
}

/// Whether to HOLD a chat-rag answer from lease this poll: eligible + serve rerank
/// enabled + its `ReRankRound` has not reached a terminal outcome. Holds even BEFORE
/// the rerank is anchored (`None`), so a lease that races ahead of the settle pass
/// never dispatches the answer on the base order.
fn chat_rag_rerank_holds(
    mote: &Mote,
    warrant: &WarrantSpec,
    projection: &Projection,
    store: Option<&LocalFsContentStore>,
) -> bool {
    if !serve_llm_rerank_enabled() || !is_chat_rag_rerank_answer(mote) {
        return false;
    }
    let Some(store) = store else {
        return false;
    };
    let Some((id, ..)) = chat_rag_rerank_id(mote, warrant, projection, store) else {
        return false; // < 2 items / no run ⇒ nothing to hold for
    };
    !matches!(
        projection.latest_rerank_round(&id).map(|r| &r.outcome),
        Some(ReRankOutcome::Reranked { .. } | ReRankOutcome::FailedClosed)
    )
}

/// The reranked `CONTEXT_ITEMS` bundle ref to deliver OUT-OF-BAND for a chat-rag answer
/// (read path): `Some(reordered_ref)` when its rerank settled `Reranked` (the bytes
/// were staged by `settle_chat_rag_reranks`; the ref is recomputed deterministically),
/// else `None` (deliver nothing ⇒ the worker uses the INLINE base-order bundle —
/// `FailedClosed` or not-eligible).
fn chat_rag_delivered_context(
    mote: &Mote,
    warrant: &WarrantSpec,
    projection: &Projection,
    store: Option<&LocalFsContentStore>,
) -> Option<ContentRef> {
    if !serve_llm_rerank_enabled() || !is_chat_rag_rerank_answer(mote) {
        return None;
    }
    let store = store?;
    let (id, ..) = chat_rag_rerank_id(mote, warrant, projection, store)?;
    let rr = projection.latest_rerank_round(&id)?;
    let ReRankOutcome::Reranked { permutation } = &rr.outcome else {
        return None;
    };
    let bundle = mote
        .def
        .config_subset
        .get(&ConfigKey(CONTEXT_ITEMS_KEY.to_string()))?;
    let reordered = reorder_context_items_bundle(&bundle.0, permutation)?;
    Some(ContentRef::of(&reordered))
}

/// Settle pass: for every ADMITTED (not-yet-committed) chat-rag answer whose rerank is
/// enabled, ANCHOR + materialize its rerank Mote (first sighting) and STAGE the reordered
/// bundle once it settles `Reranked`. Zero-cost when the flag is off / no eligible answer
/// is admitted. Idempotent (anchor + stage dedup on the durable fact).
fn settle_chat_rag_reranks<J: Journal>(
    journal: &J,
    store: Option<&LocalFsContentStore>,
    projection: &mut Projection,
    folded_through: &mut u64,
    dispatch: &mut Dispatch,
) {
    if !serve_llm_rerank_enabled() {
        return;
    }
    let Some(store) = store else {
        return;
    };
    if projection.run_registration().is_none() {
        return;
    }
    // Snapshot the eligible answers (clone out to avoid borrowing `dispatch` while we
    // mutate `projection`/`dispatch` below).
    let answers: Vec<(Mote, WarrantSpec)> = dispatch
        .defs
        .values()
        .filter(|(m, _)| is_chat_rag_rerank_answer(m) && !is_terminal(projection.state_of(&m.id)))
        .cloned()
        .collect();
    for (answer, warrant) in answers {
        let Some((id, base_ref, query_ref, n, instance_id)) =
            chat_rag_rerank_id(&answer, &warrant, projection, store)
        else {
            continue;
        };
        match projection
            .latest_rerank_round(&id)
            .map(|r| r.outcome.clone())
        {
            None => {
                // First sighting: encode the answer's warrant + anchor (fact BEFORE
                // materialize) + admit the rerank Mote. Fail-open on a fault (the answer
                // stays held; a later pass retries — or the hold's own eligibility lapses).
                let Ok(warrant_ref) = store.put(&encode_warrant(&warrant)) else {
                    continue;
                };
                let rerank_mote = crate::react_shape::build_rerank_turn(
                    &answer.def.model_id,
                    &instance_id,
                    &base_ref,
                    &query_ref,
                    warrant.model_route.max_output_tokens,
                );
                if write_rerank_anchor(
                    journal,
                    projection,
                    folded_through,
                    &rerank_mote,
                    instance_id,
                    base_ref,
                    query_ref,
                    warrant_ref,
                    n,
                )
                .is_ok()
                {
                    materialize_react_turn(
                        projection,
                        dispatch,
                        Some(store),
                        &rerank_mote,
                        warrant_ref,
                        warrant,
                    );
                }
            }
            Some(ReRankOutcome::Reranked { permutation }) => {
                // Stage the reordered bundle so the lease read path's deterministic ref
                // resolves. Idempotent (content-addressed put).
                if let Some(bundle) = answer
                    .def
                    .config_subset
                    .get(&ConfigKey(CONTEXT_ITEMS_KEY.to_string()))
                {
                    if let Some(reordered) = reorder_context_items_bundle(&bundle.0, &permutation) {
                        let _ = store.put(&reordered);
                    }
                }
            }
            Some(ReRankOutcome::Pending | ReRankOutcome::FailedClosed) => {}
        }
    }
}

#[cfg(test)]
mod rerank_delivery_tests {
    use super::*;

    #[test]
    fn parse_and_reorder_retrieval_observation() {
        let obs = br#"{"dataset":"kb","query":"how does recovery work?","passages":[{"ref":"a","text":"zero"},{"ref":"b","text":"one"},{"ref":"c","text":"two"}]}"#;
        let (q, n) = parse_retrieve_query_and_count(obs).expect("parses");
        assert_eq!(q, "how does recovery work?");
        assert_eq!(n, 3);
        // Reorder by [2,0,1] ⇒ passages become [two, zero, one]; query + dataset preserved.
        let reordered = reorder_retrieval_observation(obs, &[2, 0, 1]).expect("reorders");
        let v: serde_json::Value = serde_json::from_slice(&reordered).unwrap();
        let texts: Vec<&str> = v["passages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["text"].as_str().unwrap())
            .collect();
        assert_eq!(texts, ["two", "zero", "one"]);
        assert_eq!(v["query"], "how does recovery work?");
        assert_eq!(v["dataset"], "kb");
        // Identity permutation is order-preserving.
        let same = reorder_retrieval_observation(obs, &[0, 1, 2]).expect("identity");
        let v2: serde_json::Value = serde_json::from_slice(&same).unwrap();
        let t2: Vec<&str> = v2["passages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["text"].as_str().unwrap())
            .collect();
        assert_eq!(t2, ["zero", "one", "two"]);
    }

    #[test]
    fn reorder_retrieval_observation_fails_closed_on_shape_drift() {
        let obs = br#"{"query":"q","passages":[{"text":"a"},{"text":"b"}]}"#;
        // A permutation whose length differs from the passage count ⇒ None (base order).
        assert!(reorder_retrieval_observation(obs, &[0]).is_none());
        assert!(reorder_retrieval_observation(obs, &[0, 1, 2]).is_none());
        // Non-JSON / missing fields ⇒ None (never panics on a malformed observation).
        assert!(reorder_retrieval_observation(b"not json", &[0, 1]).is_none());
        assert!(parse_retrieve_query_and_count(b"not json").is_none());
        assert!(parse_retrieve_query_and_count(br#"{"passages":[]}"#).is_none());
        // no query
    }

    #[test]
    fn reorder_context_items_bundle_permutes_and_fails_closed() {
        use kx_mote::{decode_context_items, encode_context_items, ContextItemRef};
        let items = vec![
            ContextItemRef {
                name: "a".into(),
                content_ref: [1; 32],
            },
            ContextItemRef {
                name: "b".into(),
                content_ref: [2; 32],
            },
            ContextItemRef {
                name: "c".into(),
                content_ref: [3; 32],
            },
        ];
        let bundle = encode_context_items(&items);
        // Reorder [2,0,1] ⇒ c, a, b.
        let reordered = reorder_context_items_bundle(&bundle, &[2, 0, 1]).expect("reorders");
        let out = decode_context_items(&reordered);
        assert_eq!(
            out.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(),
            ["c", "a", "b"]
        );
        // A length mismatch fails closed to the base order.
        assert!(reorder_context_items_bundle(&bundle, &[0, 1]).is_none());
    }
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
    store: Option<&LocalFsContentStore>,
    mote: Mote,
    warrant: WarrantSpec,
) -> SubmitOutcome {
    let mote_id = mote.id;
    // Persist on BOTH the fresh and the duplicate arm: the put is idempotent
    // (same def ⇒ same bytes ⇒ same address) and the duplicate path SELF-HEALS
    // a def first admitted by a pre-Batch-B binary (whose blob never existed).
    persist_def(store, &mote.def);
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

#[cfg(test)]
mod authored_tool_tests {
    //! PR-6b-2: the fail-closed `resolve_authored_tool_args` gate — the
    //! coordinator's args-from-`config_subset` resolver for a standalone authored
    //! `tool()` node. (Full lease-path disjointness incl. `is_authored_tool`'s
    //! Projection dependence is covered by the live integration test
    //! `tests/tool_node_live.rs`.)
    use super::*;
    use kx_mote::{
        ConfigVal, GraphPosition, InferenceParams, InputDataId, LogicRef, PromptTemplateHash,
        MOTE_DEF_SCHEMA_VERSION,
    };
    use kx_tool_registry::{
        IdempotencyClass, InMemoryToolRegistry, InputSchema, ParamSpec, ParamType, ToolDef,
        ToolKind, ToolProvenance,
    };
    use kx_warrant::{FsScope, NetScope, ResourceCeiling, ToolRequirement};

    fn tool_mote(contract: &[(&str, &str)], args: Option<&[u8]>) -> Mote {
        let mut tool_contract = BTreeMap::new();
        for (n, v) in contract {
            tool_contract.insert(ToolName((*n).into()), ToolVersion((*v).into()));
        }
        let mut config_subset = BTreeMap::new();
        if let Some(a) = args {
            config_subset.insert(ConfigKey(TOOL_ARGS_KEY.to_string()), ConfigVal(a.to_vec()));
        }
        let def = MoteDef {
            logic_ref: LogicRef::from_bytes([7u8; 32]),
            model_id: ModelId("test".into()),
            prompt_template_hash: PromptTemplateHash::from_bytes([7u8; 32]),
            tool_contract,
            nd_class: NdClass::WorldMutating,
            config_subset,
            effect_pattern: EffectPattern::StageThenCommit,
            critic_for: None,
            is_topology_shaper: false,
            inference_params: InferenceParams::default(),
            critic_check: None,
            schema_version: MOTE_DEF_SCHEMA_VERSION,
        };
        Mote::new(
            def,
            InputDataId::from_bytes([7u8; 32]),
            GraphPosition(vec![1]),
            SmallVec::new(),
        )
    }

    fn registry_with(schema: Option<InputSchema>) -> InMemoryToolRegistry {
        let mut reg = InMemoryToolRegistry::new();
        let def = ToolDef {
            tool_id: ToolName("web-search".into()),
            tool_version: ToolVersion("1".into()),
            kind: ToolKind::Builtin,
            required_capability: ToolRequirement {
                net_scope_required: NetScope::None,
                fs_scope_required: FsScope::empty(),
                syscall_profile_ref: kx_content::ContentRef::from_bytes([0; 32]),
                min_resource_ceiling: ResourceCeiling {
                    cpu_milli: 0,
                    mem_bytes: 0,
                    wall_clock_ms: 0,
                    fd_count: 0,
                    disk_bytes: 0,
                },
            },
            description: String::new(),
            idempotency_class: IdempotencyClass::Staged,
            input_schema: schema,
        };
        let _ = reg.register(def, ToolProvenance::HumanAuthored { author: "t".into() });
        reg
    }

    fn schema() -> InputSchema {
        InputSchema {
            params: vec![ParamSpec {
                name: "q".into(),
                ty: ParamType::Str { max_len: 64 },
                required: true,
            }],
            deny_unknown: true,
        }
    }

    #[test]
    fn happy_path_returns_validated_args() {
        let reg = registry_with(Some(schema()));
        let mote = tool_mote(&[("web-search", "1")], Some(br#"{"q":"hi"}"#));
        let got = resolve_authored_tool_args(&mote, &reg)
            .into_resolved()
            .expect("valid args resolve");
        assert_eq!(got.0, br#"{"q":"hi"}"#.to_vec());
        assert_eq!(got.1, NetScope::None);
    }

    #[test]
    fn absent_tool_is_fail_closed() {
        let reg = InMemoryToolRegistry::new(); // empty — tool not registered
        let mote = tool_mote(&[("web-search", "1")], Some(br#"{"q":"hi"}"#));
        // PR-9a: an absent tool is a PERMANENT fault (deregister-after-authoring),
        // not a transient skip — the loud terminal, never a silent wedge.
        assert!(resolve_authored_tool_args(&mote, &reg).is_permanent());
    }

    #[test]
    fn multi_tool_contract_is_refused() {
        let reg = registry_with(Some(schema()));
        let mote = tool_mote(
            &[("web-search", "1"), ("other", "1")],
            Some(br#"{"q":"hi"}"#),
        );
        assert!(resolve_authored_tool_args(&mote, &reg).is_permanent());
    }

    #[test]
    fn missing_config_args_is_refused() {
        let reg = registry_with(Some(schema()));
        let mote = tool_mote(&[("web-search", "1")], None);
        assert!(resolve_authored_tool_args(&mote, &reg).is_permanent());
    }

    #[test]
    fn schema_reject_is_fail_closed() {
        let reg = registry_with(Some(schema()));
        // `q` required but absent, and a smuggled key under deny_unknown.
        let mote = tool_mote(&[("web-search", "1")], Some(br#"{"smuggled":1}"#));
        // PR-9a: a schema reject is PERMANENT — at the coordinator it is now a
        // dead-letter signal (it is also refused earlier, at authoring).
        assert!(resolve_authored_tool_args(&mote, &reg).is_permanent());
    }

    #[test]
    fn no_schema_tool_passes_args_through() {
        let reg = registry_with(None); // no input_schema ⇒ no client-side gate
        let mote = tool_mote(&[("web-search", "1")], Some(br#"{"anything":true}"#));
        let got = resolve_authored_tool_args(&mote, &reg)
            .into_resolved()
            .expect("passes through");
        assert_eq!(got.0, br#"{"anything":true}"#.to_vec());
    }
}

#[cfg(test)]
mod agentic_launch_disjointness_tests {
    //! PR-9b-2b: `is_agentic_launch` is provably DISJOINT from every other
    //! tool-contract Mote shape, so the lease-time PARK (and the parked-set
    //! population) can never mis-fire on an ordinary mote. The four shapes
    //! partition by `(nd_class, config keys, parent shape)`:
    //! - agentic launch — ROND + StageThenCommit + non-empty contract + `PROMPT_KEY`
    //!   + NO `TOOL_ARGS_KEY` (a generator that emits its own tool calls);
    //! - react turn — EMPTY contract;
    //! - react observation — non-empty contract + EMPTY config (no `PROMPT_KEY`) +
    //!   a react-turn Data parent;
    //! - authored tool — non-empty contract + `TOOL_ARGS_KEY` + WorldMutating.
    use super::*;
    use kx_mote::{
        ConfigVal, EdgeMeta, GraphPosition, InferenceParams, InputDataId, LogicRef, ParentRef,
        PromptTemplateHash, MOTE_DEF_SCHEMA_VERSION,
    };

    const INSTANCE: [u8; INSTANCE_ID_LEN] = [9u8; INSTANCE_ID_LEN];
    const STEP_SALT: [u8; 32] = [3u8; 32];

    fn served() -> ModelId {
        ModelId("served".into())
    }

    /// The B3-binder shape: a generator MODEL step carrying an author-declared
    /// tool-grant set + the model directive, with a declared DAG parent.
    fn launch_mote(config: BTreeMap<ConfigKey, ConfigVal>, nd: NdClass) -> Mote {
        let mut tool_contract = BTreeMap::new();
        tool_contract.insert(ToolName("mcp-echo".into()), ToolVersion("1".into()));
        let def = MoteDef {
            logic_ref: LogicRef::from_bytes([1u8; 32]),
            model_id: served(),
            prompt_template_hash: PromptTemplateHash::from_bytes([1u8; 32]),
            tool_contract,
            nd_class: nd,
            config_subset: config,
            effect_pattern: EffectPattern::StageThenCommit,
            critic_for: None,
            is_topology_shaper: false,
            inference_params: InferenceParams::default(),
            critic_check: None,
            schema_version: MOTE_DEF_SCHEMA_VERSION,
        };
        Mote::new(
            def,
            InputDataId::from_bytes([1u8; 32]),
            GraphPosition(vec![2]),
            std::iter::once(ParentRef {
                parent_id: MoteId::from_bytes([8u8; 32]),
                edge: EdgeMeta::data(),
            })
            .collect(),
        )
    }

    fn prompt_only() -> BTreeMap<ConfigKey, ConfigVal> {
        let mut c = BTreeMap::new();
        c.insert(
            ConfigKey(PROMPT_KEY.to_string()),
            ConfigVal(b"do it".to_vec()),
        );
        c
    }

    #[test]
    fn launch_shape_is_an_agentic_launch() {
        assert!(is_agentic_launch(&launch_mote(
            prompt_only(),
            NdClass::ReadOnlyNondet
        )));
    }

    #[test]
    fn launch_is_neither_authored_tool_nor_observation() {
        let p = kx_projection::Projection::new();
        let launch = launch_mote(prompt_only(), NdClass::ReadOnlyNondet);
        assert!(
            !is_authored_tool(&launch, &p),
            "no TOOL_ARGS_KEY ⇒ not authored tool"
        );
        assert!(
            !is_react_observation(&launch, &p),
            "a DAG parent (not a folded react turn) ⇒ not an observation"
        );
    }

    #[test]
    fn react_turns_are_not_launches() {
        // build_react_turn / build_agentic_turn ⇒ EMPTY tool_contract.
        let run = crate::react_shape::build_react_turn(&served(), "go", 0, &INSTANCE, 256);
        let agentic =
            crate::react_shape::build_agentic_turn(&served(), "go", 0, &INSTANCE, &STEP_SALT, 256);
        assert!(!is_agentic_launch(&run));
        assert!(!is_agentic_launch(&agentic));
    }

    #[test]
    fn react_observations_are_not_launches() {
        // build_react_tool / build_agentic_tool ⇒ EMPTY config (no PROMPT_KEY).
        let parent = MoteId::from_bytes([5u8; 32]);
        let run = crate::react_shape::build_react_tool(
            &served(),
            &ToolName("mcp-echo".into()),
            &ToolVersion("1".into()),
            0,
            &INSTANCE,
            0,
            parent,
        );
        let agentic = crate::react_shape::build_agentic_tool(
            &served(),
            &ToolName("mcp-echo".into()),
            &ToolVersion("1".into()),
            0,
            &INSTANCE,
            &STEP_SALT,
            0,
            parent,
        );
        assert!(!is_agentic_launch(&run));
        assert!(!is_agentic_launch(&agentic));
    }

    #[test]
    fn world_mutating_with_tool_args_is_authored_tool_not_launch() {
        // The authored-tool shape: WorldMutating + TOOL_ARGS_KEY ⇒ NOT a launch
        // (fails both the ReadOnlyNondet AND the no-TOOL_ARGS_KEY clauses).
        let mut config = prompt_only();
        config.insert(
            ConfigKey(TOOL_ARGS_KEY.to_string()),
            ConfigVal(b"{}".to_vec()),
        );
        assert!(!is_agentic_launch(&launch_mote(
            config,
            NdClass::WorldMutating
        )));
    }

    #[test]
    fn launch_needs_prompt_and_no_tool_args() {
        // No PROMPT_KEY ⇒ not a runnable launch (cannot reason).
        assert!(!is_agentic_launch(&launch_mote(
            BTreeMap::new(),
            NdClass::ReadOnlyNondet
        )));
        // PROMPT_KEY + TOOL_ARGS_KEY ⇒ the authored-tool discriminant wins, not a launch.
        let mut config = prompt_only();
        config.insert(
            ConfigKey(TOOL_ARGS_KEY.to_string()),
            ConfigVal(b"{}".to_vec()),
        );
        assert!(!is_agentic_launch(&launch_mote(
            config,
            NdClass::ReadOnlyNondet
        )));
    }
}

#[cfg(test)]
mod context_carry_tests {
    //! PR-9d: the per-turn upstream context-carry resolver — `decode_react_marker`
    //! (the shared both-lengths chain-key decoder) + `resolve_react_context_items`
    //! (the edge-free turn-0-anchor lookup that delivers a SUCCESSOR ReAct turn its
    //! grounding context, returning `None` for a Mote that already carries its bundle
    //! inline so it is never double-prepended).
    use super::*;
    use kx_journal::{InMemoryJournal, Journal};
    use kx_mote::{
        ConfigVal, GraphPosition, InferenceParams, InputDataId, LogicRef, PromptTemplateHash,
        MOTE_DEF_SCHEMA_VERSION,
    };

    const INSTANCE: [u8; INSTANCE_ID_LEN] = [7u8; INSTANCE_ID_LEN];
    const SALT: [u8; 32] = [9u8; 32];
    const CTX_REF: ContentRef = ContentRef([0x44; 32]);
    const IMG_REF: ContentRef = ContentRef([0x55; 32]);

    #[test]
    fn decode_react_marker_handles_both_lengths_and_rejects_others() {
        // 16 bytes ⇒ a LEGACY run-level chain (no salt).
        assert_eq!(decode_react_marker(&[7u8; 16]), Some(([7u8; 16], None)));
        // 48 bytes ⇒ `instance_id ‖ step_salt` (an agentic / per-invocation chain).
        let mut m = INSTANCE.to_vec();
        m.extend_from_slice(&SALT);
        assert_eq!(decode_react_marker(&m), Some((INSTANCE, Some(SALT))));
        // Any other length is a malformed marker ⇒ fail-closed.
        assert_eq!(decode_react_marker(&[0u8; 10]), None);
        assert_eq!(decode_react_marker(&[]), None);
    }

    /// A ReAct-turn-shaped Mote (edge-free, ROND) carrying the given `config_subset`.
    /// The resolver reads only `config_subset`, so the other def fields are arbitrary.
    fn turn_mote(config: BTreeMap<ConfigKey, ConfigVal>) -> Mote {
        let def = MoteDef {
            logic_ref: LogicRef::from_bytes([1u8; 32]),
            model_id: ModelId("served".into()),
            prompt_template_hash: PromptTemplateHash::from_bytes([1u8; 32]),
            tool_contract: BTreeMap::new(),
            nd_class: NdClass::ReadOnlyNondet,
            config_subset: config,
            effect_pattern: EffectPattern::StageThenCommit,
            critic_for: None,
            is_topology_shaper: false,
            inference_params: InferenceParams::default(),
            critic_check: None,
            schema_version: MOTE_DEF_SCHEMA_VERSION,
        };
        Mote::new(
            def,
            InputDataId::from_bytes([1u8; 32]),
            GraphPosition(vec![1]),
            std::iter::empty().collect(),
        )
    }

    fn marker(instance: [u8; INSTANCE_ID_LEN], salt: [u8; 32]) -> ConfigVal {
        let mut m = instance.to_vec();
        m.extend_from_slice(&salt);
        ConfigVal(m)
    }

    // Covers BOTH edge-free resolvers (context-items + AGENTIC-VISION image) over the same
    // folded turn-0 anchor — one fixture, the two parallel slot ladders.
    #[test]
    #[allow(clippy::too_many_lines)]
    fn resolves_anchor_context_and_image_for_a_successor_turn_but_not_an_inline_mote() {
        // Fold a turn-0 anchor carrying a context-bundle ref into a fresh projection.
        let mut projection = Projection::new();
        let journal = InMemoryJournal::new();
        let anchor = JournalEntry::ReactRound {
            turn: 0,
            turn_mote_id: MoteId::from_bytes([2u8; 32]),
            instance_id: INSTANCE,
            base_prompt_ref: ContentRef::from_bytes([1u8; 32]),
            warrant_ref: ContentRef::from_bytes([2u8; 32]),
            model_id: "served".into(),
            branch: ReactBranch::Pending,
            max_turns: 8,
            max_tool_calls: 6,
            step_salt: Some(SALT),
            is_agentic_launch: false,
            context_items_ref: Some(CTX_REF),
            image_ref: Some(IMG_REF),
            require_approval: false,
            seq: 0,
        };
        let durable = journal.append(anchor).unwrap();
        projection.fold(&durable).unwrap();

        // A SUCCESSOR turn (chain marker, NO inline bundle) gets the anchor's ref.
        let mut successor = BTreeMap::new();
        successor.insert(
            ConfigKey(REACT_TURN_KEY.to_string()),
            marker(INSTANCE, SALT),
        );
        successor.insert(
            ConfigKey(PROMPT_KEY.to_string()),
            ConfigVal(b"turn 1".to_vec()),
        );
        assert_eq!(
            resolve_react_context_items(&turn_mote(successor), &projection),
            Some(CTX_REF),
            "a successor turn reasons over the chain's turn-0 grounding context",
        );

        // A Mote carrying its bundle INLINE (turn 0 / a leaf) ⇒ `None` (the
        // config_subset path serves it; delivering it again would double-prepend).
        let mut inline = BTreeMap::new();
        inline.insert(
            ConfigKey(REACT_TURN_KEY.to_string()),
            marker(INSTANCE, SALT),
        );
        inline.insert(
            ConfigKey(CONTEXT_ITEMS_KEY.to_string()),
            ConfigVal(vec![1, 2, 3]),
        );
        assert_eq!(
            resolve_react_context_items(&turn_mote(inline), &projection),
            None,
            "an inline-bundle Mote is never double-prepended",
        );

        // A non-ReAct Mote (no marker) ⇒ `None`.
        assert_eq!(
            resolve_react_context_items(&turn_mote(BTreeMap::new()), &projection),
            None,
        );

        // A successor turn of a DIFFERENT chain (no matching anchor) ⇒ `None`
        // (fail-closed — never another run's context).
        let mut other = BTreeMap::new();
        other.insert(
            ConfigKey(REACT_TURN_KEY.to_string()),
            marker([8u8; INSTANCE_ID_LEN], SALT),
        );
        assert_eq!(
            resolve_react_context_items(&turn_mote(other), &projection),
            None
        );

        // AGENTIC-VISION: the SAME edge-free resolution for the run's grounding image.
        // A successor turn (chain marker, NO inline image) gets the anchor's image_ref.
        let mut img_successor = BTreeMap::new();
        img_successor.insert(
            ConfigKey(REACT_TURN_KEY.to_string()),
            marker(INSTANCE, SALT),
        );
        img_successor.insert(
            ConfigKey(PROMPT_KEY.to_string()),
            ConfigVal(b"turn 1".to_vec()),
        );
        assert_eq!(
            resolve_react_image_ref(&turn_mote(img_successor), &projection),
            Some(IMG_REF),
            "a successor turn reasons over the chain's turn-0 grounding image",
        );
        // A Mote carrying its image INLINE (turn 0) ⇒ `None` (config_subset serves it;
        // delivering it again out-of-band would double-attach the image).
        let mut img_inline = BTreeMap::new();
        img_inline.insert(
            ConfigKey(REACT_TURN_KEY.to_string()),
            marker(INSTANCE, SALT),
        );
        img_inline.insert(
            ConfigKey(IMAGE_REF_KEY.to_string()),
            ConfigVal(b"\"deadbeef\"".to_vec()),
        );
        assert_eq!(
            resolve_react_image_ref(&turn_mote(img_inline), &projection),
            None,
            "an inline-image Mote is never double-attached",
        );
        // A non-ReAct Mote (no marker) ⇒ `None`.
        assert_eq!(
            resolve_react_image_ref(&turn_mote(BTreeMap::new()), &projection),
            None,
        );
        // A successor turn of a DIFFERENT chain (no matching anchor) ⇒ `None`
        // (fail-closed — never another run's image).
        let mut img_other = BTreeMap::new();
        img_other.insert(
            ConfigKey(REACT_TURN_KEY.to_string()),
            marker([8u8; INSTANCE_ID_LEN], SALT),
        );
        assert_eq!(
            resolve_react_image_ref(&turn_mote(img_other), &projection),
            None
        );
    }

    // ---------------------------------------------------------------------------
    // D114 (HITL approval gate) + M11/D115 (cost-spend) — deterministic engine tests
    // ---------------------------------------------------------------------------

    /// Fold a turn-0 anchor (`require_approval` posture) into a fresh projection +
    /// journal, returning the record the gate reads.
    fn fold_gate_anchor(
        journal: &InMemoryJournal,
        projection: &mut Projection,
        require_approval: bool,
    ) -> ReactRoundRecord {
        let anchor = JournalEntry::ReactRound {
            turn: 0,
            turn_mote_id: MoteId::from_bytes([2u8; 32]),
            instance_id: INSTANCE,
            base_prompt_ref: ContentRef::from_bytes([1u8; 32]),
            warrant_ref: ContentRef::from_bytes([2u8; 32]),
            model_id: "served".into(),
            branch: ReactBranch::Pending,
            max_turns: 8,
            max_tool_calls: 20,
            step_salt: Some(SALT),
            is_agentic_launch: false,
            context_items_ref: None,
            image_ref: None,
            require_approval,
            seq: 0,
        };
        let durable = journal.append(anchor).unwrap();
        projection.fold(&durable).unwrap();
        projection
            .latest_react_round(&INSTANCE, &Some(SALT))
            .cloned()
            .expect("anchor folded")
    }

    #[test]
    fn tool_needs_approval_gates_only_irreversible_world_mutating_classes() {
        assert!(tool_needs_approval(Some(IdempotencyClass::Staged)));
        assert!(tool_needs_approval(Some(IdempotencyClass::AtLeastOnce)));
        // Self-closing (Token/Readback) + read-only/unknown auto-proceed.
        assert!(!tool_needs_approval(Some(IdempotencyClass::Token)));
        assert!(!tool_needs_approval(Some(IdempotencyClass::Readback)));
        assert!(!tool_needs_approval(None));
    }

    #[test]
    fn projected_spend_is_turns_and_toolcalls_priced() {
        // Default rates: 1000 µ$/turn + 500 µ$/tool-call.
        let pb = kx_pricing::PriceBook::default();
        // turn 2 (turns_used=3) with no folded tool calls + 1 pending = 3*1000 + 1*500.
        let rounds: Vec<ReactRoundRecord> = Vec::new();
        let spend = react_projected_spend_micro_usd(&rounds, 2, 1);
        assert_eq!(spend, pb.estimate_spend(3, 1));
    }

    #[test]
    fn approval_gate_requests_then_grants_then_proceeds_recovery_stable() {
        let journal = InMemoryJournal::new();
        let mut projection = Projection::new();
        let anchor = fold_gate_anchor(&journal, &mut projection, true);
        let obs_id = MoteId::from_bytes([0xab; 32]);
        let mut folded = projection.current_seq();

        // First encounter: a Requested fact is appended + the action WAITS.
        let d1 = approval_gate_decision(
            &journal,
            &mut projection,
            &mut folded,
            &anchor,
            obs_id,
            "fs-write",
            "1",
        );
        assert_eq!(d1, ApprovalGate::Wait);
        let request_id = approval_request_id(&anchor.instance_id, &obs_id);
        assert!(matches!(
            projection
                .approval_latest_for(&request_id)
                .map(|r| &r.state),
            Some(ApprovalState::Requested { .. })
        ));
        // Re-evaluating while still pending stays WAIT and does NOT double-request.
        let pending_before = projection.pending_approvals().len();
        let d2 = approval_gate_decision(
            &journal,
            &mut projection,
            &mut folded,
            &anchor,
            obs_id,
            "fs-write",
            "1",
        );
        assert_eq!(d2, ApprovalGate::Wait);
        assert_eq!(projection.pending_approvals().len(), pending_before);

        // Operator GRANTS → the gate PROCEEDS (the authorized action may fire).
        assert!(decide_approval(
            &journal,
            &mut projection,
            &mut folded,
            &request_id,
            true,
            42,
            "ok",
        ));
        assert_eq!(
            approval_gate_decision(
                &journal,
                &mut projection,
                &mut folded,
                &anchor,
                obs_id,
                "fs-write",
                "1",
            ),
            ApprovalGate::Proceed
        );
        // Recovery: a cold re-fold of the journal re-derives the SAME decision.
        let mut recovered = Projection::new();
        for e in journal.read_entries_by_seq(0..u64::MAX).unwrap() {
            recovered.fold(&e).unwrap();
        }
        assert_eq!(
            recovered
                .approval_latest_for(&request_id)
                .map(|r| r.state.as_u8()),
            Some(
                ApprovalState::Granted {
                    approver_id: 0,
                    reason: String::new(),
                    decided_unix_ms: 0
                }
                .as_u8()
            )
        );
        // A re-grant of a resolved request is an idempotent no-op.
        assert!(!decide_approval(
            &journal,
            &mut projection,
            &mut folded,
            &request_id,
            true,
            42,
            "again",
        ));
    }

    #[test]
    fn approval_gate_deny_dead_letters() {
        let journal = InMemoryJournal::new();
        let mut projection = Projection::new();
        let anchor = fold_gate_anchor(&journal, &mut projection, true);
        let obs_id = MoteId::from_bytes([0xcd; 32]);
        let mut folded = projection.current_seq();
        // Request, then DENY → the gate dead-letters (fail-closed).
        approval_gate_decision(
            &journal,
            &mut projection,
            &mut folded,
            &anchor,
            obs_id,
            "send-email",
            "1",
        );
        let request_id = approval_request_id(&anchor.instance_id, &obs_id);
        assert!(decide_approval(
            &journal,
            &mut projection,
            &mut folded,
            &request_id,
            false,
            7,
            "unsafe",
        ));
        assert_eq!(
            approval_gate_decision(
                &journal,
                &mut projection,
                &mut folded,
                &anchor,
                obs_id,
                "send-email",
                "1",
            ),
            ApprovalGate::DeadLetter
        );
        // The denied request leaves the pending inbox (latest state is not Requested).
        assert!(projection.pending_approvals().is_empty());
    }
}
