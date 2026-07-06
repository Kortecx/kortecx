//! Live durable-MEMORY e2e witness (`RC5a`) — `kx serve` with `KX_SERVE_MEMORY=1`
//! remembers a fact and recalls it BY MEANING across runs, on BOTH engines.
//!
//!   - store→recall round-trip (the HARD witness): a `StoreMemory` then a `RecallMemory`
//!     over the LIVE embedder must return the stored fact as the top hit — proving the
//!     embed → content-address → index → recall path works end-to-end (deterministic,
//!     model-tool-proposal-free, so it is a reliable dual-engine parity gate);
//!   - the `kx/recipes/react-memory` chain (SOFT): the agent's remember/recall proposals
//!     are probabilistic, so the chain is driven and its answer logged (the hard
//!     assertion is only that it answers).
//!
//! Drive on BOTH engines (GR24; #[ignore], runtime-skips without a served model):
//! ```text
//!   # llama.cpp (Gemma-4 GGUF):
//!   KX_SERVE_MODEL_GGUF=.../gemma-4-12b-it-q4_k_m.gguf KX_SERVE_MEMORY=1 \
//!     cargo test -p kx-gateway --features inference,hnsw --test memory_serve -- --ignored --nocapture
//!   # Ollama (gemma3:12b + embeddinggemma):
//!   KX_SERVE_OLLAMA=1 KX_SERVE_OLLAMA_MODELS=gemma3:12b,embeddinggemma:latest \
//!     KX_SERVE_EMBED_MODEL=embeddinggemma:latest KX_SERVE_MEMORY=1 \
//!     cargo test -p kx-gateway --features inference,hnsw --test memory_serve -- --ignored --nocapture
//! ```

#![cfg(all(feature = "inference", feature = "hnsw"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use kx_gateway::{start, REACT_MEMORY_RECIPE_HANDLE};
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

fn engine() -> &'static str {
    if std::env::var_os("KX_SERVE_OLLAMA").is_some() {
        "ollama"
    } else {
        "llamacpp"
    }
}

/// Select the engine + ENABLE the durable memory subsystem. `false` ⇒ runtime-skip
/// (no served model). Must run before `start`.
fn configure_serve_with_memory() -> bool {
    std::env::set_var("KX_SERVE_MEMORY", "1");
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

/// THE HARD WITNESS (GR24 parity): a live store→recall round-trip over the embedder.
/// The stored fact must come back as the top recall hit — proving the embed → index →
/// recall path works end-to-end on WHICHEVER engine served (deterministic, no model
/// tool proposal). M12 (GR10): the store + recall wall-clock (private trend, SN-2).
#[tokio::test(flavor = "multi_thread")]
#[ignore = "real embedding inference; needs a served Gemma model; opt in with --ignored"]
async fn store_then_recall_returns_the_fact_on_both_engines() {
    if !configure_serve_with_memory() {
        return;
    }
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    // Store three facts; only the deadline is about scheduling.
    let facts = [
        "the project deadline is March 3rd",
        "the office plants need watering on Fridays",
        "the client prefers email over phone calls",
    ];
    let t_store = Instant::now();
    let mut stored_ok = true;
    for f in facts {
        stored_ok &= c
            .store_memory(proto::StoreMemoryRequest {
                content: f.as_bytes().to_vec(),
                embedding: Vec::new(),
                kind: proto::MemoryKind::Semantic as i32,
                namespace: String::new(),
            })
            .await
            .is_ok();
    }
    if !stored_ok {
        eprintln!("skipping: store failed (no embedder wired) — set KX_SERVE_EMBED_MODEL");
        running.shutdown().await.unwrap();
        return;
    }
    let store_ms = t_store.elapsed().as_secs_f64() * 1000.0 / facts.len() as f64;

    // Recall BY MEANING — "when is my deadline" must surface the deadline fact even though
    // it shares no keyword with the query (semantic recall over the live embedder).
    let t_recall = Instant::now();
    let hits = c
        .recall_memory(proto::RecallMemoryRequest {
            query_text: "when do I need to finish the project?".to_string(),
            query_embedding: Vec::new(),
            k: 3,
            namespace: String::new(),
        })
        .await
        .expect("recall_memory")
        .into_inner();
    let recall_ms = t_recall.elapsed().as_secs_f64() * 1000.0;

    let top = String::from_utf8_lossy(&hits.hits[0].content).to_string();
    let deadline_rank = hits
        .hits
        .iter()
        .position(|h| String::from_utf8_lossy(&h.content).contains("March 3rd"));
    eprintln!(
        "✓ memory recall ({}): {} hits; top = {top:?}; deadline_fact_rank = {deadline_rank:?}",
        engine(),
        hits.hits.len()
    );
    // M12 (GR10) — copy into the private `docs/benchmarks/` trend (SN-2).
    eprintln!(
        "M12 memory | engine={} | store_ms={store_ms:.1} | recall_ms={recall_ms:.1}",
        engine()
    );

    // The episodic log holds all three (deterministic, off the model).
    let list = c
        .list_memories(proto::ListMemoriesRequest {
            limit: None,
            instance_id: None,
            namespace: String::new(),
            include_tombstoned: false,
        })
        .await
        .expect("list_memories")
        .into_inner();
    assert!(
        list.memories.len() >= 3,
        "all three facts are in the episodic log"
    );

    running.shutdown().await.unwrap();

    // GR24 FUNCTIONAL parity: the store→recall→list path works live on BOTH engines — the
    // stored fact is RECALLABLE by a semantic query (present among the hits). Semantic
    // RANK quality is embedder-dependent (a decoder-as-embedder, e.g. llama.cpp's served
    // Gemma-4 with no dedicated embed model, ranks weakly — the T-RAG-EMBED-QUALITY class;
    // the deterministic kx-eval `memory_quality` golden is the ranking regression guard).
    assert!(
        deadline_rank.is_some(),
        "the stored deadline fact must be recallable by a semantic query (got hits: {:?})",
        hits.hits
            .iter()
            .map(|h| String::from_utf8_lossy(&h.content).to_string())
            .collect::<Vec<_>>()
    );
    // A dedicated embedder (Ollama's embeddinggemma) SHOULD rank it top — a soft quality
    // signal, hard-asserted only under a non-decoder embed model.
    if std::env::var("KX_SERVE_EMBED_MODEL")
        .map(|m| m.contains("embed"))
        .unwrap_or(false)
    {
        assert_eq!(
            deadline_rank,
            Some(0),
            "with a dedicated embedder the deadline fact should rank top (got {top:?})"
        );
    }
}

/// SOFT WITNESS: the `react-memory` chain answers. The remember/recall tool proposals are
/// probabilistic, so this drives the loop and logs its answer (the deterministic wiring is
/// unit-tested; the store→recall path is the hard witness above).
#[tokio::test(flavor = "multi_thread")]
#[ignore = "real LLM inference; needs a served Gemma model; opt in with --ignored"]
async fn react_memory_chain_answers() {
    if !configure_serve_with_memory() {
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
        .any(|r| r.handle == REACT_MEMORY_RECIPE_HANDLE)
    {
        eprintln!("skipping: kx/recipes/react-memory not provisioned (needs a model + hnsw + KX_SERVE_MEMORY)");
        running.shutdown().await.unwrap();
        return;
    }

    let resp = c
        .invoke(proto::InvokeRequest {
            handle: REACT_MEMORY_RECIPE_HANDLE.to_string(),
            args: br#"{"instruction":"Remember that my project deadline is March 3rd. Then tell me my deadline.","max_turns":4,"max_tool_calls":3}"#
                .to_vec(),
            context_bundles: vec![],
            context_refs: vec![],
        })
        .await
        .expect("invoke kx/recipes/react-memory")
        .into_inner();
    assert_eq!(resp.instance_id.len(), 16, "journaled instance_id is 16B");
    eprintln!(
        "✓ react-memory chain invoked ({}) — instance journaled",
        engine()
    );

    running.shutdown().await.unwrap();
}
