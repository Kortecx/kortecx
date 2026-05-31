//! P2.2 exit-gate proofs (obligation-numbered):
//!
//! - **O1** — the coordinator is the sole journal writer: a worker (gRPC client
//!   with no journal handle) drives a commit *only* through `ReportCommit`, and the
//!   coordinator's own read view reflects it; re-report is an idempotent dedup hit.
//! - **O2** — the worker registry lives behind a trait: distinct ids + heartbeat
//!   tracking through the default impl, and a second `WorkerRegistry` impl proves
//!   the seam.
//! - **O3** — the identity invariant (D53): a bogus wire `mote_id` is ignored; the
//!   coordinator re-derives the canonical id Rust-side.
//! - **O4** — refusals: unknown worker / unknown Mote / bad hash length /
//!   `UNSPECIFIED` enum are rejected with no journal write.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::sync::Arc;
use std::time::Duration;

use kx_coordinator::proto::coordinator_client::CoordinatorClient;
use kx_coordinator::proto::coordinator_server::CoordinatorServer;
use kx_coordinator::proto::{
    self, CommitOutcome, ExecutorClass, HeartbeatRequest, RegisterWorkerRequest, SubmitMoteRequest,
    SubmitStatus,
};
use kx_coordinator::{
    CoordinatorService, InMemoryWorkerRegistry, MoteState, RegistryError, WorkerId, WorkerRecord,
    WorkerRegistry,
};
use kx_journal::InMemoryJournal;
use tonic::transport::{Channel, Server};
use tonic::Code;

/// Spawn the real `CoordinatorServer` over an ephemeral TCP port and return a
/// connected client (the "worker"). The caller keeps the `service` handle for
/// read-side assertions (the client itself never touches the journal).
async fn start(service: CoordinatorService) -> CoordinatorClient<Channel> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);

    tokio::spawn(async move {
        Server::builder()
            .add_service(CoordinatorServer::new(service))
            .serve(addr)
            .await
            .unwrap();
    });

    let endpoint = format!("http://{addr}");
    for _ in 0..100 {
        if let Ok(client) = CoordinatorClient::connect(endpoint.clone()).await {
            return client;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("client never connected to the coordinator");
}

fn register_req(endpoint: &str) -> RegisterWorkerRequest {
    RegisterWorkerRequest {
        executor_class: ExecutorClass::MacosSandbox as i32,
        endpoint: endpoint.into(),
    }
}

/// Register the run through the gRPC client so subsequent `submit_mote` calls
/// pass the M1.3 registration-before-submit gate (idempotent — call once).
async fn register_run(client: &mut CoordinatorClient<Channel>) {
    client
        .register_run(proto::RegisterRunRequest {
            recipe_fingerprint: vec![0x5au8; 32],
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn o1_coordinator_is_sole_journal_writer() {
    let service = CoordinatorService::new(InMemoryJournal::new());
    let mut client = start(service.clone()).await;

    let worker = client
        .register_worker(register_req("unix:///tmp/w0"))
        .await
        .unwrap()
        .into_inner();

    register_run(&mut client).await;
    let mote = common::pure_root_mote();
    let expected_id = mote.id;
    let submit = client
        .submit_mote(SubmitMoteRequest {
            mote: Some(mote.clone().into()),
            warrant: Some(common::sample_warrant().into()),
            accept_at_least_once: false,
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(submit.status, SubmitStatus::Accepted as i32);
    assert_eq!(submit.mote_id, expected_id.as_bytes().to_vec());

    // Nothing committed until the worker reports.
    assert_eq!(service.committed_count().await.unwrap(), 0);
    assert_eq!(
        service.state_of(expected_id).await.unwrap(),
        MoteState::Pending
    );

    let commit = client
        .report_commit(common::report_commit_request(&mote, worker.worker_id))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(commit.outcome, CommitOutcome::Committed as i32);
    assert!(commit.committed_seq >= 1);

    // The commit is visible through the coordinator's own read view — the client
    // has no journal handle; the only path to the log was the RPC (D40).
    assert_eq!(service.committed_count().await.unwrap(), 1);
    assert_eq!(
        service.state_of(expected_id).await.unwrap(),
        MoteState::Committed
    );

    // Idempotent re-report: dedup-by-key hit, same seq, still exactly one.
    let again = client
        .report_commit(common::report_commit_request(&mote, worker.worker_id))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(again.outcome, CommitOutcome::AlreadyCommitted as i32);
    assert_eq!(again.committed_seq, commit.committed_seq);
    assert_eq!(service.committed_count().await.unwrap(), 1);
}

#[tokio::test]
async fn o2_registry_assigns_ids_and_tracks_heartbeat() {
    let service = CoordinatorService::new(InMemoryJournal::new());
    let mut client = start(service.clone()).await;

    let a = client
        .register_worker(register_req("a"))
        .await
        .unwrap()
        .into_inner();
    let b = client
        .register_worker(register_req("b"))
        .await
        .unwrap()
        .into_inner();
    assert_ne!(a.worker_id, b.worker_id);

    let hb = client
        .heartbeat(HeartbeatRequest {
            worker_id: a.worker_id,
            timestamp_ms: 42,
            in_flight: 3,
        })
        .await
        .unwrap()
        .into_inner();
    assert!(hb.ack);

    // Observed through the trait.
    let record = service.registry().get(WorkerId(a.worker_id)).unwrap();
    assert_eq!(record.last_heartbeat_ms, 42);
    assert_eq!(record.in_flight, 3);
    assert_eq!(service.registry().len(), 2);

    // Heartbeat for an unregistered worker is refused.
    let err = client
        .heartbeat(HeartbeatRequest {
            worker_id: 999,
            timestamp_ms: 1,
            in_flight: 0,
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::InvalidArgument);
}

#[tokio::test]
async fn o2_custom_registry_impl_is_the_seam() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct CountingRegistry {
        registers: AtomicUsize,
        inner: InMemoryWorkerRegistry,
    }

    impl WorkerRegistry for CountingRegistry {
        fn register(
            &self,
            executor_class: kx_warrant::ExecutorClass,
            endpoint: String,
        ) -> WorkerId {
            self.registers.fetch_add(1, Ordering::Relaxed);
            self.inner.register(executor_class, endpoint)
        }
        fn heartbeat(
            &self,
            worker: WorkerId,
            now_ms: u64,
            in_flight: u32,
        ) -> Result<(), RegistryError> {
            self.inner.heartbeat(worker, now_ms, in_flight)
        }
        fn get(&self, worker: WorkerId) -> Option<WorkerRecord> {
            self.inner.get(worker)
        }
        fn len(&self) -> usize {
            self.inner.len()
        }
    }

    let registry = Arc::new(CountingRegistry::default());
    let service = CoordinatorService::with_registry(InMemoryJournal::new(), registry.clone());
    let mut client = start(service).await;

    client.register_worker(register_req("x")).await.unwrap();
    assert_eq!(registry.registers.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn o3_mote_id_rederived_server_side() {
    let service = CoordinatorService::new(InMemoryJournal::new());
    let mut client = start(service).await;

    register_run(&mut client).await;
    let mote = common::pure_root_mote();
    let expected_id = mote.id;
    let mut wire: proto::Mote = mote.into();
    wire.mote_id = vec![0u8; 32]; // bogus advisory id

    let submit = client
        .submit_mote(SubmitMoteRequest {
            mote: Some(wire),
            warrant: Some(common::sample_warrant().into()),
            accept_at_least_once: false,
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        submit.mote_id,
        expected_id.as_bytes().to_vec(),
        "MoteId is re-derived Rust-side, not taken from the wire"
    );
}

#[tokio::test]
async fn o4_unknown_worker_rejected_no_write() {
    let service = CoordinatorService::new(InMemoryJournal::new());
    let mut client = start(service.clone()).await;

    // Mote is known (submitted) but no worker registered.
    register_run(&mut client).await;
    let mote = common::pure_root_mote();
    client
        .submit_mote(SubmitMoteRequest {
            mote: Some(mote.clone().into()),
            warrant: Some(common::sample_warrant().into()),
            accept_at_least_once: false,
        })
        .await
        .unwrap();

    let err = client
        .report_commit(common::report_commit_request(&mote, 999))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::InvalidArgument);
    assert_eq!(service.committed_count().await.unwrap(), 0);
}

#[tokio::test]
async fn o4_unknown_mote_rejected_no_write() {
    let service = CoordinatorService::new(InMemoryJournal::new());
    let mut client = start(service.clone()).await;

    let worker = client
        .register_worker(register_req("w"))
        .await
        .unwrap()
        .into_inner();

    // Never submitted.
    let mote = common::pure_root_mote();
    let err = client
        .report_commit(common::report_commit_request(&mote, worker.worker_id))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::InvalidArgument);
    assert_eq!(service.committed_count().await.unwrap(), 0);
}

#[tokio::test]
async fn o4_bad_hash_length_rejected_no_write() {
    let service = CoordinatorService::new(InMemoryJournal::new());
    let mut client = start(service.clone()).await;

    let worker = client
        .register_worker(register_req("w"))
        .await
        .unwrap()
        .into_inner();
    register_run(&mut client).await;
    let mote = common::pure_root_mote();
    client
        .submit_mote(SubmitMoteRequest {
            mote: Some(mote.clone().into()),
            warrant: Some(common::sample_warrant().into()),
            accept_at_least_once: false,
        })
        .await
        .unwrap();

    let mut bad = common::report_commit_request(&mote, worker.worker_id);
    bad.result_ref = vec![3u8; 31]; // not 32 bytes
    let err = client.report_commit(bad).await.unwrap_err();
    assert_eq!(err.code(), Code::InvalidArgument);
    assert_eq!(service.committed_count().await.unwrap(), 0);
}

#[tokio::test]
async fn o4_unspecified_nd_class_rejected_no_write() {
    let service = CoordinatorService::new(InMemoryJournal::new());
    let mut client = start(service.clone()).await;

    let worker = client
        .register_worker(register_req("w"))
        .await
        .unwrap()
        .into_inner();
    register_run(&mut client).await;
    let mote = common::pure_root_mote();
    client
        .submit_mote(SubmitMoteRequest {
            mote: Some(mote.clone().into()),
            warrant: Some(common::sample_warrant().into()),
            accept_at_least_once: false,
        })
        .await
        .unwrap();

    let mut bad = common::report_commit_request(&mote, worker.worker_id);
    bad.nd_class = proto::NdClass::Unspecified as i32; // 0 — the rejected sentinel
    let err = client.report_commit(bad).await.unwrap_err();
    assert_eq!(err.code(), Code::InvalidArgument);
    assert_eq!(service.committed_count().await.unwrap(), 0);
}
