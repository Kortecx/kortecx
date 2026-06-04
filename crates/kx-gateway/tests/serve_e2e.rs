//! End-to-end witnesses over a REAL bound tonic port — the first real-transport
//! run through a hosted kortecx binary (the architecture audit's #1 gap closed):
//!
//! - `committed_run_is_observable_end_to_end`: a client `SubmitRun`s a PURE
//!   workflow, the embedded worker leases→runs→commits it, and the client sees
//!   it reach `Committed` via `GetProjection`, fetches the deterministic result
//!   via `GetContent`, and resumes the `StreamEvents` cursor.
//! - `deny_all_rejects_every_rpc_without_the_dev_flag`: a port bound WITHOUT
//!   `--dev-allow-local` refuses every RPC (Rule 8c — no silent open door).
//! - `restart_recovers_the_committed_run`: graceful shutdown leaves the journal
//!   at a safe boundary; a fresh server on the same paths re-serves the run.

#![cfg(feature = "embedded-worker")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::net::SocketAddr;
use std::time::Duration;

use kx_gateway::{demo_pure_result, start, GatewayConfig};
use kx_proto::proto;
use kx_proto::proto::kx_gateway_client::KxGatewayClient;
use tempfile::TempDir;
use tonic::transport::Channel;

fn config(dir: &TempDir, dev_allow_local: bool) -> GatewayConfig {
    GatewayConfig {
        listen: "127.0.0.1:0".parse().unwrap(),
        journal_path: dir.path().join("kx.db"),
        content_root: dir.path().join("blobs"),
        max_lease: 16,
        dev_allow_local,
    }
}

async fn client(addr: SocketAddr) -> KxGatewayClient<Channel> {
    let endpoint = format!("http://{addr}");
    for _ in 0..100 {
        if let Ok(c) = KxGatewayClient::connect(endpoint.clone()).await {
            return c;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("client connects to the gateway at {endpoint}");
}

/// Poll `GetProjection` until the run's single Mote is `Committed`; return its
/// (mote_id, result_ref). Fails the test on timeout.
async fn await_committed(
    client: &mut KxGatewayClient<Channel>,
    instance_id: &[u8],
) -> ([u8; 32], [u8; 32]) {
    for _ in 0..100 {
        let view = client
            .get_projection(proto::GetProjectionRequest {
                instance_id: instance_id.to_vec(),
                at_seq: None,
            })
            .await
            .unwrap()
            .into_inner();
        if let Some(m) = view
            .motes
            .iter()
            .find(|m| m.state == proto::MoteSnapshotState::Committed as i32)
        {
            let mote_id: [u8; 32] = m.mote_id.clone().try_into().unwrap();
            let result_ref: [u8; 32] = m
                .result_ref
                .clone()
                .expect("a committed Mote carries a result_ref")
                .try_into()
                .unwrap();
            return (mote_id, result_ref);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the submitted Mote never reached Committed");
}

#[tokio::test]
async fn committed_run_is_observable_end_to_end() {
    let dir = TempDir::new().unwrap();
    let running = start(config(&dir, true)).await.unwrap();
    let mut c = client(running.local_addr()).await;

    // (1) SubmitRun a single PURE Mote.
    let mote = common::pure_mote(1, &[]);
    let warrant = common::pure_warrant();
    let handle = c
        .submit_run(proto::SubmitRunRequest {
            recipe_fingerprint: vec![0x5a; 32],
            motes: vec![proto::SubmitMoteSpec {
                mote: Some(mote.into()),
                warrant: Some(warrant.into()),
                accept_at_least_once: false,
            }],
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(handle.instance_id.len(), 16, "journaled instance_id is 16B");

    // (2) The embedded worker drives it to Committed; the client observes it.
    let (mote_id, result_ref) = await_committed(&mut c, &handle.instance_id).await;

    // (3) GetContent returns the deterministic result the executor published.
    let blob = c
        .get_content(proto::GetContentRequest {
            content_ref: result_ref.to_vec(),
            instance_id: handle.instance_id.clone(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        blob.payload,
        demo_pure_result(&mote_id),
        "GetContent returns the bytes the embedded worker committed"
    );

    // (4) StreamEvents carries a Committed delta for the Mote, resumably.
    let mut stream = c
        .stream_events(proto::StreamEventsRequest {
            instance_id: handle.instance_id.clone(),
            since_seq: 0,
        })
        .await
        .unwrap()
        .into_inner();
    let mut saw_committed = false;
    let mut last_next_seq = 0u64;
    while let Some(frame) = stream.message().await.unwrap() {
        last_next_seq = frame.next_seq;
        for delta in frame.deltas {
            if let Some(proto::event_delta::Kind::Committed(c)) = delta.kind {
                if c.mote_id == mote_id.to_vec() {
                    saw_committed = true;
                }
            }
        }
        if frame.journal_boundary {
            break;
        }
    }
    assert!(saw_committed, "StreamEvents reported the Committed delta");

    // Resuming from the caught-up cursor yields no earlier events (never < cursor).
    let mut resumed = c
        .stream_events(proto::StreamEventsRequest {
            instance_id: handle.instance_id.clone(),
            since_seq: last_next_seq,
        })
        .await
        .unwrap()
        .into_inner();
    while let Some(frame) = resumed.message().await.unwrap() {
        assert!(
            frame.next_seq >= last_next_seq,
            "a resumed cursor never rewinds below the ack point"
        );
        if frame.journal_boundary {
            break;
        }
    }

    running.shutdown().await.unwrap();
}

#[tokio::test]
async fn deny_all_rejects_every_rpc_without_the_dev_flag() {
    let dir = TempDir::new().unwrap();
    let running = start(config(&dir, false)).await.unwrap();
    let mut c = client(running.local_addr()).await;

    // A bound port with no --dev-allow-local is a closed door (Rule 8c).
    let submit = c
        .submit_run(proto::SubmitRunRequest {
            recipe_fingerprint: vec![0x5a; 32],
            motes: vec![],
        })
        .await;
    assert_eq!(
        submit.unwrap_err().code(),
        tonic::Code::Unauthenticated,
        "SubmitRun is denied"
    );

    let projection = c
        .get_projection(proto::GetProjectionRequest {
            instance_id: vec![0u8; 16],
            at_seq: None,
        })
        .await;
    assert_eq!(
        projection.unwrap_err().code(),
        tonic::Code::Unauthenticated,
        "GetProjection is denied"
    );

    running.shutdown().await.unwrap();
}

#[tokio::test]
async fn restart_recovers_the_committed_run() {
    let dir = TempDir::new().unwrap();

    // First server: submit + drive to Committed, then shut down gracefully.
    let instance_id = {
        let running = start(config(&dir, true)).await.unwrap();
        let mut c = client(running.local_addr()).await;
        let handle = c
            .submit_run(proto::SubmitRunRequest {
                recipe_fingerprint: vec![0x5a; 32],
                motes: vec![proto::SubmitMoteSpec {
                    mote: Some(common::pure_mote(7, &[]).into()),
                    warrant: Some(common::pure_warrant().into()),
                    accept_at_least_once: false,
                }],
            })
            .await
            .unwrap()
            .into_inner();
        await_committed(&mut c, &handle.instance_id).await;
        running.shutdown().await.unwrap(); // graceful: returns Ok
        handle.instance_id
    };

    // Second server on the SAME journal + content: the committed run is recovered
    // from the durable log and re-served read-only (no re-execution needed).
    let running = start(config(&dir, true)).await.unwrap();
    let mut c = client(running.local_addr()).await;
    let view = c
        .get_projection(proto::GetProjectionRequest {
            instance_id: instance_id.clone(),
            at_seq: None,
        })
        .await
        .unwrap()
        .into_inner();
    assert!(
        view.motes
            .iter()
            .any(|m| m.state == proto::MoteSnapshotState::Committed as i32),
        "the committed run survives a restart (durable journal)"
    );
    running.shutdown().await.unwrap();
}
