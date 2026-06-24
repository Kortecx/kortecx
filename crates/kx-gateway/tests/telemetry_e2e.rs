//! Batch C — mote execution telemetry (`telemetry.db` + `ListMoteTelemetry`),
//! end-to-end over a real serve. Proves: an executed-then-committed mote gets a
//! JOINED row (seq + watermark instance stamped; the FFI-free degrade keeps
//! `model_id` empty and the token counts ABSENT — the row never claims a model
//! ran on an echo path); filters + `before_seq` pagination page newest-first;
//! rows survive a restart (same sidecar, same schema); and the RPC sits behind
//! the auth interceptor.

#![cfg(feature = "embedded-worker")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::HashMap;
use std::time::Duration;

use kx_gateway::start;
use kx_proto::proto;
use tonic::Code;

use common::{await_committed, connect_client, gateway_config, submit_pure_run};

/// Poll `ListMoteTelemetry` until at least `n` joined rows exist (the join tick
/// runs every ~250 ms). Returns the rows (newest-first).
async fn await_rows(
    c: &mut proto::kx_gateway_client::KxGatewayClient<tonic::transport::Channel>,
    req: proto::ListMoteTelemetryRequest,
    n: usize,
) -> Vec<proto::MoteTelemetryRow> {
    for _ in 0..120 {
        let resp = c
            .list_mote_telemetry(req.clone())
            .await
            .unwrap()
            .into_inner();
        if resp.rows.len() >= n {
            return resp.rows;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("telemetry.db never reached {n} joined rows");
}

/// Poll until the SPECIFIC mote's joined row exists. POC-5c CI-hardening: a bare
/// `await_rows(.., 1)` returns as soon as ANY one row joins, which RACES a
/// multi-mote run where a different mote's row joins first (the telemetry join
/// tick lags the Committed fact) — `.find(mote_id)` would then miss the executed
/// mote's row. Scanning every row (no filter dependency) until the target appears
/// is order-independent and closes the flake.
async fn await_row_for_mote(
    c: &mut proto::kx_gateway_client::KxGatewayClient<tonic::transport::Channel>,
    mote_id: &[u8],
) -> proto::MoteTelemetryRow {
    for _ in 0..120 {
        let resp = c
            .list_mote_telemetry(all_rows())
            .await
            .unwrap()
            .into_inner();
        if let Some(row) = resp.rows.into_iter().find(|r| r.mote_id == mote_id) {
            return row;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("telemetry.db never joined a row for the executed mote");
}

fn all_rows() -> proto::ListMoteTelemetryRequest {
    proto::ListMoteTelemetryRequest {
        limit: None,
        instance_id: None,
        mote_id: None,
        before_seq: None,
    }
}

#[tokio::test]
async fn an_executed_mote_gets_a_joined_honest_row() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = connect_client(running.local_addr()).await;

    let instance = submit_pure_run(&mut c, 0x61).await;
    let (mote_id, _) = await_committed(&mut c, &instance).await;

    let row = await_row_for_mote(&mut c, &mote_id).await;

    // Joined: the Committed fact's seq + the watermark instance are stamped.
    assert!(row.seq > 0);
    assert_eq!(row.instance_id, instance);
    assert!(
        row.started_unix_ms > 0,
        "the host stamped a start wall clock"
    );
    // The FFI-free degrade is HONEST: no model ran, so no model fields — the
    // row never falls back to a def's model id, and the token counts stay
    // absent (`input_tokens` is absent in OSS by design).
    assert_eq!(row.model_id, "");
    assert_eq!(row.output_tokens, None);
    assert_eq!(row.input_tokens, None);

    running.shutdown().await.unwrap();
}

#[tokio::test]
async fn filters_and_before_seq_paginate_newest_first() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = connect_client(running.local_addr()).await;

    // Three runs ⇒ three committed motes ⇒ three joined rows.
    let mut instances = Vec::new();
    for seed in [0x71u8, 0x72, 0x73] {
        let instance = submit_pure_run(&mut c, seed).await;
        await_committed(&mut c, &instance).await;
        instances.push(instance);
    }
    let rows = await_rows(&mut c, all_rows(), 3).await;
    assert!(rows.len() >= 3);
    // Newest-first (strictly descending seq).
    assert!(rows.windows(2).all(|w| w[0].seq > w[1].seq));

    // The instance filter scopes to one run's row(s).
    let scoped = c
        .list_mote_telemetry(proto::ListMoteTelemetryRequest {
            instance_id: Some(instances[1].clone()),
            ..all_rows()
        })
        .await
        .unwrap()
        .into_inner();
    assert!(!scoped.rows.is_empty());
    assert!(scoped.rows.iter().all(|r| r.instance_id == instances[1]));

    // The mote filter pins exactly one row.
    let one_mote = rows[0].mote_id.clone();
    let by_mote = c
        .list_mote_telemetry(proto::ListMoteTelemetryRequest {
            mote_id: Some(one_mote.clone()),
            ..all_rows()
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(by_mote.rows.len(), 1);
    assert_eq!(by_mote.rows[0].mote_id, one_mote);

    // before_seq pages walk to exhaustion without dup or miss.
    let mut walked: Vec<u64> = Vec::new();
    let mut cursor: Option<u64> = None;
    loop {
        let page = c
            .list_mote_telemetry(proto::ListMoteTelemetryRequest {
                limit: Some(1),
                before_seq: cursor,
                ..all_rows()
            })
            .await
            .unwrap()
            .into_inner();
        let Some(last) = page.rows.last() else { break };
        walked.extend(page.rows.iter().map(|r| r.seq));
        cursor = Some(last.seq);
        if !page.has_more {
            break;
        }
    }
    assert!(walked.len() >= 3);
    assert!(
        walked.windows(2).all(|w| w[0] > w[1]),
        "no dup/miss: {walked:?}"
    );

    running.shutdown().await.unwrap();
}

#[tokio::test]
async fn telemetry_rows_survive_a_restart() {
    let dir = tempfile::TempDir::new().unwrap();
    let count_before;
    {
        let running = start(gateway_config(&dir, true, HashMap::new()))
            .await
            .unwrap();
        let mut c = connect_client(running.local_addr()).await;
        let instance = submit_pure_run(&mut c, 0x81).await;
        await_committed(&mut c, &instance).await;
        count_before = await_rows(&mut c, all_rows(), 1).await.len();
        running.shutdown().await.unwrap();
    }
    // Same dirs ⇒ the sidecar reopens at the same schema and keeps its rows
    // (rebuild-to-empty fires only on a schema bump or corruption).
    let running = start(gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = connect_client(running.local_addr()).await;
    let rows = await_rows(&mut c, all_rows(), count_before).await;
    assert!(rows.len() >= count_before, "rows survived the restart");
    running.shutdown().await.unwrap();
}

#[tokio::test]
async fn summary_matches_the_sum_over_list_pages() {
    // W1a-3 cross-surface invariant: the server-side rollup MUST equal the client
    // fold over every ListMoteTelemetry page (no drift). On an FFI-free serve there
    // are no model motes, so the per-model rows are empty and output tokens are 0 —
    // but total_motes must equal the joined-row count (the honest "all runs" total).
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = connect_client(running.local_addr()).await;

    for seed in [0x91u8, 0x92, 0x93] {
        let instance = submit_pure_run(&mut c, seed).await;
        await_committed(&mut c, &instance).await;
    }
    let rows = await_rows(&mut c, all_rows(), 3).await;

    let summary = c
        .list_telemetry_summary(proto::ListTelemetrySummaryRequest { instance_id: None })
        .await
        .unwrap()
        .into_inner();

    // total_motes == the count of joined rows; output tokens == their sum (0 here).
    let page_token_sum: u64 = rows.iter().filter_map(|r| r.output_tokens).sum();
    assert_eq!(summary.total_motes as usize, rows.len());
    assert_eq!(summary.total_output_tokens, page_token_sum);
    // FFI-free: no model ran, so no per-model row is fabricated.
    assert!(
        summary.rows.is_empty(),
        "no model motes ⇒ no per-model rows"
    );

    running.shutdown().await.unwrap();
}

#[tokio::test]
async fn telemetry_denied_without_auth() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(gateway_config(&dir, false, HashMap::new()))
        .await
        .unwrap();
    let mut c = connect_client(running.local_addr()).await;
    let err = c
        .list_mote_telemetry(all_rows())
        .await
        .expect_err("deny-all rejects telemetry reads");
    assert_eq!(err.code(), Code::Unauthenticated);
    // The W1a-3 summary RPC sits behind the same auth interceptor.
    let err = c
        .list_telemetry_summary(proto::ListTelemetrySummaryRequest { instance_id: None })
        .await
        .expect_err("deny-all rejects the telemetry summary too");
    assert_eq!(err.code(), Code::Unauthenticated);
    running.shutdown().await.unwrap();
}
