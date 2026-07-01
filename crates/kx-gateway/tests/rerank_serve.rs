//! Live LLM-rerank e2e witness (RC4c-2b) — `kx serve` with `KX_SERVE_RAG_LLM_RERANK=1`
//! reranks the retrieved passages via a DURABLE, replayable coordinator rerank-turn, on
//! BOTH live RAG paths:
//!   - chat-rag: the grounded answer is HELD until a rerank of its context bundle settles
//!     (the hard witness — chat-rag always grounds, so a `ReRankRound` MUST fire);
//!   - react-rag: the agent's `retrieve@1` observation is reranked before the next turn
//!     (soft — the model's retrieve proposal is probabilistic, so the rerank is logged).
//!
//! Drive on BOTH engines (GR24; #[ignore], runtime-skips without a served model):
//! ```text
//!   # llama.cpp (Gemma-4 GGUF; degrade-to-parser on the permutation grammar):
//!   KX_SERVE_MODEL_GGUF=.../gemma-4-12b-it-q4_k_m.gguf KX_SERVE_RAG_LLM_RERANK=1 \
//!     cargo test -p kx-gateway --features inference,hnsw --test rerank_serve -- --ignored --nocapture
//!   # Ollama (gemma3:12b; strict whole-response `format`):
//!   KX_SERVE_OLLAMA=1 KX_SERVE_OLLAMA_MODELS=gemma3:12b,embeddinggemma:latest \
//!     KX_SERVE_EMBED_MODEL=embeddinggemma:latest KX_SERVE_RAG_LLM_RERANK=1 \
//!     cargo test -p kx-gateway --features inference,hnsw --test rerank_serve -- --ignored --nocapture
//! ```

#![cfg(all(feature = "inference", feature = "hnsw"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use kx_gateway::{start, CHAT_RAG_RECIPE_HANDLE, REACT_RAG_RECIPE_HANDLE};
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

fn doc(content: &[u8]) -> proto::IngestDocument {
    proto::IngestDocument {
        content: content.to_vec(),
        embedding: Vec::new(),
        ..Default::default()
    }
}

/// Select the engine (llama.cpp GGUF or Ollama) + FORCE the LLM rerank on. Returns
/// `false` ⇒ runtime-skip (no served model). Must run before `start`.
fn configure_serve_with_rerank() -> bool {
    std::env::set_var("KX_SERVE_RAG_LLM_RERANK", "1");
    if std::env::var_os("KX_SERVE_OLLAMA").is_some() {
        return true;
    }
    match serve_model() {
        Some(gguf) => {
            std::env::set_var("KX_SERVE_MODEL_GGUF", &gguf);
            true
        }
        None => {
            eprintln!("skipping: no serve model — set KX_SERVE_MODEL_GGUF or KX_SERVE_OLLAMA=1");
            false
        }
    }
}

async fn ingest_science(c: &mut KxGatewayClient<Channel>) -> bool {
    c.ingest_documents(proto::IngestDocumentsRequest {
        dataset: "science".to_string(),
        documents: vec![
            doc(b"Tectonic plates drift over the mantle, causing earthquakes at their boundaries."),
            doc(b"The mitochondria is the powerhouse of the cell, producing ATP from glucose."),
            doc(b"Plants turn sunlight, water, and carbon dioxide into sugar and oxygen inside their leaves."),
            doc(b"The Great Barrier Reef is the world's largest coral reef system off Australia."),
        ],
    })
    .await
    .is_ok()
}

/// CHAT-RAG (the HARD witness): a grounded answer is HELD until a durable rerank of its
/// context bundle settles, so a `ReRankRound` with a terminal outcome MUST appear.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "real LLM inference + dataset embedding; needs a served Gemma model; opt in with --ignored"]
async fn chat_rag_grounded_answer_is_reranked() {
    if !configure_serve_with_rerank() {
        return;
    }
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
        .any(|r| r.handle == CHAT_RAG_RECIPE_HANDLE)
    {
        eprintln!("skipping: kx/recipes/chat-rag not provisioned (needs a served model + hnsw)");
        running.shutdown().await.unwrap();
        return;
    }
    if !ingest_science(&mut c).await {
        eprintln!("skipping: ingest failed (no embedder wired) — set KX_SERVE_EMBED_MODEL");
        running.shutdown().await.unwrap();
        return;
    }

    // M7c (GR10): the LIVE wall-clock from submit → the durable `ReRankRound` settling —
    // model-INCLUSIVE (unlike M7a/M7b, the rerank turn is coordinator-materialized, not
    // client-submittable, so it cannot be driven model-free; this is the real dual-engine
    // number for the private trend). Structured, greppable; no hard threshold.
    let t_submit = Instant::now();
    let resp = c
        .invoke(proto::InvokeRequest {
            handle: CHAT_RAG_RECIPE_HANDLE.to_string(),
            args: br#"{"prompt":"How do plants make energy from the sun? Answer briefly using the dataset.","dataset":"science"}"#
                .to_vec(),
            context_bundles: vec![],
            context_refs: vec![],
        })
        .await
        .expect("invoke kx/recipes/chat-rag")
        .into_inner();
    assert_eq!(resp.instance_id.len(), 16, "journaled instance_id is 16B");

    // Await the rerank round (be generous — a 12B rerank turn is slow).
    let engine = if std::env::var_os("KX_SERVE_OLLAMA").is_some() {
        "ollama"
    } else {
        "llamacpp"
    };
    let mut outcome: Option<String> = None;
    for _ in 0..3000 {
        let rounds = c
            .list_re_rank_turns(proto::ListReRankTurnsRequest {
                limit: None,
                instance_id: Some(resp.instance_id.clone()),
            })
            .await
            .unwrap()
            .into_inner();
        if let Some(t) = rounds.turns.iter().find(|t| t.outcome != "pending") {
            outcome = Some(t.outcome.clone());
            let settled_ms = t_submit.elapsed().as_secs_f64() * 1000.0;
            eprintln!(
                "✓ chat-rag rerank fired: outcome={} candidates={} permutation={:?}",
                t.outcome, t.candidate_count, t.permutation
            );
            // M7c — copy into the private `docs/benchmarks/` trend (SN-2).
            eprintln!(
                "M7c rerank | engine={engine} | candidates={} | submit_to_settled_ms={settled_ms:.1} | outcome={}",
                t.candidate_count, t.outcome
            );
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    running.shutdown().await.unwrap();
    let outcome = outcome.expect("a chat-rag ReRankRound must settle");
    // RC4c-2c (GR24 parity gate): BOTH engines — llama.cpp Gemma-4 (grammar-free, the
    // chat-template fix) AND Ollama gemma3 (strict `format`) — must ACTUALLY rerank the
    // chat-rag context bundle, not fail-closed to base order. Before RC4c-2c llama.cpp
    // fail-closed here (an un-templated prompt degenerated the instruct model).
    assert_eq!(
        outcome, "reranked",
        "the chat-rag rerank must settle `reranked`, got {outcome}"
    );
}

/// REACT-RAG (soft): the agent's `retrieve@1` observation is reranked before the next
/// turn. The retrieve proposal is probabilistic, so the rerank is LOGGED; the hard
/// assertion is only that the chain answers.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "real LLM inference + dataset embedding; needs a served Gemma model; opt in with --ignored"]
async fn react_rag_reranks_the_retrieved_passages() {
    if !configure_serve_with_rerank() {
        return;
    }
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
        .any(|r| r.handle == REACT_RAG_RECIPE_HANDLE)
    {
        eprintln!("skipping: kx/recipes/react-rag not provisioned");
        running.shutdown().await.unwrap();
        return;
    }
    if !ingest_science(&mut c).await {
        eprintln!("skipping: ingest failed — set KX_SERVE_EMBED_MODEL");
        running.shutdown().await.unwrap();
        return;
    }

    let resp = c
        .invoke(proto::InvokeRequest {
            handle: REACT_RAG_RECIPE_HANDLE.to_string(),
            args: br#"{"instruction":"How do plants make energy from the sun? Answer briefly using the dataset.","dataset":"science","max_turns":4,"max_tool_calls":3}"#
                .to_vec(),
            context_bundles: vec![],
            context_refs: vec![],
        })
        .await
        .expect("invoke kx/recipes/react-rag")
        .into_inner();
    let step_salt = (!resp.react_chain_salt.is_empty()).then(|| resp.react_chain_salt.clone());

    let mut answered = false;
    let mut reranked = false;
    for _ in 0..3000 {
        let turns = c
            .list_react_turns(proto::ListReactTurnsRequest {
                limit: None,
                instance_id: Some(resp.instance_id.clone()),
                step_salt: step_salt.clone(),
            })
            .await
            .unwrap()
            .into_inner();
        answered = turns.turns.iter().any(|t| t.branch == "answer");
        let rounds = c
            .list_re_rank_turns(proto::ListReRankTurnsRequest {
                limit: None,
                instance_id: Some(resp.instance_id.clone()),
            })
            .await
            .unwrap()
            .into_inner();
        if rounds.turns.iter().any(|t| t.outcome == "reranked") {
            reranked = true;
        }
        if answered {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    running.shutdown().await.unwrap();
    assert!(answered, "the react-rag chain settled a terminal Answer");
    if reranked {
        eprintln!("✓ react-rag: the retrieved passages were durably reranked");
    } else {
        eprintln!("· note: no rerank fired (the model may not have called retrieve this run)");
    }
}
