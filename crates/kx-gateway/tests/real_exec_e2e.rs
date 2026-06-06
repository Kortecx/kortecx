//! PR-9b end-to-end witness: `Invoke` the real-exec demo recipe over a REAL
//! bound port → the embedded worker leases the bound Mote → the
//! [`kx_gateway`] router runs its body as a REAL process inside the platform
//! sandbox (bwrap on Linux / sandbox-exec on macOS) → the body's output is
//! reconciled into the content store and committed exactly-once → the client
//! fetches the committed bytes and they equal the body's deterministic output.
//!
//! This is the proof that `kx serve` runs a REAL sandboxed body (not the demo
//! storing executor). It is `#[ignore]` + runtime-skips: it needs the
//! `kx-executor` `pure_body` example built (`cargo build --example pure_body -p
//! kx-executor`) so the gateway can register it as the demo body, and a working
//! sandbox on the host. Opt in:
//!   cargo build --example pure_body -p kx-executor
//!   cargo test -p kx-gateway --test real_exec_e2e -- --ignored --nocapture
//!
//! The regression that the demo `echo` path is UNCHANGED lives in
//! `invoke_e2e.rs` (always-on); this file only adds the real-spawn witness.

#![cfg(feature = "embedded-worker")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use kx_gateway::{start, EXEC_RECIPE_HANDLE};
use kx_proto::proto;
use kx_proto::proto::kx_gateway_client::KxGatewayClient;
use tonic::transport::Channel;

/// The `pure_body` content-prefix (kx-executor/examples/pure_body.rs):
/// result_ref = BLAKE3(PREFIX ‖ input), so the committed object IS `PREFIX ‖
/// input`. The gateway feeds the Mote's id as input, so the committed bytes are
/// `PREFIX ‖ terminal_mote_id`. Kept in lock-step with the example + the
/// gateway's `real_exec::PURE_BODY_PREFIX`.
const PURE_BODY_PREFIX: &[u8] = b"kx-executor-pure-body-result";

/// Locate the built `pure_body` example by walking up to the workspace `target`
/// dir (mirrors the gateway's own `real_exec::real_body_binary_path`). `None` ⇒
/// the example wasn't built, so the test runtime-skips.
fn pure_body_built() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    for ancestor in exe.ancestors() {
        if ancestor.file_name().is_some_and(|n| n == "target") {
            for profile in ["debug", "release"] {
                let candidate = ancestor.join(profile).join("examples").join("pure_body");
                if candidate.exists() {
                    return Some(candidate);
                }
            }
        }
    }
    None
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

/// Poll `GetProjection` until `mote_id` is `Committed`; return its `result_ref`.
async fn await_mote_committed(
    c: &mut KxGatewayClient<Channel>,
    instance_id: &[u8],
    mote_id: &[u8],
) -> [u8; 32] {
    for _ in 0..200 {
        let view = c
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
    panic!("the invoked real-exec Mote never reached Committed");
}

#[tokio::test]
#[ignore = "real sandbox spawn; build `pure_body` first (cargo build --example pure_body -p kx-executor); opt in with --ignored"]
async fn invoke_real_exec_recipe_runs_a_sandboxed_body_to_committed() {
    if pure_body_built().is_none() {
        eprintln!(
            "skipping: pure_body example not built — run \
             `cargo build --example pure_body -p kx-executor` first"
        );
        return;
    }

    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    // Invoke the real-exec recipe (no free-params → empty JSON args). The bound
    // Mote leases on the embedded worker, whose router runs the body in the
    // platform sandbox and reconciles the output into the store.
    let resp = c
        .invoke(proto::InvokeRequest {
            handle: EXEC_RECIPE_HANDLE.to_string(),
            args: b"{}".to_vec(),
        })
        .await
        .expect("invoke exec-demo (is the sandbox available on this host?)")
        .into_inner();
    assert_eq!(resp.instance_id.len(), 16, "journaled instance_id is 16B");
    assert_eq!(
        resp.terminal_mote_id.len(),
        32,
        "server-derived terminal Mote"
    );

    let result_ref = await_mote_committed(&mut c, &resp.instance_id, &resp.terminal_mote_id).await;
    let mote_id: [u8; 32] = resp.terminal_mote_id.clone().try_into().unwrap();

    // The committed bytes ARE the real sandboxed body's deterministic output:
    // `PURE_BODY_PREFIX ‖ mote_id` (input-addressed), content-addressed by
    // result_ref. This proves a real process ran in the sandbox and its output
    // was committed exactly-once.
    let blob = c
        .get_content(proto::GetContentRequest {
            content_ref: result_ref.to_vec(),
            instance_id: resp.instance_id.clone(),
        })
        .await
        .unwrap()
        .into_inner();
    let mut expected = Vec::with_capacity(PURE_BODY_PREFIX.len() + mote_id.len());
    expected.extend_from_slice(PURE_BODY_PREFIX);
    expected.extend_from_slice(&mote_id);
    assert_eq!(
        blob.payload, expected,
        "committed bytes == the sandboxed body's reconciled output"
    );

    running.shutdown().await.unwrap();
}
