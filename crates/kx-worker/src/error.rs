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
