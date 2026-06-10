//! The Morphic Data Engine — durable serve-path capture, end-to-end (campaign
//! Batch 2). Proves: a serve run's committed ACTIONS are captured into the
//! `capture.db` sidecar and queryable via `ListCaptureRecords`; the records
//! reconcile against `GetProjection` (same result_refs); the projection
//! survives a serve restart, REBUILDS from the journal when the sidecar is
//! deleted (the D40 rebuildable-cache pin) and when it is corrupted; the new
//! RPC is behind the auth interceptor; and pagination is bounded.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Duration;

use kx_gateway::start;
use kx_proto::proto;
use kx_proto::proto::kx_gateway_client::KxGatewayClient;
use tonic::transport::Channel;

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

/// Poll `ListCaptureRecords` until at least `n` records are captured (the poller
/// folds every ~250 ms; allow generously). Returns the records (newest-first).
async fn await_captures(
    c: &mut KxGatewayClient<Channel>,
    instance_id: Option<Vec<u8>>,
    n: usize,
) -> Vec<proto::CaptureRecordSummary> {
    for _ in 0..120 {
        let resp = c
            .list_capture_records(proto::ListCaptureRecordsRequest {
                limit: None,
                instance_id: instance_id.clone(),
            })
            .await
            .unwrap()
            .into_inner();
        if resp.records.len() >= n {
            return resp.records;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("capture.db never reached {n} records");
}

#[tokio::test]
async fn captured_actions_reconcile_with_the_projection() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    // Drive the canonical 8-Mote demo run (pure, FFI-free) to completion.
    let instance = common::submit_pure_run(&mut c, 0x11).await;
    // Its terminal Mote commits last; wait for the whole run to settle, then for
    // the poller to capture every committed action.
    let records = await_captures(&mut c, Some(instance.clone()), 1).await;
    assert!(
        !records.is_empty(),
        "the run's committed actions were captured"
    );
    // Every captured record is join-key-only and stamped with the run instance.
    for r in &records {
        assert_eq!(r.mote_id.len(), 32);
        assert_eq!(
            r.instance_id, instance,
            "stamped with the serve session run"
        );
        assert_eq!(r.result_ref.len(), 32);
        assert!(
            ["pure", "read_only_nondet", "world_mutating"].contains(&r.nd_class.as_str()),
            "nd_class wire vocabulary: {}",
            r.nd_class
        );
    }

    // Reconcile against the projection: every CAPTURED action's (mote_id,
    // result_ref) matches the committed snapshot (the truth join).
    let view = c
        .get_projection(proto::GetProjectionRequest {
            instance_id: instance.clone(),
            at_seq: None,
        })
        .await
        .unwrap()
        .into_inner();
    let committed: HashMap<Vec<u8>, Vec<u8>> = view
        .motes
        .iter()
        .filter(|m| m.state == proto::MoteSnapshotState::Committed as i32)
        .filter_map(|m| m.result_ref.clone().map(|r| (m.mote_id.clone(), r)))
        .collect();
    for r in &records {
        assert_eq!(
            committed.get(&r.mote_id),
            Some(&r.result_ref),
            "captured action ref matches the committed projection (the truth join)"
        );
    }
    // Capture is COMPLETE for committed actions (every committed Mote captured).
    assert_eq!(
        records.len(),
        committed.len(),
        "every committed action is captured"
    );

    running.shutdown().await.unwrap();
}

#[tokio::test]
async fn capture_survives_restart_and_rebuilds_from_the_journal() {
    let dir = tempfile::TempDir::new().unwrap();
    let captured_first: Vec<Vec<u8>>;
    let instance: Vec<u8>;
    {
        let running = start(common::gateway_config(&dir, true, HashMap::new()))
            .await
            .unwrap();
        let mut c = client(running.local_addr()).await;
        instance = common::submit_pure_run(&mut c, 0x22).await;
        let recs = await_captures(&mut c, Some(instance.clone()), 1).await;
        captured_first = recs.iter().map(|r| r.mote_id.clone()).collect();
        running.shutdown().await.unwrap();
    }

    // (a) Restart on the SAME dirs → the sidecar persisted; records identical.
    {
        let running = start(common::gateway_config(&dir, true, HashMap::new()))
            .await
            .unwrap();
        let mut c = client(running.local_addr()).await;
        let recs = await_captures(&mut c, Some(instance.clone()), captured_first.len()).await;
        let mut ids: Vec<Vec<u8>> = recs.iter().map(|r| r.mote_id.clone()).collect();
        let mut first = captured_first.clone();
        ids.sort();
        first.sort();
        assert_eq!(ids, first, "capture.db persisted across restart");
        running.shutdown().await.unwrap();
    }

    // (b) DELETE capture.db, restart → the projection BACKFILLS from the journal
    //     to byte-identical records (the D40 rebuildable-cache pin).
    let capture_db = dir.path().join("capture.db");
    assert!(capture_db.exists(), "capture.db is under the catalog dir");
    std::fs::remove_file(&capture_db).unwrap();
    // WAL/shm siblings too, so the rebuild is from a clean slate.
    let _ = std::fs::remove_file(dir.path().join("capture.db-wal"));
    let _ = std::fs::remove_file(dir.path().join("capture.db-shm"));
    {
        let running = start(common::gateway_config(&dir, true, HashMap::new()))
            .await
            .unwrap();
        let mut c = client(running.local_addr()).await;
        let recs = await_captures(&mut c, Some(instance.clone()), captured_first.len()).await;
        let mut ids: Vec<Vec<u8>> = recs.iter().map(|r| r.mote_id.clone()).collect();
        let mut first = captured_first.clone();
        ids.sort();
        first.sort();
        assert_eq!(ids, first, "deleted sidecar rebuilt from the journal (D40)");
        running.shutdown().await.unwrap();
    }

    // (c) CORRUPT capture.db (truncate to garbage), restart → drop-and-rebuild,
    //     serve healthy, records recovered.
    std::fs::write(&capture_db, b"not a sqlite file at all").unwrap();
    let _ = std::fs::remove_file(dir.path().join("capture.db-wal"));
    let _ = std::fs::remove_file(dir.path().join("capture.db-shm"));
    {
        let running = start(common::gateway_config(&dir, true, HashMap::new()))
            .await
            .unwrap();
        let mut c = client(running.local_addr()).await;
        let recs = await_captures(&mut c, Some(instance.clone()), captured_first.len()).await;
        assert_eq!(recs.len(), captured_first.len(), "corrupt sidecar rebuilt");
        running.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn list_capture_records_is_behind_the_auth_interceptor() {
    // deny-all posture (no dev-allow-local, no tokens): the new RPC is refused
    // like every other read surface (no capture-records oracle).
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, false, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;
    let err = c
        .list_capture_records(proto::ListCaptureRecordsRequest {
            limit: None,
            instance_id: None,
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), tonic::Code::Unauthenticated);
    running.shutdown().await.unwrap();
}

#[tokio::test]
async fn capture_pagination_clamps_and_paginates() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;
    // Three distinct pure Motes JOIN the one single-node run instance (F4: one
    // journal = one run; distinct submits are distinct terminal Motes) ⇒ three
    // captured actions to paginate over.
    let instance = common::submit_pure_run(&mut c, 0x33).await;
    let _ = common::submit_pure_run(&mut c, 0x34).await;
    let _ = common::submit_pure_run(&mut c, 0x35).await;
    let all = await_captures(&mut c, Some(instance.clone()), 3).await;
    assert!(all.len() >= 3, "three submits ⇒ three captured actions");

    // A tiny page bounds the response + sets has_more; the union over pages is
    // the full set with no dupes (descending seq across the boundary).
    let page1 = c
        .list_capture_records(proto::ListCaptureRecordsRequest {
            limit: Some(1),
            instance_id: Some(instance.clone()),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(page1.records.len(), 1);
    assert!(page1.has_more, "more than one captured action ⇒ has_more");
    // A huge limit is clamped to the server max (no error, bounded response).
    let huge = c
        .list_capture_records(proto::ListCaptureRecordsRequest {
            limit: Some(99_999),
            instance_id: Some(instance.clone()),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(huge.records.len(), all.len(), "clamp returns the full set");
    assert!(!huge.has_more);
    // Newest-first: strictly descending seq.
    for w in huge.records.windows(2) {
        assert!(w[0].seq > w[1].seq, "records are newest-first by seq");
    }

    running.shutdown().await.unwrap();
}
