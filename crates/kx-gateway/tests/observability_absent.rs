//! W6.1 — the observability ABSENCE probe: on a build WITHOUT the
//! `observability` feature (the release shape), the three read RPCs answer the
//! seam's designed `unimplemented` and no telemetry.db / alerts.db sidecar is
//! ever created. This file is compiled precisely on the builds the release
//! ships (`not(observability)`), so the default `cargo test --workspace` run
//! and the RC-shaped feature arms both execute it — and on the pre-gating tree
//! (where the views were wired unconditionally) every assertion here fails.

#![cfg(all(feature = "embedded-worker", not(feature = "observability")))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::HashMap;

use kx_gateway::start;
use kx_proto::proto;
use tempfile::TempDir;
use tonic::Code;

use common::{await_committed, connect_client, gateway_config, submit_pure_run};

/// Every file under `dir` named `name`, recursively (the catalog dir resolves
/// beside the journal, so scan the whole sandbox rather than pinning a layout).
fn find_named(dir: &std::path::Path, name: &str, hits: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            find_named(&path, name, hits);
        } else if path.file_name().is_some_and(|f| f == name) {
            hits.push(path);
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_release_shape_is_honestly_inert() {
    let dir = TempDir::new().unwrap();
    let running = start(gateway_config(&dir, true, HashMap::new()))
        .await
        .expect("a build without observability still serves");
    let mut client = connect_client(running.local_addr()).await;

    // Drive one real run to Committed, so "no sidecar" is a statement about a
    // serve that actually executed work — not about an idle process that had
    // nothing to record anyway.
    let instance = submit_pure_run(&mut client, 7).await;
    await_committed(&mut client, &instance).await;

    // The three read RPCs keep the seam's designed degrade: `unimplemented`
    // with the honest "not wired" message, never an empty-but-OK page that
    // would read as "measured nothing".
    let telemetry = client
        .list_mote_telemetry(proto::ListMoteTelemetryRequest::default())
        .await
        .expect_err("ListMoteTelemetry must be unimplemented without the feature");
    assert_eq!(telemetry.code(), Code::Unimplemented, "{telemetry:?}");
    let summary = client
        .list_telemetry_summary(proto::ListTelemetrySummaryRequest::default())
        .await
        .expect_err("ListTelemetrySummary must be unimplemented without the feature");
    assert_eq!(summary.code(), Code::Unimplemented, "{summary:?}");
    let alerts = client
        .list_alerts(proto::ListAlertsRequest::default())
        .await
        .expect_err("ListAlerts must be unimplemented without the feature");
    assert_eq!(alerts.code(), Code::Unimplemented, "{alerts:?}");

    running.shutdown().await.unwrap();

    // No sidecar ever opened: the ledgers are gated out, so the run above must
    // have left neither database anywhere under the sandbox.
    for name in ["telemetry.db", "alerts.db"] {
        let mut hits = Vec::new();
        find_named(dir.path(), name, &mut hits);
        assert!(
            hits.is_empty(),
            "{name} must not exist on a build without observability: {hits:?}"
        );
    }
}
