//! D114/M11 autonomy-safety RPC e2e: drives `ListPendingApprovals` / `GrantApproval` /
//! `DenyApproval` / `GetRunCost` through the REAL gateway service (the handlers + the
//! host `ApprovalAdmin` seam over the embedded coordinator), deterministically (no
//! model). The full block→grant→fire flow against a real model is covered by the live
//! dual-engine validation + the deterministic coordinator gate tests; here we prove
//! the admin RPC wiring, the empty-inbox / unknown-request idempotent-no-op paths, and
//! the cost readout's zero-baseline.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Duration;

use kx_gateway::{start, GatewayConfig};
use kx_proto::proto;
use kx_proto::proto::kx_gateway_client::KxGatewayClient;
use tempfile::TempDir;
use tonic::transport::Channel;

mod common;

fn config(dir: &TempDir) -> GatewayConfig {
    common::gateway_config(dir, true, HashMap::new())
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

#[tokio::test]
async fn approval_inbox_is_empty_and_unknown_requests_no_op() {
    let dir = TempDir::new().unwrap();
    let running = start(config(&dir)).await.unwrap();
    let mut c = client(running.local_addr()).await;

    // A fresh serve has no withheld actions — the inbox is empty (the wiring works,
    // never `unimplemented`: the approval admin is wired over the embedded coordinator).
    let list = c
        .list_pending_approvals(proto::ListPendingApprovalsRequest { limit: 0 })
        .await
        .expect("list pending approvals")
        .into_inner();
    assert!(
        list.approvals.is_empty(),
        "no pending approvals on a fresh serve"
    );

    // Grant/Deny of an UNKNOWN (never-requested) 16-byte request id is an idempotent
    // no-op — `false`, never an error (a client can't mint authority over a request
    // that does not exist; SN-8).
    let unknown = vec![0x11u8; 16];
    let granted = c
        .grant_approval(proto::GrantApprovalRequest {
            request_id: unknown.clone(),
            reason: "n/a".to_string(),
        })
        .await
        .expect("grant approval rpc")
        .into_inner();
    assert!(!granted.granted, "granting an unknown request is a no-op");

    let denied = c
        .deny_approval(proto::DenyApprovalRequest {
            request_id: unknown,
            reason: "n/a".to_string(),
        })
        .await
        .expect("deny approval rpc")
        .into_inner();
    assert!(!denied.denied, "denying an unknown request is a no-op");

    // A malformed (non-16-byte) request id is rejected fail-closed (invalid_argument).
    let bad = c
        .grant_approval(proto::GrantApprovalRequest {
            request_id: vec![0x00u8; 8],
            reason: String::new(),
        })
        .await;
    assert!(bad.is_err(), "a non-16-byte request id is rejected");
    assert_eq!(bad.unwrap_err().code(), tonic::Code::InvalidArgument);
}

#[tokio::test]
async fn run_cost_readout_zero_baseline() {
    let dir = TempDir::new().unwrap();
    let running = start(config(&dir)).await.unwrap();
    let mut c = client(running.local_addr()).await;

    // A run with no react facts (an unknown instance) prices to zero turns/tool_calls
    // ⇒ a zero estimate, with the operator-configured rates surfaced for provenance.
    let cost = c
        .get_run_cost(proto::GetRunCostRequest {
            instance_id: vec![0x22u8; 16],
        })
        .await
        .expect("get run cost")
        .into_inner();
    assert_eq!(cost.turns, 0);
    assert_eq!(cost.tool_calls, 0);
    assert_eq!(cost.estimated_micro_usd, 0);
    // The default (env-unset) rates are non-zero (the guardrail prices something) —
    // proves the price-book resolved + the readout is wired end-to-end.
    assert!(
        cost.per_turn_micro_usd > 0,
        "default per-turn rate is surfaced"
    );
    assert!(
        cost.per_tool_call_micro_usd > 0,
        "default per-tool-call rate is surfaced"
    );

    // A malformed (non-16-byte) instance id is rejected fail-closed.
    let bad = c
        .get_run_cost(proto::GetRunCostRequest {
            instance_id: vec![0x00u8; 4],
        })
        .await;
    assert!(bad.is_err(), "a non-16-byte instance id is rejected");
    assert_eq!(bad.unwrap_err().code(), tonic::Code::InvalidArgument);
}
