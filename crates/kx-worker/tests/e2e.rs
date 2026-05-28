//! End-to-end distributed witnesses over real gRPC + a shared content-addressed store.
//!
//! - **P2.4** (`committed_result_is_readable_by_coordinator_and_peer`): a result
//!   committed via one worker is readable by the coordinator and by another worker;
//!   plus `phantom_result_ref_is_rejected` (D55 store verification).
//! - **P2.5 / P2 EXIT GATE** (`two_workers_share_the_workload_via_placement`): two
//!   workers of the same class run a workflow distributed, placement v2 (D56) balances
//!   the Motes across them, all commit exactly once.
//!
//! Topology: one in-process `CoordinatorService` built `with_store` (so it VERIFIES each
//! committed `result_ref` against the shared store — D55) on loopback; workers run PURE
//! Motes through a **storing** executor (real bytes land in the shared store before they
//! propose). The store is one `LocalFsContentStore` root all nodes share (single host
//! now; the S3 backend is the cross-host impl at P5.5).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::sync::Arc;
use std::time::Duration;

use kx_content::{ContentRef, ContentStore, LocalFsContentStore};
use kx_coordinator::proto::coordinator_server::{Coordinator, CoordinatorServer};
use kx_coordinator::{CoordinatorService, MoteState, WorkerId};
use kx_executor::{LocalResourceManager, MoteExecutor, TestMoteExecutor};
use kx_journal::InMemoryJournal;
use kx_mote::Mote;
use kx_worker::{Worker, WorkerClient};
use tempfile::TempDir;
use tonic::transport::Server;
use tonic::Request;

/// The deterministic result payload worker A's executor publishes for a Mote.
fn result_bytes(mote: &Mote) -> Vec<u8> {
    let mut v = b"kx-result:".to_vec();
    v.extend_from_slice(mote.id.as_bytes());
    v
}

/// A `MoteExecutor` that PUBLISHES its result bytes to the shared store and returns
/// the ref — the correct producer for the PURE path (no R-11 gate; content-addressed
/// so the committed ref == the stored object). Built via the existing public
/// `TestMoteExecutor::new` constructor — kx-executor source is untouched.
fn storing_executor(store: Arc<LocalFsContentStore>) -> Arc<dyn MoteExecutor> {
    Arc::new(TestMoteExecutor::new(move |mote, _warrant| {
        store
            .put(&result_bytes(mote))
            .expect("publish result bytes")
    }))
}

async fn submit(svc: &CoordinatorService, mote: &Mote, warrant: &kx_warrant::WarrantSpec) {
    svc.submit_mote(Request::new(kx_coordinator::proto::SubmitMoteRequest {
        mote: Some(mote.clone().into()),
        warrant: Some(warrant.clone().into()),
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
    panic!("worker connects to the coordinator");
}

/// Spawn an in-process coordinator (built `with_store`) on loopback; return its
/// service clone (for submission + assertions) and the endpoint URL.
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

#[tokio::test]
async fn committed_result_is_readable_by_coordinator_and_peer() {
    let dir = TempDir::new().unwrap();
    let store = Arc::new(LocalFsContentStore::open(dir.path()).unwrap());
    let (svc, endpoint) = spawn_coordinator(store.clone()).await;

    // root -> child PURE DAG.
    let root = common::pure_mote(1, &[]);
    let child = common::pure_mote(2, &[root.id]);
    let warrant = common::pure_warrant();
    submit(&svc, &root, &warrant).await;
    submit(&svc, &child, &warrant).await;

    // Worker A runs + proposes through the storing executor; the coordinator
    // verifies each result_ref is present in the store before committing.
    let mut worker_a = Worker::register(
        connect(&endpoint).await,
        common::WORKER_CLASS,
        "inproc://worker-a",
        storing_executor(store.clone()),
        LocalResourceManager::dev_defaults(),
        store.clone(),
        16,
    )
    .await
    .unwrap();

    assert_eq!(worker_a.run_once().await.unwrap(), 1, "root committed");
    assert_eq!(
        worker_a.run_once().await.unwrap(),
        1,
        "child committed once parent is"
    );
    assert_eq!(svc.committed_count().await.unwrap(), 2);
    assert_eq!(svc.state_of(child.id).await.unwrap(), MoteState::Committed);

    // (a) Readable by the coordinator: it serves the committed result_ref over
    //     ReadEntries, and the bytes resolve from the store it was built with (the
    //     same store.contains check it ran at commit time).
    let mut observer = connect(&endpoint).await;
    let (entries, _next) = observer.read_entries(0, 16).await.unwrap();
    let root_ref = entries
        .iter()
        .find_map(|e| match e.kind.as_ref().unwrap() {
            kx_coordinator::proto::journal_entry::Kind::Committed(c)
                if c.mote_id == root.id.as_bytes().to_vec() =>
            {
                Some(ContentRef::from_bytes(
                    c.result_ref.clone().try_into().unwrap(),
                ))
            }
            _ => None,
        })
        .expect("coordinator serves the root's committed result_ref");
    assert_eq!(
        store.get(&root_ref).unwrap().to_vec(),
        result_bytes(&root),
        "coordinator-visible result_ref resolves to the bytes A published"
    );

    // (b) Readable by another worker: peer B folds the log + reads from the store.
    let mut worker_b = Worker::register(
        connect(&endpoint).await,
        common::WORKER_CLASS,
        "inproc://worker-b",
        storing_executor(store.clone()), // unused by B; B only reads
        LocalResourceManager::dev_defaults(),
        store.clone(),
        16,
    )
    .await
    .unwrap();
    assert_eq!(
        worker_b.peer_read(root.id).await.unwrap(),
        result_bytes(&root),
        "peer worker reads the exact bytes worker A produced"
    );
    assert_eq!(
        worker_b.peer_read(child.id).await.unwrap(),
        result_bytes(&child),
    );
}

#[tokio::test]
async fn phantom_result_ref_is_rejected() {
    let dir = TempDir::new().unwrap();
    let store = Arc::new(LocalFsContentStore::open(dir.path()).unwrap());
    let (svc, endpoint) = spawn_coordinator(store).await;

    let mote = common::pure_mote(9, &[]);
    let warrant = common::pure_warrant();
    submit(&svc, &mote, &warrant).await;

    // A registered worker proposes a result_ref whose bytes were never stored.
    let mut client = connect(&endpoint).await;
    let worker_id = client
        .register_worker(common::WORKER_CLASS, "inproc://phantom")
        .await
        .unwrap();
    let id = mote.id.as_bytes().to_vec();
    let err = client
        .report_commit(kx_coordinator::proto::ReportCommitRequest {
            mote_id: id.clone(),
            idempotency_key: id,
            result_ref: vec![0xAB; 32], // never published to the store
            warrant_ref: vec![4; 32],
            mote_def_hash: vec![5; 32],
            nd_class: kx_coordinator::proto::NdClass::Pure as i32,
            parents: vec![],
            worker_id,
        })
        .await
        .map(|_| ())
        .unwrap_err();
    // ResultRefAbsent maps to INVALID_ARGUMENT (see kx-coordinator error.rs).
    match err {
        kx_worker::WorkerError::Rpc(status) => {
            assert_eq!(status.code(), tonic::Code::InvalidArgument);
        }
        other => panic!("expected an RPC status error, got {other:?}"),
    }
    assert_eq!(
        svc.committed_count().await.unwrap(),
        0,
        "the phantom commit was not recorded"
    );
}

/// P2 EXIT GATE: a two-node (coordinator + ≥2 worker) setup runs the workflow
/// distributed — placement v2 (D56) balances ready Motes across workers of the same
/// class, all commit exactly once, and the executor/inference/scheduler crates are
/// unchanged (thesis test, asserted by the CI diff job, not here).
#[tokio::test]
async fn two_workers_share_the_workload_via_placement() {
    let dir = TempDir::new().unwrap();
    let store = Arc::new(LocalFsContentStore::open(dir.path()).unwrap());
    let (svc, endpoint) = spawn_coordinator(store.clone()).await;

    // Eight independent ready PURE Motes.
    let warrant = common::pure_warrant();
    let motes: Vec<Mote> = (10u8..18).map(|s| common::pure_mote(s, &[])).collect();
    for m in &motes {
        submit(&svc, m, &warrant).await;
    }

    // Two workers of the same class, each leasing small batches.
    let register = |ep: String, tag: &'static str| {
        let store = store.clone();
        async move {
            Worker::register(
                connect(&ep).await,
                common::WORKER_CLASS,
                tag,
                storing_executor(store.clone()),
                LocalResourceManager::dev_defaults(),
                store,
                2,
            )
            .await
            .unwrap()
        }
    };
    let mut worker_a = register(endpoint.clone(), "inproc://a").await;
    let mut worker_b = register(endpoint.clone(), "inproc://b").await;

    // Interleave bounded polls until the DAG drains. Placement routes each worker its
    // sharded Motes first; fill-to-max keeps a poller busy if its shard is empty, so
    // neither starves and both make progress.
    let mut a_total = 0usize;
    let mut b_total = 0usize;
    for _ in 0..16 {
        a_total += worker_a.run_once().await.unwrap();
        b_total += worker_b.run_once().await.unwrap();
        if svc.committed_count().await.unwrap() >= motes.len() {
            break;
        }
    }

    assert_eq!(
        svc.committed_count().await.unwrap(),
        motes.len(),
        "every Mote committed exactly once across the two workers"
    );
    assert!(
        a_total > 0 && b_total > 0,
        "placement shared work across both workers (a={a_total}, b={b_total})"
    );
    assert_eq!(
        a_total + b_total,
        motes.len(),
        "no Mote double-committed (dedup holds)"
    );

    // Load reporting reached the coordinator (run_once heartbeats in_flight).
    let rec_a = svc.registry().get(WorkerId(worker_a.worker_id())).unwrap();
    let rec_b = svc.registry().get(WorkerId(worker_b.worker_id())).unwrap();
    assert!(
        rec_a.last_heartbeat_ms > 0 || rec_b.last_heartbeat_ms > 0,
        "at least one worker reported load via heartbeat during run"
    );

    // Cross-worker read still holds (P2.4): B reads a result regardless of who ran it.
    assert_eq!(
        worker_b.peer_read(motes[0].id).await.unwrap(),
        result_bytes(&motes[0]),
    );
}
