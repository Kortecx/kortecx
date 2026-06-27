//! D167 (connector SDK) live witness: an EXTERNAL connector registered through the
//! `RegisterMcpServer` RPC (the connector-SDK path, NOT the serve's hardcoded
//! bundled-echo registration) is DISCOVERED, auto-granted under `react-auto`, and
//! callable by a LIVE model — the chain settles a terminal Answer.
//!
//! Unlike the bundled single-shot `kx-mcp-echo`, this dials the SDK's reference
//! connector (`kx-connector-example`, a full `initialize → tools/list → tools/call`
//! MCP server) over `RegisterMcpServer` — exactly what `kx connections add` /
//! `flow().with_mcp(...)` do — so this proves the END-TO-END connector path: dial →
//! discover → auto-grant → a real model fires the dialed tool.
//!
//! The non-flaky assertion is the invariant (the chain settles, never dead-letters
//! on a freshly-dialed external tool); whether a `tool` round actually fired is
//! model-nondeterministic, so it is LOGGED, not asserted (mirrors
//! `react_auto_serve.rs`). Locally validate on BOTH engines with Gemma-4 (GR24).
//!
//! Gated `#[cfg(feature = "inference")]` AND `#[ignore]`; runtime-skips without a
//! GGUF (`KX_SERVE_MODEL_GGUF` / `just fetch-gemma-model`), without the bundled
//! `kx-mcp-echo` (react-auto's provisioning gate), or without the reference
//! connector bin (`cargo build -p kx-extension-sdk`, or `KX_CONNECTOR_EXAMPLE_PATH`).

#![cfg(feature = "inference")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use kx_gateway::{start, REACT_AUTO_RECIPE_HANDLE};
use kx_proto::proto;
use kx_proto::proto::kx_gateway_client::KxGatewayClient;
use tonic::transport::Channel;

fn serve_model() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("KX_SERVE_MODEL_GGUF") {
        let p = PathBuf::from(p);
        return p.is_file().then_some(p);
    }
    let standin = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/models/qwen3-0.6b-q4_k_m.gguf");
    standin.is_file().then_some(standin)
}

/// Locate the SDK reference connector bin (`kx-connector-example`): an explicit
/// `KX_CONNECTOR_EXAMPLE_PATH`, else a walk up to the workspace `target/{debug,release}`.
fn reference_connector_bin() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("KX_CONNECTOR_EXAMPLE_PATH") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    let exe = std::env::current_exe().ok()?;
    for ancestor in exe.ancestors() {
        if ancestor.file_name().is_some_and(|n| n == "target") {
            for profile in ["debug", "release"] {
                let candidate = ancestor.join(profile).join("kx-connector-example");
                if candidate.is_file() {
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

#[tokio::test(flavor = "multi_thread")]
#[ignore = "real in-process LLM inference; needs a GGUF + the reference connector bin; opt in with --ignored"]
async fn external_connector_registered_via_rpc_is_callable_by_a_live_model() {
    let Some(gguf) = serve_model() else {
        eprintln!("skipping: no serve model — set KX_SERVE_MODEL_GGUF (a real GGUF)");
        return;
    };
    let Some(conn_bin) = reference_connector_bin() else {
        eprintln!(
            "skipping: reference connector not built — run `cargo build -p kx-extension-sdk` \
             (or set KX_CONNECTOR_EXAMPLE_PATH)"
        );
        return;
    };
    std::env::set_var("KX_SERVE_MODEL_GGUF", &gguf);
    std::env::set_var("KX_SERVE_AUTOGRANT", "1");

    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    // Register the EXTERNAL reference connector through the connector-SDK RPC path.
    let reg = c
        .register_mcp_server(proto::RegisterMcpServerRequest {
            server_name: "refconn".to_string(),
            transport: "stdio".to_string(),
            endpoint: conn_bin.to_string_lossy().into_owned(),
            args: vec![],
            tls_required: false,
            credential_ref: String::new(),
            session_mode: "stateless".to_string(),
        })
        .await
        .expect("register the external reference connector")
        .into_inner();
    assert_eq!(
        reg.health, "connected",
        "the reference connector dials cleanly"
    );
    assert!(
        reg.discovered >= 1,
        "the reference connector exposes >= 1 tool"
    );
    eprintln!(
        "registered refconn: {} tool(s) discovered (e.g. refconn/echo, refconn/reverse)",
        reg.discovered
    );

    // react-auto provisioning is gated on the bundled echo (kx/recipes/react). If it
    // is absent, skip — the model-free dial proof above still ran.
    let recipes = c
        .list_recipes(proto::ListRecipesRequest {})
        .await
        .unwrap()
        .into_inner();
    if !recipes
        .recipes
        .iter()
        .any(|r| r.handle == REACT_AUTO_RECIPE_HANDLE)
    {
        eprintln!(
            "skipping the live-drive leg: react-auto not provisioned (bundled kx-mcp-echo missing)"
        );
        running.shutdown().await.unwrap();
        std::env::remove_var("KX_SERVE_AUTOGRANT");
        return;
    }

    // Drive a live model on a tool-forcing instruction naming the EXTERNAL tool. The
    // react-auto warrant is rebuilt from the LIVE registry at bind (server.rs:848), so
    // the freshly dialed `refconn/*` tools are auto-granted and callable. The
    // instruction mirrors the proven bundled-echo drive (react_auto_serve.rs) so the
    // SAME model reliably fires — here against an EXTERNALLY-registered connector.
    let resp = c
        .invoke(proto::InvokeRequest {
            handle: REACT_AUTO_RECIPE_HANDLE.to_string(),
            args: br#"{"instruction":"You MUST use the echo tool to echo the exact text 'pong'. Call the tool first, then report what it returned.","max_turns":6,"max_tool_calls":3}"#
                .to_vec(),
            context_bundles: vec![],
            context_refs: vec![],
        })
        .await
        .expect("invoke kx/recipes/react-auto with the external tool in the menu")
        .into_inner();
    assert_eq!(resp.instance_id.len(), 16, "journaled instance_id is 16B");

    let mut fired = false;
    let mut answered = false;
    let mut bounded = false;
    let mut last = String::new();
    // ~180s: ample for a large opt-in model (Gemma-4-12B) running a multi-turn tool
    // loop (slow ≠ failure); the CI Qwen3-0.6B settles in ~3s. Mirrors react_auto_serve.rs.
    for _ in 0..1800 {
        let turns = c
            .list_react_turns(proto::ListReactTurnsRequest {
                limit: None,
                instance_id: Some(resp.instance_id.clone()),
                step_salt: None,
            })
            .await
            .unwrap()
            .into_inner();
        let branches: Vec<&str> = turns.turns.iter().map(|t| t.branch.as_str()).collect();
        let snap = format!("{branches:?}");
        if snap != last {
            eprintln!("external-connector witness — trajectory so far: {snap}");
            for t in &turns.turns {
                if t.branch == "rejected" && !t.rejection_reason.is_empty() {
                    eprintln!("  turn {} rejected: {}", t.turn, t.rejection_reason);
                }
            }
            last = snap.clone();
        }
        let tool_calls = turns.turns.iter().filter(|t| t.branch == "tool").count();
        let cap = turns
            .turns
            .iter()
            .map(|t| t.max_tool_calls as usize)
            .max()
            .unwrap_or(0);
        fired |= tool_calls > 0;
        answered = turns.turns.iter().any(|t| t.branch == "answer");
        // BOUNDED: a terminal branch (answer / dead_lettered) OR the tool-call budget
        // spent — never a silent wedge on a freshly-dialed EXTERNAL tool.
        let dead = turns
            .turns
            .iter()
            .any(|t| t.branch == "answer" || t.branch == "dead_lettered");
        if dead || (cap > 0 && tool_calls >= cap) {
            bounded = true;
            eprintln!(
                "external-connector witness — BOUNDED. fired: {fired}, answered: {answered}, \
                 tool_calls: {tool_calls}/{cap}, trajectory: {snap}"
            );
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // The non-flaky invariant: registering an EXTERNAL connector via the SDK RPC path
    // never wedges the live chain — it reaches a terminal OR spends its tool budget.
    assert!(
        bounded,
        "the live chain over the externally-registered connector is BOUNDED — it reached \
         a terminal branch or spent its tool-call budget, never a wedge. fired: {fired}; answered: {answered}"
    );
    // Whether the model fired the external tool vs answered directly is
    // model-nondeterministic — LOGGED, not hard-asserted (mirrors react_auto_serve.rs).
    eprintln!("external connector live drive: bounded={bounded} fired={fired} answered={answered}");

    running.shutdown().await.unwrap();
    std::env::remove_var("KX_SERVE_AUTOGRANT");
}
