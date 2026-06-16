//! W1a-2 — the operator alerts inbox (`alerts.db` + `ListAlerts`), end-to-end
//! over a real serve. Proves: a HEALTHY run produces NO alert (the "System is
//! healthy" empty state is REAL, never a fabricated row — GR15); the RPC pages
//! honestly (`has_more = false` on an empty inbox); and it sits behind the auth
//! interceptor. The fold's terminal-`Failed` filter + the re-fold-stable identity
//! (the HARD-gate determinism) are proven exhaustively in the `alerts` unit
//! tests over seeded `Failed` facts — the serve path cannot synthesize a terminal
//! dead-letter without a failing worker, so the e2e asserts the honest-empty,
//! pagination, and auth contracts here.

#![cfg(feature = "embedded-worker")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::HashMap;

use kx_gateway::start;
use kx_proto::proto;
use tonic::Code;

use common::{await_committed, connect_client, gateway_config, submit_pure_run};

fn all_alerts() -> proto::ListAlertsRequest {
    proto::ListAlertsRequest {
        limit: None,
        instance_id: None,
        before_seq: None,
    }
}

#[tokio::test]
async fn a_healthy_run_produces_no_alerts() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = connect_client(running.local_addr()).await;

    // A pure run that COMMITS cleanly — no terminal Failed fact is journaled, so
    // the inbox stays honestly empty (the "System is healthy" UI state is real).
    let instance = submit_pure_run(&mut c, 0x61).await;
    await_committed(&mut c, &instance).await;

    let resp = c.list_alerts(all_alerts()).await.unwrap().into_inner();
    assert!(
        resp.alerts.is_empty(),
        "a committed run is not an alert (no fabricated rows)"
    );
    assert!(!resp.has_more, "empty inbox has no further pages");

    // Scoped to the run: still empty (no terminal failure on this instance).
    let scoped = c
        .list_alerts(proto::ListAlertsRequest {
            instance_id: Some(instance.clone()),
            ..all_alerts()
        })
        .await
        .unwrap()
        .into_inner();
    assert!(scoped.alerts.is_empty());

    running.shutdown().await.unwrap();
}

#[tokio::test]
async fn list_alerts_is_denied_without_auth() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(gateway_config(&dir, false, HashMap::new()))
        .await
        .unwrap();
    let mut c = connect_client(running.local_addr()).await;
    let err = c
        .list_alerts(all_alerts())
        .await
        .expect_err("deny-all rejects alerts reads");
    assert_eq!(err.code(), Code::Unauthenticated);
    running.shutdown().await.unwrap();
}
