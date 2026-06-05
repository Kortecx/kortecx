//! R5 — the gRPC `StreamEvents` LIVE TAIL (the snapshot-to-head → live-tail
//! upgrade), end-to-end over a real bound port.
//!
//! - `live_tail_stays_open_after_catchup`: the defining property vs. the old
//!   snapshot-to-head — after the catch-up + boundary the stream STAYS OPEN.
//! - `live_tail_delivers_a_commit_to_an_open_stream`: a commit that lands after
//!   subscribe arrives on the SAME open stream (the poller picking up the advance).
//! - `shutdown_does_not_hang_with_an_open_live_stream`: an endless live stream
//!   does not deadlock graceful shutdown (the live-shutdown watch stops it).
//! - `live_stream_ownership_is_uniform_permission_denied`: an unowned run is a
//!   clean pre-stream `permission_denied` (no existence oracle), on the live path.

#![cfg(feature = "embedded-worker")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::HashMap;
use std::time::Duration;

use kx_gateway::start;
use kx_proto::proto;
use tempfile::TempDir;
use tokio::time::timeout;

use common::{await_committed, connect_client, gateway_config, submit_pure_run};

fn open_stream_request(instance_id: &[u8], since_seq: u64) -> proto::StreamEventsRequest {
    proto::StreamEventsRequest {
        instance_id: instance_id.to_vec(),
        since_seq,
    }
}

#[tokio::test]
async fn live_tail_stays_open_after_catchup() {
    let dir = TempDir::new().unwrap();
    let running = start(gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = connect_client(running.local_addr()).await;

    let instance = submit_pure_run(&mut c, 1).await;
    let (mote_id, _) = await_committed(&mut c, &instance).await;

    let mut stream = c
        .stream_events(open_stream_request(&instance, 0))
        .await
        .unwrap()
        .into_inner();

    // Catch-up: read to the boundary, asserting we saw the Committed delta.
    let mut saw_committed = false;
    loop {
        let frame = stream
            .message()
            .await
            .unwrap()
            .expect("a catch-up frame arrives before the stream could ever end");
        for delta in frame.deltas {
            if let Some(proto::event_delta::Kind::Committed(d)) = delta.kind {
                if d.mote_id == mote_id.to_vec() {
                    saw_committed = true;
                }
            }
        }
        if frame.journal_boundary {
            break;
        }
    }
    assert!(saw_committed, "catch-up delivered the Committed delta");

    // THE live-tail property: after the boundary the stream does NOT end — the next
    // read blocks (times out). The old snapshot tailer would return `None` here.
    let next = timeout(Duration::from_millis(500), stream.message()).await;
    assert!(
        next.is_err(),
        "live tail stays open past the boundary (a snapshot stream would end)"
    );

    running.shutdown().await.unwrap();
}

#[tokio::test]
async fn live_tail_delivers_a_commit_to_an_open_stream() {
    let dir = TempDir::new().unwrap();
    let running = start(gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = connect_client(running.local_addr()).await;

    // Subscribe right after submit — the Mote may still be Proposed; the commit
    // lands later and MUST arrive on this already-open stream.
    let instance = submit_pure_run(&mut c, 2).await;
    let mut stream = c
        .stream_events(open_stream_request(&instance, 0))
        .await
        .unwrap()
        .into_inner();

    let saw_committed = timeout(Duration::from_secs(10), async {
        loop {
            match stream.message().await.unwrap() {
                Some(frame) => {
                    if frame
                        .deltas
                        .iter()
                        .any(|d| matches!(d.kind, Some(proto::event_delta::Kind::Committed(_))))
                    {
                        return true;
                    }
                    // Keep reading past boundary frames — the live tail stays open.
                }
                // A snapshot stream would end here before the commit → the test fails.
                None => return false,
            }
        }
    })
    .await
    .expect("did not time out waiting for the live commit");
    assert!(
        saw_committed,
        "the live tail delivered the post-subscribe commit on the open stream"
    );

    running.shutdown().await.unwrap();
}

#[tokio::test]
async fn shutdown_does_not_hang_with_an_open_live_stream() {
    let dir = TempDir::new().unwrap();
    let running = start(gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = connect_client(running.local_addr()).await;
    let instance = submit_pure_run(&mut c, 3).await;
    await_committed(&mut c, &instance).await;

    // Hold an endless live stream open across shutdown; shutdown must still return
    // (the live-shutdown watch stops the poll loop so the graceful drain completes).
    let _stream = c
        .stream_events(open_stream_request(&instance, 0))
        .await
        .unwrap()
        .into_inner();
    timeout(Duration::from_secs(10), running.shutdown())
        .await
        .expect("shutdown did not hang on the open live stream")
        .unwrap();
}

#[tokio::test]
async fn live_stream_ownership_is_uniform_permission_denied() {
    let dir = TempDir::new().unwrap();
    let running = start(gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = connect_client(running.local_addr()).await;

    // An unregistered instance_id is refused with a clean PRE-stream, uniform
    // permission_denied (no existence oracle) — on the live path.
    let err = c
        .stream_events(open_stream_request(&[0u8; 16], 0))
        .await
        .expect_err("an unowned run is refused");
    assert_eq!(err.code(), tonic::Code::PermissionDenied);

    running.shutdown().await.unwrap();
}
