//! [`CoordinatorError`] — the coordinator's failure vocabulary plus its mapping
//! to [`tonic::Status`] at the gRPC boundary.
//!
//! Direction of blame drives the status code: malformed or inadmissible client
//! requests (bad hash lengths, `*_UNSPECIFIED` enums, identity mismatch, unknown
//! worker / Mote, duplicate submission) map to `INVALID_ARGUMENT`; durable-layer
//! faults (journal / projection) map to `INTERNAL`; a downed orchestration core
//! maps to `UNAVAILABLE`.

use kx_journal::JournalError;
use kx_mote::MoteId;
use kx_projection::ProjectionError;
use kx_proto::ConvertError;
use kx_scheduler::{SchedulerError, WorkerId};
use thiserror::Error;

/// Errors raised while servicing a coordinator RPC.
#[derive(Debug, Error)]
pub enum CoordinatorError {
    /// A flat 32-byte wire field on a `ReportCommit` request was the wrong length.
    #[error("field {field} expected 32 bytes, got {len}")]
    BadHashLength {
        /// Which wire field failed validation.
        field: &'static str,
        /// The actual byte length received.
        len: usize,
    },

    /// The reported `idempotency_key` did not equal the Mote's identity bytes.
    /// The journal dedupes `Committed` entries by `idempotency_key`, and the
    /// identity substrate fixes `idempotency_key == MoteId` (`idempotency.md`),
    /// so a mismatch is a malformed proposal, not a recoverable state.
    #[error("idempotency_key does not match mote_id for {0:?}")]
    IdentityMismatch(MoteId),

    /// A `proto -> domain` conversion failed at the untrusted boundary.
    #[error(transparent)]
    Convert(#[from] ConvertError),

    /// The worker named in the request has not registered.
    #[error("unknown worker {0:?}")]
    UnknownWorker(WorkerId),

    /// The Mote named in a `ReportCommit` was never submitted to this coordinator.
    #[error("unknown mote {0:?} (never submitted)")]
    UnknownMote(MoteId),

    /// A `ReportFailure` (F4 worker dead-letter) named a Mote the reporting worker
    /// does not currently hold a lease on. A worker may only dead-letter work it was
    /// actually leased — the admission gate that keeps one worker from terminating
    /// another's (or phantom) Motes.
    #[error("mote {mote:?} is not leased to worker {worker:?}")]
    NotLeased {
        /// The Mote the worker tried to dead-letter.
        mote: MoteId,
        /// The worker that issued the (rejected) `ReportFailure`.
        worker: WorkerId,
    },

    /// `RegisterRun` was called on a run whose journal already has entries but no
    /// `RunRegistered` fact at seq=1 (a run started without registration). Run
    /// registration must be the FIRST journal fact (M1.1, D64); registering after
    /// the run has begun would violate the seq=1 / once-per-run invariant.
    #[error("run already started without registration; RegisterRun must be the first fact")]
    RunAlreadyStarted,

    /// `SubmitMote` was called before the run was registered (no seq=1
    /// `RunRegistered` fact). M1.3 forces registration-before-submit (D64/D98:
    /// run identity is the explicit `RegisterRun` RPC, never lazy-on-submit) so
    /// every admitted Mote anchors to a real `instance_id` (the resume key + the
    /// run-scoped idempotency-token root). An ordering precondition failure, not
    /// a refusal of the construction.
    #[error("run not registered; call RegisterRun before submitting any Mote (M1.3)")]
    RunNotRegistered,

    /// A `SubmitMote` was REFUSED by a submission-time predicate (M1.3): R-1 /
    /// R-7 / R-8 / R-14 / R-15 / R-10, or the D66 fail-closed-on-tool-resolution
    /// gate. The request was well-formed; the *construction* is unsafe. The
    /// service maps this to a `SUBMIT_STATUS_REJECTED` response (carrying the
    /// refusal detail), not a transport error — so it never reaches the `Status`
    /// conversion on the success path; the defensive mapping below classifies it
    /// as a precondition failure.
    #[error("submission refused: {0}")]
    SubmissionRefused(#[from] kx_refusal::SubmissionRefusal),

    /// A `react_seed` submit (PR-2d-1) could not seed a live ReAct chain: the seed
    /// Mote carries no instruction prompt, the coordinator has no content store
    /// (the durable anchor — and therefore crash recovery — would be impossible),
    /// or the anchor write failed. LOUD by design (the flag is explicit intent,
    /// unlike replan's silent non-anchor): a chain that silently could not recover
    /// would violate the durability law.
    #[error("react seed refused: {0}")]
    ReactSeedRefused(&'static str),

    /// A `ReportCommit` proposed a `result_ref` whose bytes are not present in the
    /// shared content store (D55 phantom-ref guard). When the coordinator is built
    /// with a store handle, it verifies `store.contains(result_ref)` before
    /// committing, so a worker cannot record a result it never published.
    #[error("result_ref for mote {0:?} is absent from the content store")]
    ResultRefAbsent(MoteId),

    /// A `ReportCommit` declared more parents than the journal encodes. Validated
    /// up front so a malformed proposal cannot poison a group-commit batch.
    #[error("commit declares {got} parents, exceeds the maximum {max}")]
    TooManyParents {
        /// Parents declared by the request.
        got: usize,
        /// The journal's per-entry maximum.
        max: usize,
    },

    /// A `ReportCommit` carried a `Data` edge marked `non_cascade` — forbidden by
    /// `journal-entry.md` §11 (the encoder would reject it). Validated up front so a
    /// malformed proposal cannot poison a group-commit batch.
    #[error("commit has a Data-edge parent marked non_cascade (forbidden)")]
    DataEdgeNonCascade,

    /// Hosted-scheduler bookkeeping failure (e.g. duplicate submission).
    #[error(transparent)]
    Scheduler(#[from] SchedulerError),

    /// The journal append (the sole-writer path) failed.
    #[error(transparent)]
    Journal(#[from] JournalError),

    /// Folding the new entry into the read-side projection failed.
    #[error(transparent)]
    Projection(#[from] ProjectionError),

    /// The orchestration core thread is not reachable (it exited, e.g. on a
    /// startup recovery failure). The journal is the durable truth; a restart
    /// re-folds from it.
    #[error("coordinator orchestration core is unavailable")]
    CoreUnavailable,

    /// A group-commit batch failed at the durable layer (journal/projection). The
    /// batch is atomic, so nothing was written; every waiter in the batch receives
    /// this so it can retry. Carries the underlying error's message (the source
    /// error is not `Clone`, so it is stringified to fan out to all waiters).
    #[error("group commit failed: {0}")]
    CommitFailed(String),
}

impl From<CoordinatorError> for tonic::Status {
    fn from(error: CoordinatorError) -> Self {
        let message = error.to_string();
        match error {
            CoordinatorError::BadHashLength { .. }
            | CoordinatorError::IdentityMismatch(_)
            | CoordinatorError::Convert(_)
            | CoordinatorError::UnknownWorker(_)
            | CoordinatorError::UnknownMote(_)
            | CoordinatorError::ResultRefAbsent(_)
            | CoordinatorError::TooManyParents { .. }
            | CoordinatorError::DataEdgeNonCascade
            | CoordinatorError::Scheduler(_) => Self::invalid_argument(message),
            // The run is not in a state that allows registration (it already
            // began) — the gRPC-canonical code for a state precondition failure.
            // Submit-before-register is the same class of ordering violation; the F4
            // lease-admission failure (a worker that does not hold the lease it tried to
            // dead-letter) is likewise a precondition. A `SubmissionRefused` is normally
            // consumed into a REJECTED response before this conversion (this arm is
            // defensive), and is the same class of construction precondition.
            CoordinatorError::RunAlreadyStarted
            | CoordinatorError::RunNotRegistered
            | CoordinatorError::NotLeased { .. }
            | CoordinatorError::SubmissionRefused(_)
            | CoordinatorError::ReactSeedRefused(_) => Self::failed_precondition(message),
            CoordinatorError::Journal(_)
            | CoordinatorError::Projection(_)
            | CoordinatorError::CommitFailed(_) => Self::internal(message),
            CoordinatorError::CoreUnavailable => Self::unavailable(message),
        }
    }
}
