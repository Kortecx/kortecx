//! Distributed scaling stress harness (P4.1 scale & performance validation campaign).
//!
//! `#[ignore]`d; run explicitly in RELEASE:
//!
//! ```text
//! cargo test -p kx-coordinator --release --test stress_distributed \
//!     -- --ignored --nocapture --test-threads=1
//! ```
//!
//! **H3** spins up a real `CoordinatorService` behind a real tonic `Server` over
//! an ephemeral loopback TCP port, registers N in-process worker CLIENTS (real
//! gRPC transport), submits M Motes, and has the N workers concurrently report
//! commits. It measures wall-clock + motes/sec per worker-count N in {1,2,4,8}
//! (a scaling curve) and asserts distributed-journal exactly-once
//! (`committed_count == M`, every Mote committed exactly once).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use kx_coordinator::proto::coordinator_server::CoordinatorServer;
use kx_coordinator::CoordinatorService;
use kx_journal::InMemoryJournal;
use kx_mote::{EdgeMeta, GraphPosition, InputDataId, Mote, MoteId, NdClass, ParentRef};
use kx_worker::WorkerClient;
use smallvec::SmallVec;
use tonic::transport::Server;

const WORKER_COUNTS: &[usize] = &[1, 2, 4, 8];
const M: u64 = 2_000;

fn dag_mote(index: u64, parents: &[MoteId]) -> Mote {
    let mut input = [0u8; 32];
    input[..8].copy_from_slice(&index.to_le_bytes());
    let prefs: SmallVec<[ParentRef; 4]> = parents
        .iter()
        .map(|id| ParentRef {
            parent_id: *id,
            edge: EdgeMeta::data(),
        })
        .collect();
    Mote::new(
        common::mote_def(NdClass::Pure),
        InputDataId::from_bytes(input),
        GraphPosition(index.to_le_bytes().to_vec()),
        prefs,
    )
}

async fn spawn_coordinator() -> (CoordinatorService, String) {
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
    (svc, format!("http://{addr}"))
}

async fn connect(endpoint: &str) -> WorkerClient {
    for _ in 0..200 {
        if let Ok(c) = WorkerClient::connect(endpoint.to_string()).await {
            return c;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("worker connects to the coordinator");
}

/// One scaling point: N gRPC workers committing M parentless PURE Motes.
async fn run_point(n_workers: usize) {
    let (svc, endpoint) = spawn_coordinator().await;
    let warrant = common::sample_warrant();

    // Build + submit M parentless ready Motes (submit through the in-process
    // service clone; commits go over real gRPC transport).
    let motes: Arc<Vec<Mote>> = Arc::new((0..M).map(|i| dag_mote(i, &[])).collect());
    for m in motes.iter() {
        let r = common::submit(&svc, m, &warrant).await;
        assert_eq!(
            r.status,
            kx_coordinator::proto::SubmitStatus::Accepted as i32
        );
    }

    // Register N worker clients over gRPC.
    let mut worker_ids = Vec::with_capacity(n_workers);
    let mut clients = Vec::with_capacity(n_workers);
    for w in 0..n_workers {
        let mut client = connect(&endpoint).await;
        let id = client
            .register_worker(common::WORKER_CLASS, &format!("inproc://w{w}"))
            .await
            .unwrap();
        worker_ids.push(id);
        clients.push(client);
    }

    // Shard the Motes across workers by index; each worker reports its shard's
    // commits over its own gRPC channel concurrently.
    let start = Instant::now();
    let mut handles = Vec::with_capacity(n_workers);
    for (w, (mut client, worker_id)) in clients.into_iter().zip(worker_ids).enumerate() {
        let motes = motes.clone();
        handles.push(tokio::spawn(async move {
            for (i, m) in motes.iter().enumerate() {
                if i % n_workers != w {
                    continue;
                }
                let id = m.id.as_bytes().to_vec();
                client
                    .report_commit(kx_coordinator::proto::ReportCommitRequest {
                        mote_id: id.clone(),
                        idempotency_key: id,
                        result_ref: vec![3u8; 32],
                        warrant_ref: vec![4u8; 32],
                        mote_def_hash: vec![5u8; 32],
                        nd_class: kx_coordinator::proto::NdClass::Pure as i32,
                        parents: vec![],
                        worker_id,
                    })
                    .await
                    .unwrap();
            }
        }));
    }
    for h in handles {
        h.await.unwrap();
    }
    let elapsed = start.elapsed();
    let wall_ms = elapsed.as_millis();
    let per_sec = if wall_ms > 0 {
        (M as f64) * 1000.0 / (wall_ms as f64)
    } else {
        f64::INFINITY
    };

    let committed = svc.committed_count().await.unwrap();
    assert_eq!(
        committed as u64, M,
        "distributed exactly-once: every Mote committed once"
    );
    println!(
        "H3 N={n_workers}: M={M} commit_ms={wall_ms} motes/sec={per_sec:.0} \
         transport=grpc exactly-once=ok"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "stress: run with --release --ignored --nocapture --test-threads=1"]
async fn h3_distributed_scaling_curve() {
    for &n in WORKER_COUNTS {
        run_point(n).await;
    }
    println!("H3: transport=grpc exactly-once=ok across N={WORKER_COUNTS:?}");
}
