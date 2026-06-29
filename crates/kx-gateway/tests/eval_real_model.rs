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

/// RC2 grammar witness (Tier-B, observe-not-gate per GR16): FORCE a tool the model
/// cannot answer without (a kv lookup of an arbitrary key), then PRINT each
/// committed turn's RAW output so the emitted tool-call FORMAT is visible, and
/// OBSERVE whether a tool fired. The lazy GBNF triggers on the `{"tool_call"`
/// opener, so this reveals whether the model emits that JSON envelope (the grammar
/// bites + structurally constrains) or a native format (the robust parser recovers
/// it). Soft assertion: the chain SETTLES with no crash (the firing path itself is
/// gated deterministically in kx-coordinator/kx-toolcall).
#[tokio::test(flavor = "multi_thread")]
#[ignore = "real LLM; forces a tool to witness the grammar-constrained fire + emitted format"]
async fn grammar_forces_and_witnesses_a_tool_fire() {
    if let Some(gguf) = serve_gguf() {
        std::env::set_var("KX_SERVE_MODEL_GGUF", &gguf);
    } else if !ollama_opted_in() {
        eprintln!("skipping: no model");
        return;
    }
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
    if !recipes
        .recipes
        .iter()
        .any(|r| r.handle == REACT_RECIPE_HANDLE)
    {
        eprintln!("skipping: react not provisioned (bundled bins missing)");
        running.shutdown().await.unwrap();
        std::env::remove_var("KX_SERVE_AUTOGRANT");
        return;
    }

    // A kv lookup of an ARBITRARY key the model cannot know without the tool.
    let resp = c
        .invoke(proto::InvokeRequest {
            handle: REACT_AUTO_RECIPE_HANDLE.to_string(),
            args: {
                // The live serve react prompt presents NO tool menu/schema (SERVE_SYSTEM
                // is generic) — so a real model can only fire a tool when the instruction
                // itself describes the call format. Override via KX_WITNESS_INSTRUCTION.
                let instruction = std::env::var("KX_WITNESS_INSTRUCTION").unwrap_or_else(|_| {
                    "To use a tool, output ONLY a JSON object exactly like \
                     {\\\"tool_call\\\":{\\\"name\\\":\\\"<tool>\\\",\\\"version\\\":\\\"1\\\",\\\"args\\\":{...}}}. \
                     The value for key 'x' lives in a key-value store you cannot see; call the tool \
                     \\\"mcp-kv/get\\\" with args {\\\"key\\\":\\\"x\\\"} to retrieve it, then tell me ONLY that value."
                        .to_string()
                });
                format!(
                    r#"{{"instruction":"{instruction}","max_turns":4,"max_tool_calls":3}}"#
                )
                .into_bytes()
            },
            context_bundles: vec![],
            context_refs: vec![],
        })
        .await
        .expect("invoke react-auto")
        .into_inner();

    let mut settled = None;
    for _ in 0..1500 {
        let t = c
            .list_react_turns(proto::ListReactTurnsRequest {
                limit: None,
                instance_id: Some(resp.instance_id.clone()),
                step_salt: None,
            })
            .await
            .unwrap()
            .into_inner();
        if t.turns
            .iter()
            .any(|x| x.branch == "answer" || x.branch == "dead_lettered")
        {
            settled = Some(t);
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let turns = settled.expect("the chain settled a terminal branch").turns;

    // Map each committed turn mote -> its result_ref to fetch the RAW emitted output.
    let view = c
        .get_projection(proto::GetProjectionRequest {
            instance_id: resp.instance_id.clone(),
            at_seq: None,
        })
        .await
        .unwrap()
        .into_inner();

    let mut fired: Vec<String> = Vec::new();
    for t in &turns {
        let raw = view
            .motes
            .iter()
            .find(|m| m.mote_id == t.turn_mote_id)
            .and_then(|m| m.result_ref.clone());
        let text = match raw {
            Some(rref) => c
                .get_content(proto::GetContentRequest {
                    content_ref: rref,
                    instance_id: resp.instance_id.clone(),
                })
                .await
                .ok()
                .map(|r| String::from_utf8_lossy(&r.into_inner().payload).into_owned())
                .unwrap_or_default(),
            None => String::new(),
        };
        eprintln!(
            "GRAMMAR-WITNESS turn={} branch={} tool_id={} raw={:?}",
            t.turn, t.branch, t.tool_id, text
        );
        if t.branch == "tool" {
            fired.push(t.tool_id.clone());
        }
    }
    eprintln!("GRAMMAR-WITNESS: fired tools = {fired:?}");

    running.shutdown().await.unwrap();
    std::env::remove_var("KX_SERVE_AUTOGRANT");

    // Observe-not-gate: a terminal branch with no crash is the bounded invariant.
    assert!(
        turns
            .iter()
            .any(|t| t.branch == "answer" || t.branch == "dead_lettered"),
        "the forcing chain settled a terminal branch"
    );
}

/// RC3 menu witness (Tier-B, observe-not-gate per GR16): the LEAD proof for
/// T-REACT-TOOL-MENU. Unlike the grammar witness above, the instruction describes
/// only the DESIRED EFFECT — NOT the tool name, args, or call format. Pre-RC3 a real
/// model could not fire a tool from such a goal (the live prompt showed NO tool
/// menu); RC3 prepends the granted-tool MENU (+ the curated agentic system prompt) so
/// the model proposes the call AUTONOMOUSLY, and the RC2 grammar then constrains it.
/// This PRINTS each turn's raw output + whether a tool fired (the headline signal,
/// observed not gated) and soft-asserts the chain settles. Run on BOTH engines
/// (restart-per-run): `KX_SERVE_OLLAMA=on KX_SERVE_OLLAMA_MODELS=gemma3:12b just
/// eval-real` and `KX_SERVE_MODEL_GGUF=<gemma-4-12b-it-q4_k_m.gguf> just eval-real`.
/// T-OLLAMA-GEMMA3-GGUF-SKEW: the llama.cpp arm needs the HF unsloth GGUF (`just
/// fetch-gemma-model`), NOT Ollama's gemma3 blob.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "real LLM; witnesses that the tool MENU (not a format hint) elicits an autonomous tool fire"]
async fn menu_elicits_a_tool_without_format_instructions() {
    if let Some(gguf) = serve_gguf() {
        std::env::set_var("KX_SERVE_MODEL_GGUF", &gguf);
    } else if !ollama_opted_in() {
        eprintln!("skipping: no model");
        return;
    }
    std::env::set_var("KX_SERVE_AUTOGRANT", "1");
    // The menu is on by default; assert it is NOT disabled for this witness.
    std::env::remove_var("KX_SERVE_REACT_TOOL_MENU");

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
    if !recipes
        .recipes
        .iter()
        .any(|r| r.handle == REACT_RECIPE_HANDLE)
    {
        eprintln!("skipping: react not provisioned (bundled bins missing)");
        running.shutdown().await.unwrap();
        std::env::remove_var("KX_SERVE_AUTOGRANT");
        return;
    }

    // Goal describes the EFFECT only — no tool name, no args, no envelope. A tool fire
    // here is attributable to the MENU, not to the instruction describing the format.
    let resp = c
        .invoke(proto::InvokeRequest {
            handle: REACT_AUTO_RECIPE_HANDLE.to_string(),
            args: {
                let instruction =
                    std::env::var("KX_MENU_WITNESS_INSTRUCTION").unwrap_or_else(|_| {
                        "Use your available tools to echo the word 'kortecx', then tell me \
                     exactly what the tool returned."
                            .to_string()
                    });
                format!(r#"{{"instruction":"{instruction}","max_turns":4,"max_tool_calls":3}}"#)
                    .into_bytes()
            },
            context_bundles: vec![],
            context_refs: vec![],
        })
        .await
        .expect("invoke react-auto")
        .into_inner();

    let mut settled = None;
    for _ in 0..1500 {
        let t = c
            .list_react_turns(proto::ListReactTurnsRequest {
                limit: None,
                instance_id: Some(resp.instance_id.clone()),
                step_salt: None,
            })
            .await
            .unwrap()
            .into_inner();
        if t.turns
            .iter()
            .any(|x| x.branch == "answer" || x.branch == "dead_lettered")
        {
            settled = Some(t);
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let turns = settled.expect("the chain settled a terminal branch").turns;

    let view = c
        .get_projection(proto::GetProjectionRequest {
            instance_id: resp.instance_id.clone(),
            at_seq: None,
        })
        .await
        .unwrap()
        .into_inner();

    let mut fired: Vec<String> = Vec::new();
    for t in &turns {
        let raw = view
            .motes
            .iter()
            .find(|m| m.mote_id == t.turn_mote_id)
            .and_then(|m| m.result_ref.clone());
        let text = match raw {
            Some(rref) => c
                .get_content(proto::GetContentRequest {
                    content_ref: rref,
                    instance_id: resp.instance_id.clone(),
                })
                .await
                .ok()
                .map(|r| String::from_utf8_lossy(&r.into_inner().payload).into_owned())
                .unwrap_or_default(),
            None => String::new(),
        };
        eprintln!(
            "MENU-WITNESS turn={} branch={} tool_id={} raw={:?}",
            t.turn, t.branch, t.tool_id, text
        );
        if t.branch == "tool" {
            fired.push(t.tool_id.clone());
        }
    }
    eprintln!(
        "MENU-WITNESS: fired tools (from a NO-format-hint goal) = {fired:?} \
         — non-empty proves the menu elicited an autonomous tool proposal"
    );

    running.shutdown().await.unwrap();
    std::env::remove_var("KX_SERVE_AUTOGRANT");

    assert!(
        turns
            .iter()
            .any(|t| t.branch == "answer" || t.branch == "dead_lettered"),
        "the menu-driven chain settled a terminal branch"
    );
}
