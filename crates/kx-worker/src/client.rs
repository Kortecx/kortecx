//! [`WorkerClient`] — a thin typed wrapper over the generated gRPC
//! `CoordinatorClient`. Each method converts at the boundary and unwraps the
//! response; the [`Worker`](crate::Worker) drives them.

use kx_proto::proto;
use kx_proto::proto::coordinator_client::CoordinatorClient;
use kx_warrant::ExecutorClass;
use tonic::transport::Channel;

use crate::error::WorkerError;

/// A worker's connection to the coordinator (the four worker-facing RPCs:
/// register / heartbeat / lease / report-commit).
#[derive(Clone)]
pub struct WorkerClient {
    inner: CoordinatorClient<Channel>,
}

impl WorkerClient {
    /// Connect to a coordinator at `endpoint` (`http://host:port` for TCP across
    /// nodes, or a Unix-socket URI locally).
    pub async fn connect(endpoint: impl Into<String>) -> Result<Self, WorkerError> {
        let inner = CoordinatorClient::connect(endpoint.into()).await?;
        Ok(Self { inner })
    }

    /// Wrap an already-established channel (e.g. an in-process test transport).
    #[must_use]
    pub fn from_channel(channel: Channel) -> Self {
        Self {
            inner: CoordinatorClient::new(channel),
        }
    }

    /// Register as a worker for `executor_class`, reachable at `endpoint`. Returns
    /// the coordinator-assigned worker id.
    pub async fn register_worker(
        &mut self,
        executor_class: ExecutorClass,
        endpoint: impl Into<String>,
    ) -> Result<u64, WorkerError> {
        let resp = self
            .inner
            .register_worker(proto::RegisterWorkerRequest {
                executor_class: proto::ExecutorClass::from(executor_class) as i32,
                endpoint: endpoint.into(),
            })
            .await?
            .into_inner();
        Ok(resp.worker_id)
    }

    /// Report liveness: `timestamp_ms` is wall-clock (liveness only, never hashed),
    /// `in_flight` the number of Motes currently executing. Returns the ack.
    pub async fn heartbeat(
        &mut self,
        worker_id: u64,
        timestamp_ms: u64,
        in_flight: u32,
    ) -> Result<bool, WorkerError> {
        let resp = self
            .inner
            .heartbeat(proto::HeartbeatRequest {
                worker_id,
                timestamp_ms,
                in_flight,
            })
            .await?
            .into_inner();
        Ok(resp.ack)
    }

    /// Pull up to `max_motes` ready PURE Motes runnable on `executor_class`.
    pub async fn lease_work(
        &mut self,
        worker_id: u64,
        executor_class: ExecutorClass,
        max_motes: u32,
    ) -> Result<Vec<proto::WorkItem>, WorkerError> {
        let resp = self
            .inner
            .lease_work(proto::LeaseWorkRequest {
                worker_id,
                executor_class: proto::ExecutorClass::from(executor_class) as i32,
                max_motes,
            })
            .await?
            .into_inner();
        Ok(resp.items)
    }

    /// Propose a commit. The coordinator validates + appends (sole writer, D40) and
    /// returns the assigned seq + outcome.
    pub async fn report_commit(
        &mut self,
        request: proto::ReportCommitRequest,
    ) -> Result<proto::ReportCommitResponse, WorkerError> {
        Ok(self.inner.report_commit(request).await?.into_inner())
    }
}
