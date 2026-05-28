//! [`CoordinatorService`] — the gRPC `Coordinator` server implementation.
//!
//! Holds the worker registry (behind a trait) and a handle to the single
//! orchestration core thread (which owns the journal + projection + hosted
//! scheduler). The four RPCs are thin adapters: convert at the untrusted boundary,
//! route to the registry or the core, map errors to [`tonic::Status`].

use std::sync::Arc;

use kx_journal::Journal;
use kx_mote::{Mote, MoteId};
use kx_projection::MoteState;
use kx_proto::proto;
use kx_proto::proto::coordinator_server::Coordinator;
use kx_scheduler::WorkerId;
use kx_warrant::WarrantSpec;
use tonic::{Request, Response, Status};

use crate::commit;
use crate::error::CoordinatorError;
use crate::registry::{InMemoryWorkerRegistry, RegistryError, WorkerRegistry};
use crate::state::CoreHandle;

/// The coordinator gRPC service: hosts the scheduler, owns the worker registry,
/// and is the sole journal writer per run.
#[derive(Clone)]
pub struct CoordinatorService {
    core: CoreHandle,
    registry: Arc<dyn WorkerRegistry>,
}

impl CoordinatorService {
    /// Build a coordinator over `journal` with the default in-memory worker
    /// registry. Takes sole ownership of the journal (the single-writer handle).
    pub fn new<J: Journal + Send + 'static>(journal: J) -> Self {
        Self::with_registry(journal, Arc::new(InMemoryWorkerRegistry::new()))
    }

    /// Build a coordinator over `journal` with a caller-supplied worker registry.
    pub fn with_registry<J: Journal + Send + 'static>(
        journal: J,
        registry: Arc<dyn WorkerRegistry>,
    ) -> Self {
        Self {
            core: CoreHandle::spawn(journal),
            registry,
        }
    }

    /// Read-side accessor: the current [`MoteState`] of `mote_id` in the
    /// coordinator's projection (the journal's folded read view).
    pub async fn state_of(&self, mote_id: MoteId) -> Result<MoteState, CoordinatorError> {
        self.core.state_of(mote_id).await
    }

    /// Read-side accessor: the number of `Committed` (non-repudiated) Motes.
    pub async fn committed_count(&self) -> Result<usize, CoordinatorError> {
        self.core.committed_count().await
    }

    /// Read-side accessor: the current ready set — submitted Motes whose parents
    /// are all `Committed`-and-not-`Repudiated`. The dispatch surface P2.3 consumes.
    pub async fn ready_set(&self) -> Result<Vec<MoteId>, CoordinatorError> {
        self.core.ready_set().await
    }

    /// Borrow the worker registry (diagnostics / operator queries).
    #[must_use]
    pub fn registry(&self) -> &dyn WorkerRegistry {
        self.registry.as_ref()
    }
}

#[tonic::async_trait]
impl Coordinator for CoordinatorService {
    #[tracing::instrument(skip_all)]
    async fn register_worker(
        &self,
        request: Request<proto::RegisterWorkerRequest>,
    ) -> Result<Response<proto::RegisterWorkerResponse>, Status> {
        let req = request.into_inner();
        let proto_class = proto::ExecutorClass::try_from(req.executor_class).map_err(|_| {
            Status::invalid_argument(format!("unknown executor_class {}", req.executor_class))
        })?;
        let executor_class =
            kx_warrant::ExecutorClass::try_from(proto_class).map_err(CoordinatorError::from)?;
        let id = self.registry.register(executor_class, req.endpoint);
        tracing::info!(worker_id = id.0, ?executor_class, "worker registered");
        Ok(Response::new(proto::RegisterWorkerResponse {
            worker_id: id.0,
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn heartbeat(
        &self,
        request: Request<proto::HeartbeatRequest>,
    ) -> Result<Response<proto::HeartbeatResponse>, Status> {
        let req = request.into_inner();
        self.registry
            .heartbeat(WorkerId(req.worker_id), req.timestamp_ms, req.in_flight)
            .map_err(|RegistryError::UnknownWorker(worker)| {
                CoordinatorError::UnknownWorker(worker)
            })?;
        Ok(Response::new(proto::HeartbeatResponse { ack: true }))
    }

    #[tracing::instrument(skip_all)]
    async fn submit_mote(
        &self,
        request: Request<proto::SubmitMoteRequest>,
    ) -> Result<Response<proto::SubmitMoteResponse>, Status> {
        let req = request.into_inner();
        // IDENTITY INVARIANT (D53): `TryFrom<proto::Mote>` re-derives the MoteId
        // Rust-side; the wire `mote_id` is advisory and never trusted.
        let mote: Mote = req
            .mote
            .ok_or_else(|| Status::invalid_argument("SubmitMote.mote is required"))?
            .try_into()
            .map_err(CoordinatorError::from)?;
        let warrant: WarrantSpec = req
            .warrant
            .ok_or_else(|| Status::invalid_argument("SubmitMote.warrant is required"))?
            .try_into()
            .map_err(CoordinatorError::from)?;

        let outcome = self.core.submit(mote, warrant).await?;
        let status = if outcome.duplicate {
            proto::SubmitStatus::Duplicate
        } else {
            proto::SubmitStatus::Accepted
        };
        Ok(Response::new(proto::SubmitMoteResponse {
            mote_id: outcome.mote_id.as_bytes().to_vec(),
            status: status as i32,
            detail: String::new(),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn report_commit(
        &self,
        request: Request<proto::ReportCommitRequest>,
    ) -> Result<Response<proto::ReportCommitResponse>, Status> {
        let req = request.into_inner();
        // D40 admission: only a registered worker may propose a commit.
        let worker = WorkerId(req.worker_id);
        if self.registry.get(worker).is_none() {
            return Err(CoordinatorError::UnknownWorker(worker).into());
        }
        let proposal = commit::assemble(req)?;
        let applied = self.core.commit(proposal).await?;
        let outcome = if applied.already_committed {
            proto::CommitOutcome::AlreadyCommitted
        } else {
            proto::CommitOutcome::Committed
        };
        tracing::info!(
            seq = applied.committed_seq,
            already_committed = applied.already_committed,
            "commit recorded"
        );
        Ok(Response::new(proto::ReportCommitResponse {
            committed_seq: applied.committed_seq,
            outcome: outcome as i32,
            detail: String::new(),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn lease_work(
        &self,
        request: Request<proto::LeaseWorkRequest>,
    ) -> Result<Response<proto::LeaseWorkResponse>, Status> {
        let req = request.into_inner();
        // Admission: only a registered worker may lease (mirrors report_commit).
        let worker = WorkerId(req.worker_id);
        if self.registry.get(worker).is_none() {
            return Err(CoordinatorError::UnknownWorker(worker).into());
        }
        let proto_class = proto::ExecutorClass::try_from(req.executor_class).map_err(|_| {
            Status::invalid_argument(format!("unknown executor_class {}", req.executor_class))
        })?;
        let executor_class =
            kx_warrant::ExecutorClass::try_from(proto_class).map_err(CoordinatorError::from)?;
        let max = usize::try_from(req.max_motes).unwrap_or(usize::MAX);
        let work = self.core.lease_work(executor_class, max).await?;
        let items = work
            .into_iter()
            .map(|(mote, warrant)| proto::WorkItem {
                mote: Some(mote.into()),
                warrant: Some(warrant.into()),
            })
            .collect();
        Ok(Response::new(proto::LeaseWorkResponse { items }))
    }
}
