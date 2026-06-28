//! RC1 (D172) real-model eval witness (Tier-B, advisory): drive a LIVE `ReAct` chain on
//! a real OSS model and score it through the `ScoreRun` RPC — the end-to-end proof that
//! the per-run quality readout works over genuine model output (GR15 real-model
//! integrity).
//!
//! Model selection mirrors the rest of the serve e2e suite: a GGUF
//! (`just fetch-agent-model` / `KX_SERVE_MODEL_GGUF` — Qwen3-0.6B in CI) OR the Ollama
//! engine (`KX_SERVE_OLLAMA=on KX_SERVE_OLLAMA_MODELS=gemma3:12b` — the local Gemma deep
//! test). Gated `#[cfg(feature = "inference")]` (the FFI links for the GGUF arm; the
//! Ollama arm is FFI-free but shares the harness) AND `#[ignore]`; runtime-skips with no
//! model. The assertions are LOOSE Tier-B floors — a real run's quality is recorded, not
//! a hard gate (the deterministic golden gate is `just eval`).

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

fn serve_gguf() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("KX_SERVE_MODEL_GGUF") {
        let p = PathBuf::from(p);
        return p.is_file().then_some(p);
    }
    let standin = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/models/qwen3-0.6b-q4_k_m.gguf");
    standin.is_file().then_some(standin)
}

/// Whether the operator opted Ollama in (`KX_SERVE_OLLAMA` truthy).
fn ollama_opted_in() -> bool {
    matches!(
        std::env::var("KX_SERVE_OLLAMA")
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str(),
        "1" | "on" | "true" | "yes"
    )
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
#[ignore = "real LLM inference; needs a GGUF (just fetch-agent-model) or Ollama (KX_SERVE_OLLAMA=on); opt in with --ignored"]
async fn score_run_over_a_live_react_chain() {
    // Resolve the model: a GGUF (set the env so serve loads it) or the Ollama opt-in.
    if let Some(gguf) = serve_gguf() {
        std::env::set_var("KX_SERVE_MODEL_GGUF", &gguf);
    } else if !ollama_opted_in() {
        eprintln!(
            "skipping: no model — `just fetch-agent-model` (GGUF) or \
             `KX_SERVE_OLLAMA=on KX_SERVE_OLLAMA_MODELS=gemma3:12b`"
        );
        return;
    }
    std::env::set_var("KX_SERVE_AUTOGRANT", "1");

    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    // react-auto needs the model + the bundled echo capability. If react itself didn't
    // provision, the bundled bin is absent — skip (matches react_auto_serve).
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
        eprintln!("skipping: kx/recipes/react not provisioned — bundled kx-mcp-echo missing");
        running.shutdown().await.unwrap();
        std::env::remove_var("KX_SERVE_AUTOGRANT");
        return;
    }

    // A tool-eliciting instruction (the model MAY call echo or answer directly — the
    // Tier-B floors are model-choice-agnostic).
    let resp = c
        .invoke(proto::InvokeRequest {
            handle: REACT_AUTO_RECIPE_HANDLE.to_string(),
            args: br#"{"instruction":"Echo the word 'kortecx' using the echo tool, then tell me what it echoed. If you cannot, answer directly.","max_turns":4,"max_tool_calls":2}"#
                .to_vec(),
            context_bundles: vec![],
            context_refs: vec![],
        })
        .await
        .expect("invoke kx/recipes/react-auto")
        .into_inner();
    assert_eq!(resp.instance_id.len(), 16, "journaled instance_id is 16B");

    // Poll until the chain settles a terminal branch (answer or dead-letter).
    let mut settled = false;
    for _ in 0..1200 {
        let turns = c
            .list_react_turns(proto::ListReactTurnsRequest {
                limit: None,
                instance_id: Some(resp.instance_id.clone()),
                step_salt: None,
            })
            .await
            .unwrap()
            .into_inner();
        if turns
            .turns
            .iter()
            .any(|t| t.branch == "answer" || t.branch == "dead_lettered")
        {
            settled = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(settled, "the react-auto chain settled a terminal branch");

    // THE WITNESS: score the real run through the ScoreRun RPC.
    let score = c
        .score_run(proto::ScoreRunRequest {
            instance_id: resp.instance_id.clone(),
        })
        .await
        .expect("ScoreRun over a live run")
        .into_inner();

    eprintln!(
        "eval ScoreRun (real model): terminal={} reached_answer={} turns={}/{} tools={}/{} \
         rejections={} budget(turns)={}‰ budget(tools)={}‰",
        score.terminal,
        score.reached_answer,
        score.turns_used,
        score.max_turns,
        score.tool_calls_used,
        score.max_tool_calls,
        score.rejections,
        score.turn_budget_used_per_mille,
        score.tool_budget_used_per_mille,
    );

    // Loose Tier-B floors (a real run's quality is RECORDED, not gated).
    assert_eq!(
        score.instance_id, resp.instance_id,
        "ScoreRun echoes the run id"
    );
    assert!(
        score.turns_used >= 1,
        "a settled run used at least one turn"
    );
    assert!(score.max_turns >= 1, "the admitted turn cap is recorded");
    assert!(
        matches!(
            score.terminal.as_str(),
            "answer" | "tool" | "rejected" | "dead_lettered" | "pending"
        ),
        "a valid terminal branch: {}",
        score.terminal
    );
    assert!(score.turn_budget_used_per_mille <= 1000);
    assert!(score.tool_budget_used_per_mille <= 1000);

    running.shutdown().await.unwrap();
    std::env::remove_var("KX_SERVE_AUTOGRANT");
}
