//! Live RC5b e2e witness — `kx serve` with `KX_SERVE_MEMORY=1` consolidates episodic
//! memories into a durable semantic fact, and the decay/stats/restore RPC surface
//! responds, on BOTH engines.
//!
//!   - the decay/stats/restore RPCs (the HARD witness): deterministic, model-free — a
//!     store → `MemoryStats` → `DecayMemory --dry-run` → `RestoreMemory` round-trip
//!     smokes the whole RC5b RPC surface end-to-end (a reliable dual-engine parity gate;
//!     the tombstone/restore LOGIC is exhaustively unit-tested in `kx-memory` with an
//!     injected clock, since a live store stamps `created_ms = now` and nothing is old
//!     enough to evict);
//!   - the `kx/recipes/react-memory` CONSOLIDATION chain (SOFT): the model's
//!     bundle→distill→remember proposals are probabilistic, so the chain is driven and
//!     polled for a new semantic memory; if the model ignores the tool the chain still
//!     answers (soft-logged). The deterministic kx-eval `consolidation_quality` golden is
//!     the quality regression guard.
//!
//! Drive on BOTH engines (GR24; #[ignore], runtime-skips without a served model):
//! ```text
//!   # llama.cpp (Gemma-4 GGUF):
//!   KX_SERVE_MODEL_GGUF=.../gemma-4-12b-it-q4_k_m.gguf KX_SERVE_MEMORY=1 \
//!     cargo test -p kx-gateway --features inference,hnsw --test consolidate_serve -- --ignored --nocapture
//!   # Ollama (gemma3:12b + embeddinggemma):
//!   KX_SERVE_OLLAMA=1 KX_SERVE_OLLAMA_MODELS=gemma3:12b,embeddinggemma:latest \
//!     KX_SERVE_EMBED_MODEL=embeddinggemma:latest KX_SERVE_MEMORY=1 \
//!     cargo test -p kx-gateway --features inference,hnsw --test consolidate_serve -- --ignored --nocapture
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

/// The three episodic facts the agent consolidates.
const EPISODICS: [&str; 3] = [
    "on the Q3 launch call we set the deadline to March 3rd",
    "the client said they prefer email over phone for the launch",
    "we agreed to review launch metrics every Friday",
];

async fn store_episodics(c: &mut KxGatewayClient<Channel>) -> bool {
    let mut ok = true;
    for f in EPISODICS {
        ok &= c
            .store_memory(proto::StoreMemoryRequest {
                content: f.as_bytes().to_vec(),
                embedding: Vec::new(),
                kind: proto::MemoryKind::Episodic as i32,
                namespace: String::new(),
            })
            .await
            .is_ok();
    }
    ok
}

async fn semantic_count(c: &mut KxGatewayClient<Channel>) -> usize {
    c.list_memories(proto::ListMemoriesRequest {
        limit: Some(200),
        instance_id: None,
        namespace: String::new(),
        include_tombstoned: false,
    })
    .await
    .expect("list_memories")
    .into_inner()
    .memories
    .iter()
    .filter(|m| m.kind == "semantic")
    .count()
}

/// THE HARD WITNESS (GR24 parity): the RC5b decay/stats/restore RPCs respond live on
/// whichever engine served. Deterministic, model-free — a store → stats → dry-run decay
/// → restore round-trip. (Eviction/restore LOGIC is unit-tested with an injected clock;
/// a live store stamps `created_ms = now`, so nothing is old enough to actually evict.)
#[tokio::test(flavor = "multi_thread")]
#[ignore = "real embedding inference; needs a served Gemma model; opt in with --ignored"]
async fn decay_stats_restore_rpcs_respond_on_both_engines() {
    if !configure_serve_with_memory() {
        return;
    }
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    if !store_episodics(&mut c).await {
        eprintln!("skipping: store failed (no embedder wired) — set KX_SERVE_EMBED_MODEL");
        running.shutdown().await.unwrap();
        return;
    }

    // MemoryStats reflects the three live episodic memories.
    let stats = c
        .memory_stats(proto::MemoryStatsRequest {
            namespace: String::new(),
        })
        .await
        .expect("memory_stats")
        .into_inner();
    assert!(
        stats.total >= 3,
        "stats counts the stored memories (got {stats:?})"
    );
    assert!(stats.episodic >= 3, "the stored facts are episodic");
    assert_eq!(stats.tombstoned, 0, "nothing decayed yet");

    // DecayMemory --dry-run responds with a report; fresh facts are not old enough to evict.
    let t_decay = Instant::now();
    let decay = c
        .decay_memory(proto::DecayMemoryRequest {
            namespace: String::new(),
            ttl_days: 1,
            min_access: 1,
            dry_run: true,
        })
        .await
        .expect("decay_memory")
        .into_inner();
    let decay_ms = t_decay.elapsed().as_secs_f64() * 1000.0;
    assert!(decay.dry_run, "the sweep was a preview");
    assert_eq!(decay.evicted, 0, "a dry run evicts nothing");
    assert_eq!(
        decay.would_evict, 0,
        "freshly-stored memories are not past the TTL"
    );

    // RestoreMemory on an unknown id is an honest false (no tombstone to clear).
    let restore = c
        .restore_memory(proto::RestoreMemoryRequest {
            memory_id: vec![0u8; 32],
            namespace: String::new(),
        })
        .await
        .expect("restore_memory")
        .into_inner();
    assert!(!restore.restored, "restoring a non-tombstoned id is false");

    eprintln!(
        "✓ RC5b decay/stats/restore RPCs live ({}): {} live / {} tombstoned; decay_dryrun_ms={decay_ms:.1}",
        engine(),
        stats.total,
        stats.tombstoned
    );
    running.shutdown().await.unwrap();
}

/// SOFT WITNESS + M13 (GR10): drive the react-memory CONSOLIDATION chain and poll for a
/// new semantic memory (the distilled summary). The distillation is model-probabilistic,
/// so a missing semantic write is soft-logged (the deterministic kx-eval
/// `consolidation_quality` golden is the quality guard); the hard assertion is the chain
/// invokes + settles. M13 = the consolidation-round wall-clock (private trend, SN-2).
#[tokio::test(flavor = "multi_thread")]
#[ignore = "real LLM inference; needs a served Gemma model; opt in with --ignored"]
async fn consolidate_chain_distills_a_semantic_memory() {
    if !configure_serve_with_memory() {
        return;
    }
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    // react-memory must be provisioned (model + hnsw + KX_SERVE_MEMORY).
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
        eprintln!("skipping: kx/recipes/react-memory not provisioned");
        running.shutdown().await.unwrap();
        return;
    }
    if !store_episodics(&mut c).await {
        eprintln!("skipping: store failed (no embedder wired)");
        running.shutdown().await.unwrap();
        return;
    }
    let before = semantic_count(&mut c).await;

    // Drive the consolidation chain: bundle → distill → remember(kind=semantic).
    let t = Instant::now();
    let resp = c
        .invoke(proto::InvokeRequest {
            handle: REACT_MEMORY_RECIPE_HANDLE.to_string(),
            args: br#"{"instruction":"You have episodic memories from earlier that you CANNOT see until you retrieve them. FIRST call the consolidate tool to bundle your recent episodic memories about the Q3 launch. THEN distill the key durable facts and call remember with kind=\"semantic\" to save ONE concise summary. Only AFTER remembering, report what you consolidated. Do NOT answer from guesswork - you must use the tools.","max_turns":6,"max_tool_calls":4}"#
                .to_vec(),
            context_bundles: vec![],
            context_refs: vec![],
        })
        .await
        .expect("invoke react-memory consolidation")
        .into_inner();
    assert_eq!(resp.instance_id.len(), 16, "journaled instance_id is 16B");

    // Poll (≤90s) for a NEW semantic memory (the distilled summary). Soft — the model may
    // ignore the tool; the chain still answers.
    let mut wrote_semantic = false;
    for _ in 0..90 {
        if semantic_count(&mut c).await > before {
            wrote_semantic = true;
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    let round_ms = t.elapsed().as_secs_f64() * 1000.0;
    // Diagnostic (GR20): dump what the model actually proposed each turn — did it call
    // consolidate → remember, or answer without the tools?
    if let Ok(turns) = c
        .list_react_turns(proto::ListReactTurnsRequest {
            instance_id: Some(resp.instance_id.clone()),
            step_salt: Some(resp.react_chain_salt.clone()),
            limit: None,
        })
        .await
    {
        for t in turns.into_inner().turns.iter().rev() {
            eprintln!(
                "  turn {} branch={} tool={}@{}{}",
                t.turn,
                t.branch,
                t.tool_id,
                t.tool_version,
                if t.rejection_reason.is_empty() {
                    String::new()
                } else {
                    format!(" rejected={}", t.rejection_reason)
                }
            );
        }
    }
    eprintln!(
        "✓ consolidation chain ({}): wrote_semantic={wrote_semantic} (before={before})",
        engine()
    );
    // M13 (GR10) — copy into the private `docs/benchmarks/` trend (SN-2).
    eprintln!(
        "M13 consolidation | engine={} | round_ms={round_ms:.1}",
        engine()
    );

    if wrote_semantic {
        // The distilled summary is recallable by a paraphrase of the launch plan.
        let hits = c
            .recall_memory(proto::RecallMemoryRequest {
                query_text: "what is the plan for the launch?".to_string(),
                query_embedding: Vec::new(),
                k: 5,
                namespace: String::new(),
            })
            .await
            .expect("recall_memory")
            .into_inner();
        eprintln!(
            "  consolidated summary recall: {} hits; top = {:?}",
            hits.hits.len(),
            hits.hits
                .first()
                .map(|h| String::from_utf8_lossy(&h.content).to_string())
        );
    }

    running.shutdown().await.unwrap();
}
