//! PR-6b-4 e2e witness: `KX_SERVE_AUTOGRANT=1 kx serve --features inference`
//! provisions `kx/recipes/react-auto` and drives a LIVE `ReAct` chain through it.
//!
//! With the operator opt-in, the serve seeds `kx/recipes/react-auto`; the binder
//! rebuilds its warrant from the LIVE registry at bind (auto-granting the
//! registered/dialed tool set, capped) and submits with `react_seed = true` → the
//! coordinator anchors the run-salted chain → the embedded worker drives REAL
//! greedy inference → the settle freezes the terminal branch, surfaced via
//! `ListReactTurns`. The bind-layer override (union warrant, `MoteId` invariance,
//! auth gate) is pinned model-free in `react_auto_bind.rs`; this proves the SERVE
//! wiring under the flag (recipe provisioning, the form, the live drive).
//!
//! Gated `#[cfg(feature = "inference")]` AND `#[ignore]`; runtime-skips without a
//! `GGUF` (`just fetch-agent-model` or `KX_SERVE_MODEL_GGUF`) or the bundled
//! `kx-mcp-echo` bin (`cargo build -p kx-mcp`, or `KX_MCP_ECHO_PATH`).

#![cfg(feature = "inference")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use kx_gateway::{start, REACT_AUTO_RECIPE_HANDLE, REACT_RECIPE_HANDLE};
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
async fn autogrant_serve_provisions_react_auto_and_drives_a_live_chain() {
    let Some(gguf) = serve_model() else {
        eprintln!(
            "skipping: no serve model — run `just fetch-agent-model` (or set KX_SERVE_MODEL_GGUF)"
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

    let recipes = c
        .list_recipes(proto::ListRecipesRequest {})
        .await
        .unwrap()
        .into_inner();
    // react-auto requires the model + the bundled echo capability (same gate as
    // react). If react itself didn't provision, the bundled bin is absent — skip.
    if !recipes
        .recipes
        .iter()
        .any(|r| r.handle == REACT_RECIPE_HANDLE)
    {
        eprintln!("skipping: kx/recipes/react not provisioned — bundled kx-mcp-echo missing");
        running.shutdown().await.unwrap();
        std::env::remove_var("KX_SERVE_AUTOGRANT");
        return;
    }
    assert!(
        recipes
            .recipes
            .iter()
            .any(|r| r.handle == REACT_AUTO_RECIPE_HANDLE),
        "KX_SERVE_AUTOGRANT on ⇒ kx/recipes/react-auto is provisioned"
    );

    // The form is the react contract (instruction + the two budget caps).
    let form = c
        .get_recipe_form(proto::GetRecipeFormRequest {
            handle: REACT_AUTO_RECIPE_HANDLE.to_string(),
        })
        .await
        .unwrap()
        .into_inner();
    let names: Vec<&str> = form.fields.iter().map(|f| f.name.as_str()).collect();
    assert!(names.contains(&"instruction"));
    assert!(names.contains(&"max_turns"));
    assert!(names.contains(&"max_tool_calls"));

    let resp = c
        .invoke(proto::InvokeRequest {
            handle: REACT_AUTO_RECIPE_HANDLE.to_string(),
            args: br#"{"instruction":"What is 2+2? Answer briefly in prose.","max_turns":4,"max_tool_calls":2}"#
                .to_vec(),
            context_bundles: vec![],
            context_refs: vec![],
        })
        .await
        .expect("invoke kx/recipes/react-auto")
        .into_inner();
    assert_eq!(resp.instance_id.len(), 16, "journaled instance_id is 16B");

    let mut answered = false;
    for _ in 0..600 {
        let turns = c
            .list_react_turns(proto::ListReactTurnsRequest {
                limit: None,
                instance_id: Some(resp.instance_id.clone()),
            })
            .await
            .unwrap()
            .into_inner();
        if turns.turns.iter().any(|t| t.branch == "answer") {
            answered = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        answered,
        "the react-auto chain settled a terminal Answer fact"
    );

    running.shutdown().await.unwrap();
    std::env::remove_var("KX_SERVE_AUTOGRANT");
}
