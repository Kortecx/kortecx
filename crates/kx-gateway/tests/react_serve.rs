//! PR-2d-2 e2e witness: `kx serve --features inference` drives a LIVE `ReAct`
//! chain end-to-end through the real serve stack.
//!
//! `Invoke` the server-provisioned `kx/recipes/react` recipe by handle+args
//! (instruction + the budget caps) → the gateway-core binder binds the three
//! free-params and submits with `react_seed = true` → the coordinator's
//! seed-swap anchors the run-salted chain with the BOUND caps → the embedded
//! worker leases turn 0, the `ModelRouterExecutor`'s react arm runs REAL greedy
//! inference and raw-commits the output → the settle decodes it through the ONE
//! authority gate and (for a plain question) freezes `Answer`. The chain's
//! durable facts surface via `ListReactTurns`.
//!
//! The full TOOL round (a frozen `Tool` fact → the observation firing the
//! bundled `kx-mcp-echo` through the broker) is pinned model-free at the
//! coordinator layer (`kx-coordinator/tests/react_live.rs` — a live model
//! cannot be scripted into proposing an envelope deterministically); this test
//! proves the SERVE wiring: recipe provisioning, the form, the `react_seed`
//! plumbing, the bound caps, and the live answer settle.
//!
//! Gated `#[cfg(feature = "inference")]` AND `#[ignore]`; runtime-skips without
//! a GGUF (`just fetch-agent-model` or `KX_SERVE_MODEL_GGUF`). The react recipe
//! additionally needs the bundled `kx-mcp-echo` bin (`cargo build -p kx-mcp`
//! places it in `target/debug`; or set `KX_MCP_ECHO_PATH`).

#![cfg(feature = "inference")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use kx_gateway::{start, REACT_RECIPE_HANDLE};
use kx_proto::proto;
use kx_proto::proto::kx_gateway_client::KxGatewayClient;
use tonic::transport::Channel;

/// The serve model GGUF: `KX_SERVE_MODEL_GGUF` if set, else the public stand-in
/// `just fetch-agent-model` downloads. `None` ⇒ the test runtime-skips.
fn serve_model() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("KX_SERVE_MODEL_GGUF") {
        let p = PathBuf::from(p);
        return p.is_file().then_some(p);
    }
    let standin = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/models/qwen3-0.6b-q4_k_m.gguf");
    standin.is_file().then_some(standin)
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
#[ignore = "real in-process LLM inference; needs a GGUF (just fetch-agent-model); opt in with --ignored"]
async fn invoke_react_recipe_drives_a_live_chain_to_answer() {
    let Some(gguf) = serve_model() else {
        eprintln!(
            "skipping: no serve model — run `just fetch-agent-model` (or set \
             KX_SERVE_MODEL_GGUF) first"
        );
        return;
    };
    std::env::set_var("KX_SERVE_MODEL_GGUF", &gguf);

    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    // The react recipe is provisioned (model + bundled tool present) and its
    // form declares the three typed free-params.
    let recipes = c
        .list_recipes(proto::ListRecipesRequest {})
        .await
        .unwrap()
        .into_inner();
    if !recipes
        .recipes
        .iter()
        .any(|r| r.handle == REACT_RECIPE_HANDLE)
    {
        eprintln!(
            "skipping: kx/recipes/react not provisioned — the bundled kx-mcp-echo \
             bin was not found (cargo build -p kx-mcp, or set KX_MCP_ECHO_PATH)"
        );
        running.shutdown().await.unwrap();
        return;
    }
    let form = c
        .get_recipe_form(proto::GetRecipeFormRequest {
            handle: REACT_RECIPE_HANDLE.to_string(),
        })
        .await
        .unwrap()
        .into_inner();
    let names: Vec<&str> = form.fields.iter().map(|f| f.name.as_str()).collect();
    assert!(names.contains(&"instruction"));
    assert!(names.contains(&"max_turns"));
    assert!(names.contains(&"max_tool_calls"));

    // Invoke with explicit caps — the durable anchor must record THESE.
    let resp = c
        .invoke(proto::InvokeRequest {
            handle: REACT_RECIPE_HANDLE.to_string(),
            args: br#"{"instruction":"What is 2+2? Answer briefly in prose.","max_turns":4,"max_tool_calls":2}"#
                .to_vec(),
            context_bundles: vec![],
            context_refs: vec![],
        })
        .await
        .expect("invoke kx/recipes/react")
        .into_inner();
    assert_eq!(resp.instance_id.len(), 16, "journaled instance_id is 16B");

    // Await the chain's settle: the model answers in prose ⇒ the settle freezes
    // a terminal `Answer` branch (CPU/Metal inference is slow; be generous).
    let mut answered = false;
    let mut caps_seen: Option<(u32, u32)> = None;
    for _ in 0..600 {
        let turns = c
            .list_react_turns(proto::ListReactTurnsRequest {
                limit: None,
                instance_id: Some(resp.instance_id.clone()),
                step_salt: None,
            })
            .await
            .unwrap()
            .into_inner();
        for t in &turns.turns {
            caps_seen = Some((t.max_turns, t.max_tool_calls));
            if t.branch == "answer" {
                answered = true;
            }
        }
        if answered {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        answered,
        "the live chain settled a terminal Answer fact (via ListReactTurns)"
    );
    assert_eq!(
        caps_seen,
        Some((4, 2)),
        "the durable anchor recorded the BOUND caps, not the defaults"
    );

    running.shutdown().await.unwrap();
}
