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
            | CoordinatorError::Scheduler(_) => Self::invalid_argument(message),
            CoordinatorError::Journal(_) | CoordinatorError::Projection(_) => {
                Self::internal(message)
            }
            CoordinatorError::CoreUnavailable => Self::unavailable(message),
        }
    }
}
