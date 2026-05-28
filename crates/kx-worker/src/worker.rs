//! [`Worker`] — registers with the coordinator, then leases / runs / proposes.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use kx_content::{ContentStore, LocalFsContentStore};
use kx_executor::{LocalResourceManager, MoteExecutor};
use kx_mote::{Mote, MoteId};
use kx_proto::proto;
use kx_warrant::{ExecutorClass, WarrantSpec};

use crate::client::WorkerClient;
use crate::error::WorkerError;
use crate::read_model::ReadModel;
use crate::{commit_builder, run};

/// `ReadEntries` page size when folding the local read model.
const READ_PAGE: u32 = 256;

/// A registered worker bound to one coordinator. Holds the hosted executor + a
/// resource manager (the verbatim P1 execution stack), the shared content store it
/// reads peer results from (data plane), and a local read model of committed results
/// folded incrementally from the coordinator's log (so reads stay off the hot path).
pub struct Worker {
    client: WorkerClient,
    id: u64,
    executor_class: ExecutorClass,
    executor: Arc<dyn MoteExecutor>,
    resource_manager: LocalResourceManager,
    store: Arc<LocalFsContentStore>,
    read_model: ReadModel,
    max_lease: u32,
}

impl Worker {
    /// Register `client` with the coordinator as a worker for `executor_class`,
    /// reachable at `endpoint`, and return a ready worker. `executor` +
    /// `resource_manager` host the P1 execution stack verbatim; `store` is the shared
    /// content-addressed store (the worker's executor publishes results to it and the
    /// worker reads peer results from it); `max_lease` bounds how many Motes a single
    /// [`run_once`](Self::run_once) pulls.
    pub async fn register(
        mut client: WorkerClient,
        executor_class: ExecutorClass,
        endpoint: impl Into<String>,
        executor: Arc<dyn MoteExecutor>,
        resource_manager: LocalResourceManager,
        store: Arc<LocalFsContentStore>,
        max_lease: u32,
    ) -> Result<Self, WorkerError> {
        let id = client.register_worker(executor_class, endpoint).await?;
        Ok(Self {
            client,
            id,
            executor_class,
            executor,
            resource_manager,
            store,
            read_model: ReadModel::new(),
            max_lease,
        })
    }

    /// The coordinator-assigned worker id.
    #[must_use]
    pub fn worker_id(&self) -> u64 {
        self.id
    }

    /// Read a peer's committed result: fold the coordinator's committed-entry log
    /// into the local read model until `mote_id`'s commit is seen, resolve its
    /// `result_ref`, and fetch the bytes from the shared content store. This is the
    /// distributed-read path (D55) — the journal stays single-writer, the content
    /// store is the shared data plane.
    pub async fn peer_read(&mut self, mote_id: MoteId) -> Result<Vec<u8>, WorkerError> {
        loop {
            if let Some(result_ref) = self.read_model.result_ref_of(&mote_id) {
                let bytes = self
                    .store
                    .get(&result_ref)
                    .map_err(|_| WorkerError::ContentMissing(result_ref))?;
                return Ok(bytes.to_vec());
            }
            let cursor = self.read_model.cursor();
            let (entries, next_seq) = self.client.read_entries(cursor, READ_PAGE).await?;
            self.read_model.fold(entries, next_seq);
            if next_seq == cursor {
                // Caught up to current_seq without finding the commit.
                return Err(WorkerError::NotCommitted(mote_id));
            }
        }
    }

    /// Lease one batch of ready PURE Motes, run each through the hosted executor,
    /// and propose its commit. Returns the number of commits the coordinator
    /// accepted this round (0 when no ready work matches).
    pub async fn run_once(&mut self) -> Result<usize, WorkerError> {
        let items = self
            .client
            .lease_work(self.id, self.executor_class, self.max_lease)
            .await?;

        let mut committed = 0usize;
        for item in items {
            let mote: Mote = item
                .mote
                .ok_or(WorkerError::MissingField("mote"))?
                .try_into()?;
            let warrant: WarrantSpec = item
                .warrant
                .ok_or(WorkerError::MissingField("warrant"))?
                .try_into()?;

            let result_ref =
                run::run_pure(&mote, &warrant, &*self.executor, &self.resource_manager)?;
            let request =
                commit_builder::report_commit_request(&mote, &warrant, result_ref, self.id);
            let response = self.client.report_commit(request).await?;

            match proto::CommitOutcome::try_from(response.outcome) {
                Ok(proto::CommitOutcome::Committed | proto::CommitOutcome::AlreadyCommitted) => {
                    tracing::info!(
                        worker_id = self.id,
                        seq = response.committed_seq,
                        mote = ?mote.id,
                        "commit proposal accepted"
                    );
                    committed += 1;
                }
                _ => return Err(WorkerError::CommitRejected(response.detail)),
            }
        }
        Ok(committed)
    }

    /// Send a liveness heartbeat with the current wall-clock and `in_flight` count.
    /// Returns the coordinator's ack.
    pub async fn heartbeat(&mut self, in_flight: u32) -> Result<bool, WorkerError> {
        self.client.heartbeat(self.id, now_ms(), in_flight).await
    }
}

/// Wall-clock milliseconds since the Unix epoch (liveness only; never hashed).
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| u64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}
