//! End-to-end witnesses for the `Invoke` RPC over a REAL bound port (R2b) — the
//! G3 enterprise-adoption unlock made reachable:
//!
//! - `Invoke` a server-provisioned PURE recipe by handle+args → the embedded
//!   worker leases→runs→commits the bound Mote → the client awaits ITS
//!   `terminal_mote_id` and fetches the deterministic result via `GetContent`;
//! - a recipe run is keyed by recipe identity, so distinct args → distinct
//!   `terminal_mote_id`s WITHIN one run (exactly-once-per-input);
//! - an unknown handle is uniformly `permission_denied` (no existence oracle on
//!   the execution surface); malformed args are `invalid_argument` (fail-closed);
//! - a deny-all port refuses `Invoke` (`unauthenticated`); a bearer-token port
//!   authorizes a configured party end-to-end;
//! - many concurrent `Invoke`s (distinct args) all commit distinct Motes.

#![cfg(feature = "embedded-worker")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::{BTreeSet, HashMap};
use std::net::SocketAddr;
use std::time::Duration;

use kx_gateway::{start, DEMO_RECIPE_HANDLE};
use kx_proto::proto;
use kx_proto::proto::kx_gateway_client::KxGatewayClient;
use tonic::transport::Channel;
use tonic::Request;

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

fn with_bearer<T>(payload: T, token: Option<&str>) -> Request<T> {
    let mut req = Request::new(payload);
    if let Some(token) = token {
        req.metadata_mut()
            .insert("authorization", format!("Bearer {token}").parse().unwrap());
    }
    req
}

fn args(topic: &str) -> Vec<u8> {
    format!("{{\"topic\":\"{topic}\"}}").into_bytes()
}

/// Poll `GetProjection` (optionally bearer-authenticated) until the specific
/// `mote_id` is `Committed`; return its `result_ref`. Fails on timeout.
async fn await_mote_committed(
    c: &mut KxGatewayClient<Channel>,
    instance_id: &[u8],
    mote_id: &[u8],
    token: Option<&str>,
) -> [u8; 32] {
    for _ in 0..100 {
        let view = c
            .get_projection(with_bearer(
                proto::GetProjectionRequest {
                    instance_id: instance_id.to_vec(),
                    at_seq: None,
                },
                token,
            ))
            .await
            .unwrap()
            .into_inner();
        if let Some(m) = view
            .motes
            .iter()
            .find(|m| m.mote_id == mote_id && m.state == proto::MoteSnapshotState::Committed as i32)
        {
            return m
                .result_ref
                .clone()
                .expect("a committed Mote carries a result_ref")
                .try_into()
                .unwrap();
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the invoked Mote never reached Committed");
}

#[tokio::test]
async fn invoke_runs_demo_recipe_to_committed() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    let resp = c
        .invoke(proto::InvokeRequest {
            handle: DEMO_RECIPE_HANDLE.to_string(),
            args: args("incidents"),
            context_bundles: vec![],
            context_refs: vec![],
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(resp.instance_id.len(), 16, "journaled instance_id is 16B");
    assert_eq!(
        resp.terminal_mote_id.len(),
        32,
        "server-derived terminal Mote"
    );

    // Await THIS invocation's Mote, then fetch its committed result.
    let result_ref =
        await_mote_committed(&mut c, &resp.instance_id, &resp.terminal_mote_id, None).await;
    let blob = c
        .get_content(proto::GetContentRequest {
            content_ref: result_ref.to_vec(),
            instance_id: resp.instance_id.clone(),
        })
        .await
        .unwrap()
        .into_inner();
    // GR15: `echo` is a TRUE echo — it commits its bound `topic` verbatim.
    assert_eq!(
        blob.payload, b"incidents",
        "GetContent returns the bytes the worker committed for the invoked recipe (the echoed topic)"
    );

    running.shutdown().await.unwrap();
}

#[tokio::test]
async fn invoke_distinct_args_yield_distinct_motes() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    let a = c
        .invoke(proto::InvokeRequest {
            handle: DEMO_RECIPE_HANDLE.to_string(),
            args: args("alpha"),
            context_bundles: vec![],
            context_refs: vec![],
        })
        .await
        .unwrap()
        .into_inner();
    let b = c
        .invoke(proto::InvokeRequest {
            handle: DEMO_RECIPE_HANDLE.to_string(),
            args: args("bravo"),
            context_bundles: vec![],
            context_refs: vec![],
        })
        .await
        .unwrap()
        .into_inner();

    // Same recipe ⇒ one run instance; distinct args ⇒ distinct Motes within it.
    assert_eq!(a.instance_id, b.instance_id, "same recipe shares one run");
    assert_ne!(
        a.terminal_mote_id, b.terminal_mote_id,
        "distinct args → distinct committed Mote identities (exactly-once-per-input)"
    );

    // Both reach Committed.
    await_mote_committed(&mut c, &a.instance_id, &a.terminal_mote_id, None).await;
    await_mote_committed(&mut c, &b.instance_id, &b.terminal_mote_id, None).await;

    running.shutdown().await.unwrap();
}

#[tokio::test]
async fn invoke_unknown_handle_is_permission_denied() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    // An authenticated caller probing a non-existent recipe learns nothing —
    // uniform permission_denied (no existence oracle on the execution surface).
    let err = c
        .invoke(proto::InvokeRequest {
            handle: "kx/recipes/does-not-exist".to_string(),
            args: args("x"),
            context_bundles: vec![],
            context_refs: vec![],
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), tonic::Code::PermissionDenied);

    running.shutdown().await.unwrap();
}

#[tokio::test]
async fn invoke_malformed_args_are_invalid_argument() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    for bad in [br#"{"topic":5}"#.to_vec(), b"{}".to_vec()] {
        let err = c
            .invoke(proto::InvokeRequest {
                handle: DEMO_RECIPE_HANDLE.to_string(),
                args: bad,
                context_bundles: vec![],
                context_refs: vec![],
            })
            .await
            .unwrap_err();
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
    }

    running.shutdown().await.unwrap();
}

#[tokio::test]
async fn invoke_under_deny_all_is_unauthenticated() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, false, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    let err = c
        .invoke(proto::InvokeRequest {
            handle: DEMO_RECIPE_HANDLE.to_string(),
            args: args("x"),
            context_bundles: vec![],
            context_refs: vec![],
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), tonic::Code::Unauthenticated);

    running.shutdown().await.unwrap();
}

#[tokio::test]
async fn invoke_with_bearer_token_runs_to_committed() {
    let dir = tempfile::TempDir::new().unwrap();
    let mut tokens = HashMap::new();
    tokens.insert("s3cr3t".to_string(), "alice@acme".to_string());
    let running = start(common::gateway_config(&dir, false, tokens))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    // No credential → denied (every RPC is gated by the interceptor).
    let no_tok = c
        .invoke(proto::InvokeRequest {
            handle: DEMO_RECIPE_HANDLE.to_string(),
            args: args("x"),
            context_bundles: vec![],
            context_refs: vec![],
        })
        .await
        .unwrap_err();
    assert_eq!(no_tok.code(), tonic::Code::Unauthenticated);

    // Valid credential → the configured party holds a Use grant → runs to Committed.
    let resp = c
        .invoke(with_bearer(
            proto::InvokeRequest {
                handle: DEMO_RECIPE_HANDLE.to_string(),
                args: args("incidents"),
                context_bundles: vec![],
                context_refs: vec![],
            },
            Some("s3cr3t"),
        ))
        .await
        .unwrap()
        .into_inner();
    // The polling RPCs are gated too — carry the token.
    await_mote_committed(
        &mut c,
        &resp.instance_id,
        &resp.terminal_mote_id,
        Some("s3cr3t"),
    )
    .await;

    running.shutdown().await.unwrap();
}

#[tokio::test]
async fn concurrent_invokes_all_commit() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let c = client(running.local_addr()).await;

    // 16 concurrent invokes with DISTINCT args → 16 distinct Motes, all commit.
    let mut tasks = Vec::new();
    for i in 0..16u32 {
        let mut c = c.clone();
        tasks.push(tokio::spawn(async move {
            let resp = c
                .invoke(proto::InvokeRequest {
                    handle: DEMO_RECIPE_HANDLE.to_string(),
                    args: args(&format!("topic-{i}")),
                    context_bundles: vec![],
                    context_refs: vec![],
                })
                .await
                .unwrap()
                .into_inner();
            await_mote_committed(&mut c, &resp.instance_id, &resp.terminal_mote_id, None).await;
            let id: [u8; 32] = resp.terminal_mote_id.try_into().unwrap();
            id
        }));
    }
    let mut committed = BTreeSet::new();
    for t in tasks {
        committed.insert(t.await.unwrap());
    }
    assert_eq!(
        committed.len(),
        16,
        "all 16 concurrent invokes committed distinct Motes"
    );

    running.shutdown().await.unwrap();
}
