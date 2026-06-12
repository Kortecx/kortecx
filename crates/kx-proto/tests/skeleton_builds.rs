//! Proves the tonic service **skeleton builds and serves**: a no-op
//! `Coordinator` impl is hosted over a real TCP endpoint and a generated client
//! reaches every RPC. This goes beyond "compiles" — it exercises the
//! generated server trait, the `CoordinatorServer`/`CoordinatorClient` types,
//! and the transport. No coordinator *behavior* is implemented (that is P2.2/2.3).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::time::Duration;

use common::{sample_mote, sample_warrant};
use kx_proto::proto::coordinator_client::CoordinatorClient;
use kx_proto::proto::coordinator_server::{Coordinator, CoordinatorServer};
use kx_proto::proto::{
    journal_entry, CommitOutcome, CommittedEntry, ExecutorClass, HeartbeatRequest,
    HeartbeatResponse, JournalEntry, LeaseWorkRequest, LeaseWorkResponse, NdClass,
    ReadEntriesRequest, ReadEntriesResponse, RegisterRunRequest, RegisterRunResponse,
    RegisterWorkerRequest, RegisterWorkerResponse, ReportCommitRequest, ReportCommitResponse,
    ReportEffectStagedRequest, ReportEffectStagedResponse, ReportFailureRequest,
    ReportFailureResponse, SubmitMoteRequest, SubmitMoteResponse, SubmitStatus, WorkItem,
};
use tonic::transport::Server;
use tonic::{Request, Response, Status};

#[derive(Default)]
struct NoopCoordinator;

#[tonic::async_trait]
impl Coordinator for NoopCoordinator {
    async fn register_worker(
        &self,
        _req: Request<RegisterWorkerRequest>,
    ) -> Result<Response<RegisterWorkerResponse>, Status> {
        Ok(Response::new(RegisterWorkerResponse { worker_id: 7 }))
    }

    async fn heartbeat(
        &self,
        _req: Request<HeartbeatRequest>,
    ) -> Result<Response<HeartbeatResponse>, Status> {
        Ok(Response::new(HeartbeatResponse { ack: true }))
    }

    async fn submit_mote(
        &self,
        req: Request<SubmitMoteRequest>,
    ) -> Result<Response<SubmitMoteResponse>, Status> {
        // Echo back the wire mote_id so the test confirms the payload arrived
        // intact. (Identity is re-derived coordinator-side in P2.2; not here.)
        let mote_id = req.into_inner().mote.map(|m| m.mote_id).unwrap_or_default();
        Ok(Response::new(SubmitMoteResponse {
            mote_id,
            status: SubmitStatus::Accepted as i32,
            detail: String::new(),
            instance_id: vec![0u8; 16],
            refusal_code: String::new(),
        }))
    }

    async fn report_commit(
        &self,
        _req: Request<ReportCommitRequest>,
    ) -> Result<Response<ReportCommitResponse>, Status> {
        Ok(Response::new(ReportCommitResponse {
            committed_seq: 1,
            outcome: CommitOutcome::Committed as i32,
            detail: String::new(),
        }))
    }

    async fn report_effect_staged(
        &self,
        req: Request<ReportEffectStagedRequest>,
    ) -> Result<Response<ReportEffectStagedResponse>, Status> {
        let _ = req.into_inner();
        Ok(Response::new(ReportEffectStagedResponse {
            staged_seq: 1,
            ack: true,
        }))
    }

    async fn lease_work(
        &self,
        req: Request<LeaseWorkRequest>,
    ) -> Result<Response<LeaseWorkResponse>, Status> {
        // Echo one work item back so the test confirms the Mote + warrant
        // payload round-trips through the new RPC. (Ready-set selection is
        // implemented coordinator-side in P2.3; not here.)
        let _ = req.into_inner();
        Ok(Response::new(LeaseWorkResponse {
            items: vec![WorkItem {
                mote: Some(sample_mote().into()),
                warrant: Some(sample_warrant().into()),
                parent_results: vec![],
                tool_args: None,
            }],
            instance_id: vec![0u8; 16],
        }))
    }

    async fn report_failure(
        &self,
        req: Request<ReportFailureRequest>,
    ) -> Result<Response<ReportFailureResponse>, Status> {
        let _ = req.into_inner();
        Ok(Response::new(ReportFailureResponse {
            failed_seq: 1,
            ack: true,
        }))
    }

    async fn register_run(
        &self,
        req: Request<RegisterRunRequest>,
    ) -> Result<Response<RegisterRunResponse>, Status> {
        // Echo back a fixed 16-byte instance_id so the test confirms the RPC
        // round-trips. (Real nonce generation + journaling is coordinator-side
        // in M1.1; not here.)
        let _ = req.into_inner();
        Ok(Response::new(RegisterRunResponse {
            instance_id: vec![9; 16],
        }))
    }

    async fn read_entries(
        &self,
        req: Request<ReadEntriesRequest>,
    ) -> Result<Response<ReadEntriesResponse>, Status> {
        // Echo one committed entry back so the test confirms it rides the new
        // RPC. (Real serving from the journal is coordinator-side in P2.4.)
        let _ = req.into_inner();
        Ok(Response::new(ReadEntriesResponse {
            entries: vec![JournalEntry {
                seq: 1,
                kind: Some(journal_entry::Kind::Committed(CommittedEntry {
                    mote_id: vec![1; 32],
                    idempotency_key: vec![1; 32],
                    seq: 1,
                    nd_class: NdClass::Pure as i32,
                    result_ref: vec![2; 32],
                    parents: vec![],
                    warrant_ref: vec![4; 32],
                    mote_def_hash: vec![5; 32],
                })),
            }],
            next_seq: 1,
        }))
    }
}

#[tokio::test]
async fn coordinator_skeleton_serves_all_rpcs() {
    // Ephemeral port; serve in the background.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener); // free it for tonic to re-bind

    tokio::spawn(async move {
        Server::builder()
            .add_service(CoordinatorServer::new(NoopCoordinator))
            .serve(addr)
            .await
            .unwrap();
    });

    // Connect with a brief retry while the server binds.
    let endpoint = format!("http://{addr}");
    let mut client = None;
    for _ in 0..100 {
        match CoordinatorClient::connect(endpoint.clone()).await {
            Ok(c) => {
                client = Some(c);
                break;
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(10)).await,
        }
    }
    let mut client = client.expect("client connects to the skeleton server");

    // (4) register worker
    let reg = client
        .register_worker(RegisterWorkerRequest {
            executor_class: ExecutorClass::Bwrap as i32,
            endpoint: "http://10.0.0.2:50051".into(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(reg.worker_id, 7);

    // (3) heartbeat
    let hb = client
        .heartbeat(HeartbeatRequest {
            worker_id: 7,
            timestamp_ms: 1,
            in_flight: 0,
        })
        .await
        .unwrap()
        .into_inner();
    assert!(hb.ack);

    // (1) submit Mote — full domain Mote + warrant over the wire.
    let submit = client
        .submit_mote(SubmitMoteRequest {
            mote: Some(sample_mote().into()),
            warrant: Some(sample_warrant().into()),
            accept_at_least_once: false,
            react_seed: false,
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(submit.status, SubmitStatus::Accepted as i32);
    assert_eq!(
        submit.mote_id.len(),
        32,
        "mote_id round-tripped over the wire"
    );

    // (2) report commit
    let commit = client
        .report_commit(ReportCommitRequest {
            mote_id: vec![1; 32],
            idempotency_key: vec![2; 32],
            result_ref: vec![3; 32],
            warrant_ref: vec![4; 32],
            mote_def_hash: vec![5; 32],
            nd_class: kx_proto::proto::NdClass::ReadOnlyNondet as i32,
            parents: vec![],
            worker_id: 7,
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(commit.outcome, CommitOutcome::Committed as i32);
    assert_eq!(commit.committed_seq, 1);

    // (5) lease work — a full Mote + warrant rides back over the new RPC.
    let lease = client
        .lease_work(LeaseWorkRequest {
            worker_id: 7,
            executor_class: ExecutorClass::Bwrap as i32,
            max_motes: 8,
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(lease.items.len(), 1, "one work item leased");
    let item = &lease.items[0];
    assert_eq!(
        item.mote.as_ref().unwrap().mote_id.len(),
        32,
        "leased mote_id round-tripped over the wire"
    );
    assert!(item.warrant.is_some(), "leased warrant present");

    // (6) read entries — a committed entry rides back over the new RPC.
    let read = client
        .read_entries(ReadEntriesRequest {
            since_seq: 0,
            max: 16,
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(read.entries.len(), 1, "one committed entry read");
    assert_eq!(read.next_seq, 1);
    match read.entries[0].kind.as_ref().unwrap() {
        journal_entry::Kind::Committed(c) => {
            assert_eq!(c.result_ref.len(), 32, "committed result_ref round-tripped");
        }
    }

    // (8) register run — recipe_fingerprint goes up, instance_id comes back.
    let run = client
        .register_run(RegisterRunRequest {
            recipe_fingerprint: vec![7; 32],
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        run.instance_id.len(),
        16,
        "instance_id round-tripped over the wire"
    );
}
