//! F4 worker dead-letter — the worker-side spin-prevention witness.
//!
//! A live worker whose executor TERMINALLY fails a leased Mote must NOT `?`-abort the
//! batch and re-lease the failing Mote forever (the spin PR-9b's startup probe only
//! narrowly patched). Instead `run_once` classifies the failure, reports a terminal
//! `Failed` to the coordinator (F4), and continues — so a bad Mote dead-letters cleanly
//! and a healthy sibling in the same batch still commits.
//!
//! Topology mirrors `e2e.rs`: one in-process `CoordinatorService` on loopback, a worker
//! whose executor is a tiny test fixture that fails a chosen Mote and stores results for
//! the rest. kx-executor source is untouched (the fixture is a local `MoteExecutor` impl).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::sync::Arc;
use std::time::Duration;

use kx_content::{ContentStore, LocalFsContentStore};
use kx_coordinator::proto::coordinator_server::{Coordinator, CoordinatorServer};
use kx_coordinator::{CoordinatorService, MoteState};
use kx_executor::{
    LocalResourceManager, MoteExecutionResult, MoteExecutor, MoteExecutorError, Rootfs,
};
use kx_journal::InMemoryJournal;
use kx_mote::{Mote, MoteId};
use kx_warrant::{ExecutorClass, WarrantSpec};
use kx_worker::{Worker, WorkerClient};
use tempfile::TempDir;
use tonic::transport::Server;
use tonic::Request;

/// A `MoteExecutor` that fails (terminal `Internal`) for one chosen Mote and publishes a
/// stable result for every other Mote. Models the shaper executor's fail-closed verdict
/// on a malformed proposal (which surfaces as `Internal`), or any deterministic body
/// failure — both must dead-letter, never spin.
#[derive(Debug)]
struct FailOneExecutor {
    fail: MoteId,
    store: Arc<LocalFsContentStore>,
}

impl MoteExecutor for FailOneExecutor {
    fn run(
        &self,
        mote: &Mote,
        _warrant: &WarrantSpec,
        _env: Option<Rootfs>,
    ) -> Result<MoteExecutionResult, MoteExecutorError> {
        if mote.id == self.fail {
            return Err(MoteExecutorError::Internal {
                reason: "injected terminal failure (fail-closed verdict)".into(),
            });
        }
        let mut bytes = b"kx-result:".to_vec();
        bytes.extend_from_slice(mote.id.as_bytes());
        let result_ref = self.store.put(&bytes).expect("publish result bytes");
        Ok(MoteExecutionResult {
            result_ref,
            started_at_epoch_ms: 0,
            finished_at_epoch_ms: 0,
        })
    }

    fn supports(&self, _executor_class: ExecutorClass) -> bool {
        true
    }
}

async fn spawn_coordinator(store: Arc<LocalFsContentStore>) -> (CoordinatorService, String) {
    let svc = CoordinatorService::with_store(InMemoryJournal::new(), store);
    let server_svc = svc.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    tokio::spawn(async move {
        Server::builder()
            .add_service(CoordinatorServer::new(server_svc))
            .serve(addr)
            .await
            .unwrap();
    });
    (svc, format!("http://{addr}"))
}

async fn submit(svc: &CoordinatorService, mote: &Mote, warrant: &WarrantSpec) {
    let _ = svc
        .register_run(Request::new(kx_coordinator::proto::RegisterRunRequest {
            recipe_fingerprint: vec![0x5au8; 32],
        }))
        .await;
    svc.submit_mote(Request::new(kx_coordinator::proto::SubmitMoteRequest {
        mote: Some(mote.clone().into()),
        warrant: Some(warrant.clone().into()),
        accept_at_least_once: false,
    }))
    .await
    .unwrap();
}

async fn connect(endpoint: &str) -> WorkerClient {
    for _ in 0..100 {
        if let Ok(c) = WorkerClient::connect(endpoint.to_string()).await {
            return c;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("worker could not connect to the coordinator");
}

/// A terminally-failing Mote in a lease batch dead-letters (not `?`-abort), a healthy
/// sibling still commits, and a subsequent poll never re-leases the dead Mote (no spin).
#[tokio::test]
async fn terminal_failure_dead_letters_without_aborting_or_spinning() {
    let dir = TempDir::new().unwrap();
    let store = Arc::new(LocalFsContentStore::open(dir.path()).unwrap());
    let (svc, endpoint) = spawn_coordinator(store.clone()).await;

    // Two independent root PURE Motes: one will fail terminally, the other succeed —
    // both ready in the SAME lease batch, proving the failure doesn't abort the batch.
    let bad = common::pure_mote(1, &[]);
    let good = common::pure_mote(2, &[]);
    let warrant = common::pure_warrant();
    submit(&svc, &bad, &warrant).await;
    submit(&svc, &good, &warrant).await;

    let executor = Arc::new(FailOneExecutor {
        fail: bad.id,
        store: store.clone(),
    });
    let mut worker = Worker::register(
        connect(&endpoint).await,
        common::WORKER_CLASS,
        "inproc://worker-f4",
        executor,
        LocalResourceManager::dev_defaults(),
        store.clone(),
        common::noop_broker(),
        16,
    )
    .await
    .unwrap();

    // One batch: the bad Mote dead-letters, the good Mote commits. run_once returns the
    // committed count (1) — crucially it does NOT return an Err (no batch abort).
    let committed = worker
        .run_once()
        .await
        .expect("run_once must not abort the batch on one Mote's terminal failure");
    assert_eq!(committed, 1, "the healthy sibling committed despite the failure");

    assert_eq!(
        svc.state_of(good.id).await.unwrap(),
        MoteState::Committed,
        "the good Mote committed"
    );
    assert_eq!(
        svc.state_of(bad.id).await.unwrap(),
        MoteState::Failed,
        "the failing Mote was dead-lettered (terminal Failed)"
    );

    // Spin-prevention: subsequent polls find NOTHING to do — the dead Mote left ready_set
    // and the good Mote is committed. run_once returns Ok(0) every time (no re-lease loop).
    for _ in 0..3 {
        assert_eq!(
            worker.run_once().await.unwrap(),
            0,
            "no work re-leased: the dead-lettered Mote is gone for good (no spin)"
        );
    }
}
