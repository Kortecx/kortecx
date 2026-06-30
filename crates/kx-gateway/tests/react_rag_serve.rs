//! Live agentic-RAG e2e witness — `kx serve` drives a chain where the model
//! autonomously searches a dataset with the `retrieve` tool and answers grounded in
//! what it found. The hard assertion is that the chain ANSWERS; whether the model
//! fired `retrieve` is logged (a live tool proposal is probabilistic — the
//! deterministic wiring is unit-tested in `retrieve_tool` + `provision`).
//!
//! ```text
//! Flow: ingest a paraphrase corpus (server-embed) -> Invoke kx/recipes/react-rag
//!   -> the binder folds the dataset into the instruction, react_seed=true
//!   -> the coordinator seed-swap anchors the run-salted chain
//!   -> the worker runs REAL inference; on a retrieve proposal the broker runs the
//!      hybrid HostDatasetView::query and commits the passages as the Observation
//!   -> the chain settles a terminal Answer (durable facts via ListReactTurns).
//!
//! Drive on BOTH engines (GR24; #[ignore], runtime-skips without a served model):
//!   # llama.cpp (decoder-as-embedder honest-degrade, or set KX_SERVE_EMBED_MODEL):
//!   KX_SERVE_MODEL_GGUF=.../gemma-4-12b-it-q4_k_m.gguf \
//!     cargo test -p kx-gateway --features inference,hnsw --test react_rag_serve -- --ignored --nocapture
//!   # Ollama:
//!   KX_SERVE_OLLAMA=1 KX_SERVE_OLLAMA_MODELS=gemma3:12b,embeddinggemma:latest \
//!     KX_SERVE_EMBED_MODEL=embeddinggemma:latest \
//!     cargo test -p kx-gateway --features inference,hnsw --test react_rag_serve -- --ignored --nocapture
//! ```

#![cfg(all(feature = "inference", feature = "hnsw"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use kx_gateway::{start, REACT_RAG_RECIPE_HANDLE};
use kx_proto::proto;
use kx_proto::proto::kx_gateway_client::KxGatewayClient;
use tonic::transport::Channel;

/// The serve model GGUF (llama.cpp path): `KX_SERVE_MODEL_GGUF` if set, else the
/// public stand-in. `None` ⇒ runtime-skip (unless an Ollama serve is configured).
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

/// One server-embed document (empty embedding ⇒ the host embeds `content`).
fn doc(content: &[u8]) -> proto::IngestDocument {
    proto::IngestDocument {
        content: content.to_vec(),
        embedding: Vec::new(),
        ..Default::default()
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "real LLM inference + dataset embedding; needs a served Gemma model; opt in with --ignored"]
async fn react_rag_chain_searches_a_dataset_and_answers() {
    // The llama.cpp path needs a GGUF; the Ollama path is selected via KX_SERVE_OLLAMA.
    let ollama = std::env::var_os("KX_SERVE_OLLAMA").is_some();
    if !ollama {
        let Some(gguf) = serve_model() else {
            eprintln!("skipping: no serve model — set KX_SERVE_MODEL_GGUF or KX_SERVE_OLLAMA=1");
            return;
        };
        std::env::set_var("KX_SERVE_MODEL_GGUF", &gguf);
    }

    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    // react-rag is provisioned only on a model + hnsw serve (the retrieve capability).
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
        eprintln!("skipping: kx/recipes/react-rag not provisioned (needs a served model + hnsw)");
        running.shutdown().await.unwrap();
        return;
    }

    // Ingest a tiny paraphrase corpus (server-embed). The target doc never uses the
    // word "photosynthesis" — only the agent's own retrieve query can surface it.
    let ingest = c
        .ingest_documents(proto::IngestDocumentsRequest {
            dataset: "science".to_string(),
            documents: vec![
                doc(b"Plants turn sunlight, water, and carbon dioxide into sugar and oxygen inside their leaves."),
                doc(b"The mitochondria is the powerhouse of the cell, producing ATP from glucose."),
                doc(b"Tectonic plates drift over the mantle, causing earthquakes at their boundaries."),
            ],
        })
        .await;
    if ingest.is_err() {
        eprintln!("skipping: ingest failed (no embedder wired) — set KX_SERVE_EMBED_MODEL");
        running.shutdown().await.unwrap();
        return;
    }

    // Invoke react-rag: the dataset is folded into the instruction; the model must
    // search it with `retrieve` to answer (the corpus paraphrases the question).
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
    assert_eq!(resp.instance_id.len(), 16, "journaled instance_id is 16B");
    let step_salt = (!resp.react_chain_salt.is_empty()).then(|| resp.react_chain_salt.clone());

    // Await settle: the chain freezes a terminal Answer (be generous — inference is slow).
    let mut answered = false;
    let mut retrieved = false;
    for _ in 0..900 {
        let turns = c
            .list_react_turns(proto::ListReactTurnsRequest {
                limit: None,
                instance_id: Some(resp.instance_id.clone()),
                step_salt: step_salt.clone(),
            })
            .await
            .unwrap()
            .into_inner();
        for t in &turns.turns {
            if t.branch == "tool" && t.tool_id == "retrieve" {
                retrieved = true;
            }
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
        "the live agentic-RAG chain settled a terminal Answer"
    );
    // The agentic-RAG proof: did the model AUTONOMOUSLY call retrieve? Logged, not
    // hard-required (a live model's tool-proposal is probabilistic — RC3 menu + RC2
    // grammar make it likely; the deterministic wiring is unit-tested elsewhere).
    if retrieved {
        eprintln!("✓ agentic RAG: the model fired the `retrieve` tool autonomously");
    } else {
        eprintln!("· note: the model answered without firing `retrieve` (check the prompt/menu)");
    }

    running.shutdown().await.unwrap();
}
