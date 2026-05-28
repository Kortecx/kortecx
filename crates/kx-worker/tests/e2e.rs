//! End-to-end P2.3 exit-gate witness: a worker **registers**, **pulls** a ready
//! Mote, runs it through the hosted executor, and **proposes** its commit — all
//! through the coordinator over real gRPC. Then the committed parent unblocks the
//! child, which the worker pulls and commits on the next round.
//!
//! The coordinator is a real [`CoordinatorService`] hosted on an in-process gRPC
//! server (ephemeral TCP loopback). Submission is done in-process via a service
//! clone (standing in for an external submitter); everything the *worker* does —
//! register / lease / report-commit — rides the gRPC client. The service is the
//! sole journal writer (D40): the worker only proposes.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::sync::Arc;
use std::time::Duration;

use kx_coordinator::proto::coordinator_server::{Coordinator, CoordinatorServer};
use kx_coordinator::{CoordinatorService, MoteState, WorkerId};
use kx_executor::{LocalResourceManager, MoteExecutor, TestMoteExecutor};
use kx_journal::InMemoryJournal;
use kx_worker::{Worker, WorkerClient};
use tonic::transport::Server;
use tonic::Request;

/// Submit a Mote + warrant in-process (test setup — an external submitter would
/// use the SubmitMote RPC).
async fn submit(svc: &CoordinatorService, mote: &kx_mote::Mote, warrant: &kx_warrant::WarrantSpec) {
    svc.submit_mote(Request::new(kx_coordinator::proto::SubmitMoteRequest {
        mote: Some(mote.clone().into()),
        warrant: Some(warrant.clone().into()),
    }))
    .await
    .unwrap();
}

async fn connect_worker(endpoint: String) -> Worker {
    // Brief retry while the in-process server binds (the kx-proto pattern).
    let mut client = None;
    for _ in 0..100 {
        match WorkerClient::connect(endpoint.clone()).await {
            Ok(c) => {
                client = Some(c);
                break;
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(10)).await,
        }
    }
    let client = client.expect("worker connects to the coordinator");
    let executor: Arc<dyn MoteExecutor> = Arc::new(TestMoteExecutor::deterministic());
    Worker::register(
        client,
        common::WORKER_CLASS,
        "inproc://worker-1",
        executor,
        LocalResourceManager::dev_defaults(),
        16,
    )
    .await
    .expect("worker registers")
}

#[tokio::test]
async fn worker_registers_pulls_runs_and_proposes_a_dag() {
    // One service, two clones sharing the same orchestration core: one hosts the
    // gRPC server, one stays here for submission + read-side assertions.
    let svc = CoordinatorService::new(InMemoryJournal::new());
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

    // A 2-Mote PURE DAG: root -> child.
    let root = common::pure_mote(1, &[]);
    let child = common::pure_mote(2, &[root.id]);
    let warrant = common::pure_warrant();
    submit(&svc, &root, &warrant).await;
    submit(&svc, &child, &warrant).await;

    let mut worker = connect_worker(format!("http://{addr}")).await;

    // Round 1: only the root is ready; the worker leases, runs, and proposes it.
    let n = worker.run_once().await.unwrap();
    assert_eq!(n, 1, "the root is committed this round");
    assert_eq!(svc.committed_count().await.unwrap(), 1);
    assert_eq!(svc.state_of(root.id).await.unwrap(), MoteState::Committed);

    // Round 2: the committed root unblocks the child.
    let n = worker.run_once().await.unwrap();
    assert_eq!(n, 1, "the child is committed once its parent is");
    assert_eq!(svc.committed_count().await.unwrap(), 2);
    assert_eq!(svc.state_of(child.id).await.unwrap(), MoteState::Committed);

    // Round 3: nothing ready remains.
    assert_eq!(worker.run_once().await.unwrap(), 0, "the DAG is drained");

    // Heartbeat advances the registry liveness clock (0 at registration).
    assert!(
        worker.heartbeat(0).await.unwrap(),
        "coordinator acks heartbeat"
    );
    let record = svc
        .registry()
        .get(WorkerId(worker.worker_id()))
        .expect("worker is registered");
    assert!(
        record.last_heartbeat_ms > 0,
        "heartbeat recorded a wall-clock timestamp"
    );
}
