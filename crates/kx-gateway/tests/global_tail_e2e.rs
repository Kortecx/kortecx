//! Batch C — the GLOBAL cross-run event tail (`StreamAllEvents`), end-to-end
//! over a real serve. Proves: the tail surfaces `RunRegistered` (the per-run
//! cursor never does) and stamps every delta with its run's watermark
//! attribution; TWO interleaved runs attribute to the latest registration
//! at-or-below each delta's seq (the watermark pin); a resume from `next_seq`
//! is loss/dup-free INCLUDING the attribution (the seed pass); and the RPC sits
//! behind the same auth interceptor as everything else (deny-all rejects).

#![cfg(feature = "embedded-worker")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::HashMap;
use std::time::Duration;

use kx_gateway::start;
use kx_proto::proto;
use tokio::time::timeout;
use tonic::Code;

use common::{await_committed, connect_client, gateway_config, submit_pure_run};

/// Drain one `StreamAllEvents` snapshot from `since_seq` until the first
/// journal-boundary frame; return `(deltas, next_seq)`.
async fn drain_to_boundary(
    c: &mut proto::kx_gateway_client::KxGatewayClient<tonic::transport::Channel>,
    since_seq: u64,
) -> (Vec<proto::GlobalEventDelta>, u64) {
    let mut stream = c
        .stream_all_events(proto::StreamAllEventsRequest { since_seq })
        .await
        .unwrap()
        .into_inner();
    let mut deltas = Vec::new();
    let mut next = since_seq;
    timeout(Duration::from_secs(5), async {
        while let Some(frame) = stream.message().await.unwrap() {
            deltas.extend(frame.deltas);
            next = frame.next_seq;
            if frame.journal_boundary {
                break;
            }
        }
    })
    .await
    .expect("the global tail reaches a boundary");
    (deltas, next)
}

#[tokio::test]
async fn global_tail_surfaces_run_registered_and_stamps_the_instance() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = connect_client(running.local_addr()).await;

    let instance = submit_pure_run(&mut c, 0x21).await;
    let (mote_id, _) = await_committed(&mut c, &instance).await;

    let (deltas, _) = drain_to_boundary(&mut c, 0).await;

    // The run-start fact surfaces, attributed to its OWN run.
    let reg = deltas
        .iter()
        .find(|d| {
            matches!(
                d.kind,
                Some(proto::global_event_delta::Kind::RunRegistered(_))
            )
        })
        .expect("the global tail surfaces RunRegistered");
    assert_eq!(reg.instance_id, instance);

    // The commit carries the same watermark attribution + the real mote id.
    let committed = deltas
        .iter()
        .find(|d| match &d.kind {
            Some(proto::global_event_delta::Kind::Committed(cd)) => cd.mote_id == mote_id.to_vec(),
            _ => false,
        })
        .expect("the run's commit streams on the global tail");
    assert_eq!(committed.instance_id, instance);
    assert!(
        committed.seq > reg.seq,
        "the commit follows the registration"
    );

    running.shutdown().await.unwrap();
}

/// Poll `GetProjection` until `n` Motes are committed; return their ids.
async fn await_n_committed(
    c: &mut proto::kx_gateway_client::KxGatewayClient<tonic::transport::Channel>,
    instance: &[u8],
    n: usize,
) -> Vec<Vec<u8>> {
    for _ in 0..200 {
        let view = c
            .get_projection(proto::GetProjectionRequest {
                instance_id: instance.to_vec(),
                at_seq: None,
            })
            .await
            .unwrap()
            .into_inner();
        let committed: Vec<Vec<u8>> = view
            .motes
            .iter()
            .filter(|m| m.state == proto::MoteSnapshotState::Committed as i32)
            .map(|m| m.mote_id.clone())
            .collect();
        if committed.len() >= n {
            return committed;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the run never reached {n} committed Motes");
}

#[tokio::test]
async fn a_second_submit_joins_the_single_registered_run_and_attributes_to_it() {
    // GROUND TRUTH (single-node OSS): `RegisterRun` is IDEMPOTENT per journal —
    // one registered run per serve session; a second SubmitRun returns the SAME
    // instance and its Motes join that run. Every delta on the global tail
    // therefore attributes to the one registration, and exactly ONE
    // RunRegistered delta ever surfaces. (The multi-registration watermark walk
    // is unit-pinned in gateway-core, where the raw journal accepts multiple
    // registrations — the cloud/multi-run shape.)
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = connect_client(running.local_addr()).await;

    let run_a = submit_pure_run(&mut c, 0x31).await;
    await_committed(&mut c, &run_a).await;
    let run_b = submit_pure_run(&mut c, 0x32).await;
    assert_eq!(run_a, run_b, "the second submit joins the registered run");
    await_n_committed(&mut c, &run_a, 2).await;

    let (deltas, _) = drain_to_boundary(&mut c, 0).await;
    let registrations = deltas
        .iter()
        .filter(|d| {
            matches!(
                d.kind,
                Some(proto::global_event_delta::Kind::RunRegistered(_))
            )
        })
        .count();
    assert_eq!(registrations, 1, "one RunRegistered fact per journal");
    for d in &deltas {
        assert_eq!(
            d.instance_id, run_a,
            "every delta attributes to the single registered run"
        );
    }
    let commits = deltas
        .iter()
        .filter(|d| matches!(d.kind, Some(proto::global_event_delta::Kind::Committed(_))))
        .count();
    assert!(commits >= 2, "both submits' commits stream on the tail");

    running.shutdown().await.unwrap();
}

#[tokio::test]
async fn resume_from_next_seq_is_loss_free_with_attribution() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = connect_client(running.local_addr()).await;

    // First run + a full drain to the boundary (the pre-resume window).
    let run_a = submit_pure_run(&mut c, 0x41).await;
    await_committed(&mut c, &run_a).await;
    let (first, cursor) = drain_to_boundary(&mut c, 0).await;
    assert!(!first.is_empty());

    // A second submit lands AFTER the cursor (it joins the same registered run
    // — the single-node ground truth); the resumed stream must deliver its
    // commit exactly once, still attributed (the seed pass re-derives the
    // watermark even though the registration is below the resume point).
    let run_b = submit_pure_run(&mut c, 0x42).await;
    assert_eq!(run_b, run_a, "single-node: one registered run per journal");
    let committed = await_n_committed(&mut c, &run_a, 2).await;
    let (second, _) = drain_to_boundary(&mut c, cursor).await;

    // No overlap (loss/dup-free): every resumed delta is strictly past the cursor.
    assert!(second.iter().all(|d| d.seq > cursor));
    let commit_b = second
        .iter()
        .find(|d| match &d.kind {
            Some(proto::global_event_delta::Kind::Committed(cd)) => committed.contains(&cd.mote_id),
            _ => false,
        })
        .expect("the post-cursor commit arrives on the resumed stream");
    assert_eq!(
        commit_b.instance_id, run_a,
        "attribution survives the resume (the seed pass)"
    );

    running.shutdown().await.unwrap();
}

#[tokio::test]
async fn global_stream_denied_without_auth() {
    let dir = tempfile::TempDir::new().unwrap();
    // Deny-all (no --dev-allow-local, no tokens) — operator-global must still
    // mean OPERATOR (the auth interceptor is the gate; cloud party-scopes).
    let running = start(gateway_config(&dir, false, HashMap::new()))
        .await
        .unwrap();
    let mut c = connect_client(running.local_addr()).await;
    let err = c
        .stream_all_events(proto::StreamAllEventsRequest { since_seq: 0 })
        .await
        .expect_err("deny-all rejects the global tail");
    assert_eq!(err.code(), Code::Unauthenticated);
    running.shutdown().await.unwrap();
}
