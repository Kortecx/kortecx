//! [`WorkerError`] — the worker's failure vocabulary.

use thiserror::Error;

/// Errors raised while a worker registers, leases, runs, or proposes commits.
///
/// The transport / RPC / executor sources are boxed: `tonic::Status` alone is
/// ~176 bytes, and an un-boxed variant would bloat every `Result` the worker
/// returns (`clippy::result_large_err`).
#[derive(Debug, Error)]
pub enum WorkerError {
    /// Establishing the gRPC channel to the coordinator failed.
    #[error("coordinator transport error: {0}")]
    Transport(Box<tonic::transport::Error>),

    /// A coordinator RPC returned an error status (e.g. an unregistered worker, a
    /// rejected proposal).
    #[error("coordinator RPC failed: {0}")]
    Rpc(Box<tonic::Status>),

    /// A `proto -> domain` conversion of a leased Mote/warrant failed.
    #[error("wire conversion failed: {0}")]
    Convert(#[from] kx_proto::ConvertError),

    /// A leased `WorkItem` was missing a required field (the coordinator always
    /// sends both, so this is a malformed response).
    #[error("a leased WorkItem was missing its {0}")]
    MissingField(&'static str),

    /// Running a leased Mote through the hosted executor failed.
    #[error("executing a leased Mote failed: {0}")]
    Execute(Box<kx_executor::LifecycleError>),

    /// The coordinator accepted the request but rejected the commit proposal.
    #[error("coordinator rejected the commit: {0}")]
    CommitRejected(String),

    /// A peer read asked for a Mote that is not committed in the coordinator's log.
    #[error("mote {0:?} is not committed")]
    NotCommitted(kx_mote::MoteId),

    /// A committed result's bytes are absent from the shared content store.
    #[error("content {0:?} is missing from the shared store")]
    ContentMissing(kx_content::ContentRef),

    /// The coordinator did NOT ack a `ReportEffectStaged` (`ack == false`). The
    /// worker MUST NOT fire the effect without the durable staged-intent record
    /// (D58 §2) — firing first would re-fire on recovery with no staged hint (the
    /// double-effect hazard). The dispatch is aborted; the Mote stays leasable.
    #[error("coordinator did not ack EffectStaged for mote {0:?}")]
    EffectStagedRejected(kx_mote::MoteId),

    /// Firing a WORLD-MUTATING effect through the capability broker failed (boxed:
    /// `BrokerError` is large, and an un-boxed variant would bloat every `Result`).
    #[error("broker dispatch failed: {0}")]
    Dispatch(Box<kx_capability::BrokerError>),

    /// A non-PURE Mote's `tool_contract` named no capability to dispatch under, so
    /// the worker cannot resolve which effect to fire.
    #[error("cannot resolve a capability from the tool_contract of mote {0:?}")]
    CapabilityResolution(kx_mote::MoteId),

    /// PR-2d-2 (react-tools-live): a WARRANT-GRANTED `StageThenCommit` tool Mote
    /// (a ReAct observation) was leased WITHOUT its coordinator-validated
    /// `tool_args`. The worker NEVER fires a granted tool with an empty payload
    /// — a dropped/absent args field must fail closed, not fire a wrong effect.
    #[error("mote {0:?} grants a tool but carried no coordinator-validated tool_args")]
    MissingToolArgs(kx_mote::MoteId),

    /// A leased WORLD-MUTATING / READ-ONLY-NONDET effect (tool / MCP / IO)
    /// exceeded the operator-set per-Mote wall-clock deadline (`KX_SERVE_TOOL_DEADLINE_SECS`,
    /// default OFF). The in-flight dispatch future is cancelled — equivalent to a
    /// mini-crash of that one Mote, made safe by the broker's idempotency-key dedup +
    /// the R-13 re-dispatch guard on any retry. Classified TRANSIENT so it retries
    /// within the F4 budget, then dead-letters (a persistently-hung tool never pins a
    /// pool worker's slot forever). Off-journal: a live wall-clock check, never a fact.
    #[error("mote {0:?} exceeded the per-Mote execution deadline")]
    ExecutionTimedOut(kx_mote::MoteId),
}

impl From<kx_capability::BrokerError> for WorkerError {
    fn from(error: kx_capability::BrokerError) -> Self {
        Self::Dispatch(Box::new(error))
    }
}

impl From<tonic::transport::Error> for WorkerError {
    fn from(error: tonic::transport::Error) -> Self {
        Self::Transport(Box::new(error))
    }
}

impl From<tonic::Status> for WorkerError {
    fn from(status: tonic::Status) -> Self {
        Self::Rpc(Box::new(status))
    }
}

impl From<kx_executor::LifecycleError> for WorkerError {
    fn from(error: kx_executor::LifecycleError) -> Self {
        Self::Execute(Box::new(error))
    }
}

/// How a per-Mote execution failure should be handled by the worker's F4 dead-letter
/// path: a transient *infrastructure* hiccup (back off + retry within a budget) versus
/// a *terminal logic* failure (dead-letter immediately — a retry cannot succeed).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FailureClass {
    /// Storage / broker / spawn / timeout / resource-contention — may vary across
    /// attempts; retry within the budget.
    TransientInfra,
    /// Refusals, validation verdicts, the R-13 double-fire guard, a non-zero body
    /// exit, an unresolvable capability, a malformed lease item — deterministic across
    /// retries; dead-letter now.
    TerminalLogic,
}

/// Classify a per-Mote [`WorkerError`] as transient-infra (retry) or terminal-logic
/// (dead-letter). **Conservative by construction**: only clearly-transient infrastructure
/// errors retry; everything else — including any future/unknown nested variant — defaults
/// to terminal, so a deterministic failure is never re-leased in a storm (the F4 spin
/// this exists to close). Mirrors `kx_runtime::failure_policy::classify_lifecycle_error`
/// for the distributed worker path; the worker cannot depend on kx-runtime (the engine),
/// so the same conservative partition is re-expressed over the public kx-executor error
/// types it already sees. The partition matches the recovery oracle's pre-commit-crash
/// (transient) vs terminal split exactly.
pub(crate) fn classify_worker_failure(err: &WorkerError) -> FailureClass {
    use FailureClass::{TerminalLogic, TransientInfra};
    match err {
        // Delegated nested classification of an executor lifecycle error.
        WorkerError::Execute(e) => classify_lifecycle(e),
        // Clearly-transient infrastructure that may vary across attempts: a broker dispatch
        // hiccup (the broker's idempotency-key dedup makes a bounded WM re-dispatch
        // exactly-once), a non-acked stage, a not-yet-visible peer result, or a
        // transport/RPC blip (normally batch-level, but transient if it reaches here).
        WorkerError::Dispatch(_)
        | WorkerError::EffectStagedRejected(_)
        | WorkerError::ContentMissing(_)
        | WorkerError::ExecutionTimedOut(_)
        | WorkerError::Transport(_)
        | WorkerError::Rpc(_) => TransientInfra,
        // Everything else is deterministic — a malformed lease item, an unresolvable
        // capability, missing coordinator-validated tool args (PR-2d-2: never fire a
        // granted tool empty), a rejected commit, a not-committed peer read:
        // dead-letter now.
        WorkerError::Convert(_)
        | WorkerError::MissingField(_)
        | WorkerError::CapabilityResolution(_)
        | WorkerError::MissingToolArgs(_)
        | WorkerError::CommitRejected(_)
        | WorkerError::NotCommitted(_) => TerminalLogic,
    }
}

fn classify_lifecycle(err: &kx_executor::LifecycleError) -> FailureClass {
    use kx_executor::LifecycleError;
    use FailureClass::{TerminalLogic, TransientInfra};
    match err {
        // A journal append hiccup is transient storage.
        LifecycleError::JournalAppend(_) => TransientInfra,
        LifecycleError::ResourceAcquire(e) => classify_resource(e),
        LifecycleError::ExecutorRun(e) => classify_executor(e),
        LifecycleError::CommitProtocol(e) => classify_commit(e),
        // A submission refusal (a permission/construction verdict) or an opaque internal
        // lifecycle error: both deterministic, never retried.
        LifecycleError::Refused(_) | LifecycleError::Internal(_) => TerminalLogic,
    }
}

fn classify_resource(err: &kx_executor::ResourceError) -> FailureClass {
    use kx_executor::ResourceError;
    use FailureClass::{TerminalLogic, TransientInfra};
    match err {
        // Momentary saturation — a backoff may free a slot.
        ResourceError::NoCapacity => TransientInfra,
        // The Mote's own ceilings are exceeded, or an internal/unknown-slot bug —
        // deterministic across retries.
        ResourceError::CpuCapExceeded { .. }
        | ResourceError::MemCapExceeded { .. }
        | ResourceError::FdCapExceeded { .. }
        | ResourceError::UnknownSlot(_)
        | ResourceError::Internal(_) => TerminalLogic,
    }
}

fn classify_executor(err: &kx_executor::MoteExecutorError) -> FailureClass {
    use kx_executor::MoteExecutorError;
    use FailureClass::{TerminalLogic, TransientInfra};
    match err {
        // Transient OS / sandbox / timeout conditions that may vary across attempts.
        MoteExecutorError::ProcessSpawnFailed { .. }
        | MoteExecutorError::SandboxLoadFailed { .. }
        | MoteExecutorError::RlimitFailed { .. }
        | MoteExecutorError::WallClockTimedOut { .. } => TransientInfra,
        // Deterministic: the backend can't run this class, the rootfs/profile is bad, or
        // the body itself exited non-zero (an agent *logic* failure — incl. the shaper
        // executor's fail-closed verdict on a malformed/over-budget/unknown-role model
        // proposal, which surfaces as `Internal`/`BodyExited`). A retry cannot fix it.
        MoteExecutorError::BackendUnsupported { .. }
        | MoteExecutorError::RootfsExtractFailed { .. }
        | MoteExecutorError::ProfileSyntaxError { .. }
        | MoteExecutorError::BodyExited { .. }
        | MoteExecutorError::Internal { .. } => TerminalLogic,
    }
}

fn classify_commit(err: &kx_executor::CommitProtocolError) -> FailureClass {
    use kx_executor::CommitProtocolError;
    use FailureClass::{TerminalLogic, TransientInfra};
    match err {
        // Storage / broker / probe transients — the broker's idempotency-key dedup makes
        // a WM re-dispatch exactly-once, so a bounded retry is safe.
        CommitProtocolError::BrokerDispatchFailed { .. }
        | CommitProtocolError::ContentStorePutFailed { .. }
        | CommitProtocolError::JournalAppendCommittedFailed { .. }
        | CommitProtocolError::JournalAppendEffectStagedFailed { .. }
        | CommitProtocolError::ProbeFailed { .. } => TransientInfra,
        // Deterministic protocol / validation verdicts + the R-13 re-dispatch refusal (the
        // double-fire guard — MUST NOT be retried).
        CommitProtocolError::R11ResultRefIncomplete { .. }
        | CommitProtocolError::R12CommittedNotProofOfValidity { .. }
        | CommitProtocolError::R13WmReDispatchRefused { .. }
        | CommitProtocolError::CompensateFailed { .. }
        | CommitProtocolError::Internal { .. } => TerminalLogic,
    }
}
