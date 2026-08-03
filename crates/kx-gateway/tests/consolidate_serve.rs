//! Live memory-lifecycle e2e witness — `kx serve` with `KX_SERVE_MEMORY=1` decays stale
//! memories, spares the recalled ones, restores what it tombstoned, and consolidates
//! episodic memories into a durable semantic fact, on BOTH engines.
//!
//!   - the decay/stats/restore LIFECYCLE (the HARD witness): store → recall ONE → age the
//!     rows → `MemoryStats` → `DecayMemory --dry-run` → `DecayMemory` → `RestoreMemory`.
//!     Every assertion is about what SURVIVED, never that an RPC answered. The recalled
//!     memory is the surviving witness and the ONLY variable between it and the evicted
//!     pair is the recall, so the arm is its own one-variable accepting control.
//!   - the `kx/recipes/react-memory` CONSOLIDATION chain: the chain must WRITE a semantic
//!     memory and that memory must be RECALLABLE by a paraphrase. M13 = the consolidation
//!     round wall-clock (private trend).
//!
//! ## Why the age is injected, and what is NOT injected
//! `DecayMemory` clamps `ttl_days == 0` to 90 and floors the rest at 1 day, so NOTHING a
//! test stores can ever age past the policy inside a run. The single injected variable is
//! therefore `created_ms`, backdated in the gateway's OWN `memory.db` between the store and
//! the sweep — the live analogue of the injected clock the `kx-memory` unit tests use.
//! Store, recall, stats, decay, restore and list all travel the REAL RPC path, and the
//! backdate offsets are derived from the server's own reported `created_ms`, never pinned.
//!
//! Drive on BOTH engines (#[ignore], and it FAILS rather than skips without a served model):
//! ```text
//!   # llama.cpp (Gemma-4 GGUF):
//!   KX_SERVE_MODEL_GGUF=.../gemma-4-12b-it-q4_k_m.gguf KX_SERVE_MEMORY=1 \
//!     cargo test -p kx-gateway --features inference,hnsw --test consolidate_serve -- --ignored --nocapture
//!   # Ollama (gemma4:12b + embeddinggemma) — no C++ toolchain needed:
//!   KX_SERVE_OLLAMA=on KX_SERVE_OLLAMA_MODELS=gemma4:12b,embeddinggemma:latest \
//!     KX_SERVE_EMBED_MODEL=embeddinggemma:latest KX_SERVE_MEMORY=1 \
//!     cargo test -p kx-gateway --features serve-engine,hnsw --test consolidate_serve -- --ignored --nocapture
//! ```

#![cfg(all(feature = "serve-engine", feature = "hnsw"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
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

/// The gateway's own two-gate connect (TCP accept, then the H2 handshake). The local
/// one-second connect loop this replaces is the known CI flake — the helper's own doc
/// records it, and it was still copied into every file in this directory.
async fn client(addr: SocketAddr) -> KxGatewayClient<Channel> {
    common::connect_client(addr).await
}

fn engine() -> &'static str {
    if std::env::var_os("KX_SERVE_OLLAMA").is_some() {
        "ollama"
    } else {
        "llamacpp"
    }
}

/// Which embedder produced the vectors in this run. An unset `KX_SERVE_EMBED_MODEL`
/// silently falls back to the CHAT primary, which is a decoder — every retrieval number
/// in this file is reported beside this string so it can never be read as unattributed.
fn embedder() -> String {
    std::env::var("KX_SERVE_EMBED_MODEL").unwrap_or_else(|_| "(unset — the chat primary)".into())
}

/// Select the engine and turn memory on. FAILS rather than skips: a green from a run
/// where no model was ever served is indistinguishable from a green where everything
/// worked, and that is precisely the shape this file exists to stop producing.
fn configure_serve_with_memory() {
    std::env::set_var("KX_SERVE_MEMORY", "1");
    if std::env::var_os("KX_SERVE_OLLAMA").is_some() {
        return;
    }
    let gguf = serve_model().expect(
        "PRECONDITION: no serve model. This is a LIVE oracle — set KX_SERVE_MODEL_GGUF to a \
         GGUF, or KX_SERVE_OLLAMA=on with KX_SERVE_OLLAMA_MODELS. It fails instead of \
         skipping because a skip is indistinguishable from a pass.",
    );
    std::env::set_var("KX_SERVE_MODEL_GGUF", &gguf);
}

/// The three episodic facts the agent consolidates.
const EPISODICS: [&str; 3] = [
    "on the Q3 launch call we set the deadline to March 3rd",
    "the client said they prefer email over phone for the launch",
    "we agreed to review launch metrics every Friday",
];

/// Store the episodics through the real RPC. Every write must succeed — a failed store
/// used to become a silent `return`, so the whole lifecycle could be skipped by an
/// unwired embedder and still report green.
async fn store_episodics(c: &mut KxGatewayClient<Channel>) {
    for f in EPISODICS {
        c.store_memory(proto::StoreMemoryRequest {
            content: f.as_bytes().to_vec(),
            embedding: Vec::new(),
            kind: proto::MemoryKind::Episodic as i32,
            namespace: String::new(),
        })
        .await
        .unwrap_or_else(|e| {
            panic!(
                "PRECONDITION: StoreMemory failed on [{}] with embedder {} — set \
                 KX_SERVE_EMBED_MODEL to a dedicated embedder (e.g. embeddinggemma:latest). \
                 Status: {e}",
                engine(),
                embedder()
            )
        });
    }
}

async fn list_memories(
    c: &mut KxGatewayClient<Channel>,
    include_tombstoned: bool,
) -> Vec<proto::MemorySummary> {
    c.list_memories(proto::ListMemoriesRequest {
        limit: Some(200),
        instance_id: None,
        namespace: String::new(),
        include_tombstoned,
    })
    .await
    .expect("list_memories")
    .into_inner()
    .memories
}

async fn semantic_count(c: &mut KxGatewayClient<Channel>) -> usize {
    list_memories(c, false)
        .await
        .iter()
        .filter(|m| m.kind == "semantic")
        .count()
}

async fn stats(c: &mut KxGatewayClient<Channel>) -> proto::MemoryStatsResponse {
    c.memory_stats(proto::MemoryStatsRequest {
        namespace: String::new(),
    })
    .await
    .expect("memory_stats")
    .into_inner()
}

/// The gateway's OWN catalog directory, as the server reports it — never re-derived from
/// the test's tempdir, so the file we age is provably the file the server opened.
async fn served_memory_db(c: &mut KxGatewayClient<Channel>) -> PathBuf {
    let info = c
        .get_server_info(proto::GetServerInfoRequest {})
        .await
        .expect("get_server_info")
        .into_inner();
    assert!(
        info.feature_hnsw,
        "PRECONDITION: the serve reports feature_hnsw=false — the memory data-plane is absent"
    );
    let db = Path::new(&info.catalog_dir)
        .join("memory")
        .join("memory.db");
    assert!(
        db.is_file(),
        "PRECONDITION: the serve reported catalog_dir={} but {} does not exist — \
         the memory store was never opened",
        info.catalog_dir,
        db.display()
    );
    db
}

/// Backdate `created_ms` for exactly the named memories, by an offset derived from the
/// server's OWN reported `created_ms`. This is the one injected variable in the decay
/// arm: `DecayMemory` floors its TTL at one day, so no memory a test can create is ever
/// old enough to sweep. Everything else travels the real RPC path.
///
/// Returns the number of rows actually rewritten so the caller can assert the injection
/// LANDED — an UPDATE that matched nothing would otherwise leave a sweep that evicts
/// nothing looking exactly like a sweep that works.
fn age_memories(db: &Path, ids: &[Vec<u8>], by_ms: i64) -> usize {
    let conn = rusqlite::Connection::open(db).expect("open the served memory.db");
    conn.busy_timeout(Duration::from_secs(10))
        .expect("busy_timeout");
    let mut rewritten = 0usize;
    for id in ids {
        rewritten += conn
            .execute(
                "UPDATE memories SET created_ms = created_ms - ?1 WHERE memory_id = ?2",
                rusqlite::params![by_ms, id.as_slice()],
            )
            .expect("backdate created_ms");
    }
    rewritten
}

/// THE HARD WITNESS (dual-engine): a real eviction, a real restore, and an assertion about
/// what SURVIVED both.
///
/// The shape is a one-variable A/B. Three episodics are stored identically; exactly ONE is
/// recalled, which bumps its salience counter; all three are then aged past the TTL by the
/// same offset. Under `min_access = 1` the two unrecalled memories are swept and the
/// recalled one is spared — so the surviving arm IS the accepting control for the evicting
/// arm, and the only difference between them is the recall.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "real embedding inference; needs a served model; opt in with --ignored"]
async fn decay_evicts_the_stale_spares_the_recalled_and_restore_returns_it() {
    configure_serve_with_memory();
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;
    let db = served_memory_db(&mut c).await;

    store_episodics(&mut c).await;

    // The starting picture, from the server's own report.
    let before = list_memories(&mut c, false).await;
    assert_eq!(
        before.len(),
        EPISODICS.len(),
        "the three stored episodics are all live [{}]",
        engine()
    );
    assert!(
        before.iter().all(|m| m.access_count == 0),
        "nothing has been recalled yet, so every salience counter is 0 [{}]",
        engine()
    );
    let s0 = stats(&mut c).await;
    assert_eq!(s0.total, 3, "stats counts the three live memories");
    assert_eq!(s0.episodic, 3, "the stored facts are episodic");
    assert_eq!(s0.tombstoned, 0, "nothing decayed yet");

    // ── The single variable: recall exactly ONE memory. k=1 so the salience bump lands
    // on exactly one row, and the RESPONSE tells us which — the survivor is the system's
    // own choice, never a value this test picked.
    let hits = c
        .recall_memory(proto::RecallMemoryRequest {
            query_text: "when is the launch deadline?".to_string(),
            query_embedding: Vec::new(),
            k: 1,
            namespace: String::new(),
        })
        .await
        .unwrap_or_else(|e| {
            panic!(
                "PRECONDITION: RecallMemory failed on [{}] with embedder {} — {e}",
                engine(),
                embedder()
            )
        })
        .into_inner();
    assert_eq!(
        hits.hits.len(),
        1,
        "k=1 recall returns exactly one hit [{}] (embedder {})",
        engine(),
        embedder()
    );
    let survivor_id = hits.hits[0].memory_id.clone();
    let survivor_content = hits.hits[0].content.clone();

    // The salience bump is visible on the wire, and ONLY on the recalled row.
    let after_recall = list_memories(&mut c, false).await;
    let bumped: Vec<_> = after_recall
        .iter()
        .filter(|m| m.access_count >= 1)
        .collect();
    assert_eq!(
        bumped.len(),
        1,
        "exactly one memory was recalled, so exactly one salience counter moved [{}] — \
         counts were {:?}",
        engine(),
        after_recall
            .iter()
            .map(|m| m.access_count)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        bumped[0].memory_id,
        survivor_id,
        "the bumped row is the one recall returned [{}]",
        engine()
    );

    // ── Age every memory past the one-day floor, by an offset derived from the server's
    // own `created_ms`. Assert the injection LANDED: an UPDATE that matched nothing would
    // make the sweep below evict nothing and read exactly like a working sweep.
    let oldest = after_recall.iter().map(|m| m.created_ms).min().unwrap();
    let now_ms = i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap();
    // Two days older than the oldest row is comfortably past a one-day TTL, whatever the
    // wall clock says when the sweep runs.
    let by_ms = (now_ms - oldest) + 2 * 86_400_000;
    let ids: Vec<Vec<u8>> = after_recall.iter().map(|m| m.memory_id.clone()).collect();
    let rewritten = age_memories(&db, &ids, by_ms);
    assert_eq!(
        rewritten,
        EPISODICS.len(),
        "the backdate must rewrite all three rows in {} — it rewrote {rewritten}",
        db.display()
    );
    let aged = list_memories(&mut c, false).await;
    assert!(
        aged.iter().all(|m| now_ms - m.created_ms > 86_400_000),
        "every memory now reads as older than the one-day TTL floor [{}] — created_ms {:?}",
        engine(),
        aged.iter()
            .map(|m| now_ms - m.created_ms)
            .collect::<Vec<_>>()
    );

    // ── DRY RUN: a NON-ZERO preview that changes nothing. The previous version of this
    // test asserted `would_evict == 0`, which reads identically to a no-op stub.
    let t_decay = Instant::now();
    let preview = c
        .decay_memory(proto::DecayMemoryRequest {
            namespace: String::new(),
            ttl_days: 1,
            min_access: 1,
            dry_run: true,
        })
        .await
        .expect("decay_memory (dry run)")
        .into_inner();
    let decay_ms = t_decay.elapsed().as_secs_f64() * 1000.0;
    assert!(preview.dry_run, "the sweep echoes that it was a preview");
    assert_eq!(
        preview.would_evict,
        2,
        "the two UNRECALLED memories are past the TTL and under the salience floor [{}]",
        engine()
    );
    assert_eq!(
        preview.kept,
        1,
        "the recalled memory is spared [{}]",
        engine()
    );
    assert_eq!(
        preview.evicted,
        0,
        "a dry run writes nothing [{}]",
        engine()
    );
    assert!(
        preview
            .candidates
            .iter()
            .all(|cand| cand.memory_id != survivor_id),
        "the recalled memory is never a candidate [{}]",
        engine()
    );
    // Nothing moved: the preview is a preview.
    let s1 = stats(&mut c).await;
    assert_eq!(
        (s1.total, s1.tombstoned),
        (s0.total, s0.tombstoned),
        "a dry run leaves the store byte-for-byte as it found it [{}]",
        engine()
    );

    // ── THE REAL SWEEP.
    let swept = c
        .decay_memory(proto::DecayMemoryRequest {
            namespace: String::new(),
            ttl_days: 1,
            min_access: 1,
            dry_run: false,
        })
        .await
        .expect("decay_memory (sweep)")
        .into_inner();
    assert!(!swept.dry_run, "this sweep was real [{}]", engine());
    assert_eq!(
        swept.evicted,
        2,
        "two memories were tombstoned [{}]",
        engine()
    );
    assert_eq!(swept.kept, 1, "one memory survived [{}]", engine());

    // What SURVIVED, asserted four ways.
    let s2 = stats(&mut c).await;
    assert_eq!(
        s2.tombstoned,
        2,
        "stats reports the two tombstones [{}]",
        engine()
    );
    assert_eq!(s2.total, 1, "one live memory remains [{}]", engine());

    let live = list_memories(&mut c, false).await;
    assert_eq!(
        live.len(),
        1,
        "the default view hides the tombstoned [{}]",
        engine()
    );
    assert_eq!(
        live[0].memory_id,
        survivor_id,
        "the survivor is exactly the recalled memory [{}]",
        engine()
    );

    let with_tombstones = list_memories(&mut c, true).await;
    assert_eq!(
        with_tombstones.len(),
        3,
        "the tombstoned rows are retained, not deleted [{}]",
        engine()
    );
    let tombstoned: Vec<_> = with_tombstones
        .iter()
        .filter(|m| m.tombstoned_ms > 0)
        .collect();
    assert_eq!(
        tombstoned.len(),
        2,
        "two rows carry a tombstone [{}]",
        engine()
    );

    // The survivor is still RECALLABLE — the eviction did not damage the live index.
    let post = c
        .recall_memory(proto::RecallMemoryRequest {
            query_text: "when is the launch deadline?".to_string(),
            query_embedding: Vec::new(),
            k: 5,
            namespace: String::new(),
        })
        .await
        .expect("recall after the sweep")
        .into_inner();
    assert_eq!(
        post.hits.len(),
        1,
        "only the survivor is recallable; a tombstone is never surfaced [{}] (embedder {})",
        engine(),
        embedder()
    );
    assert_eq!(
        post.hits[0].content,
        survivor_content,
        "the survivor's content came back unchanged [{}]",
        engine()
    );

    // ── RESTORE brings one tombstone back, and it is recallable again.
    let restored_id = tombstoned[0].memory_id.clone();
    let restored_content = tombstoned[0].content.clone();
    let restore = c
        .restore_memory(proto::RestoreMemoryRequest {
            memory_id: restored_id.clone(),
            namespace: String::new(),
        })
        .await
        .expect("restore_memory")
        .into_inner();
    assert!(
        restore.restored,
        "restoring a tombstoned memory reports true [{}]",
        engine()
    );
    let s3 = stats(&mut c).await;
    assert_eq!(s3.tombstoned, 1, "one tombstone was cleared [{}]", engine());
    assert_eq!(
        s3.total,
        2,
        "the restored memory is live again [{}]",
        engine()
    );
    let back = list_memories(&mut c, false).await;
    assert!(
        back.iter().any(|m| m.memory_id == restored_id),
        "the restored memory is back in the default view [{}]",
        engine()
    );
    // The assertion that matters: it is RECALLABLE, not merely listed. Restore rehydrates
    // the in-memory content projection; a restore that only cleared the tombstone column
    // would satisfy every assertion above this one and still leave the memory unreachable.
    let recalled_back = c
        .recall_memory(proto::RecallMemoryRequest {
            query_text: String::from_utf8_lossy(&restored_content).to_string(),
            query_embedding: Vec::new(),
            k: 5,
            namespace: String::new(),
        })
        .await
        .expect("recall the restored memory")
        .into_inner();
    assert!(
        recalled_back
            .hits
            .iter()
            .any(|h| h.memory_id == restored_id),
        "the restored memory is RECALLABLE again, not merely listed [{}] (embedder {}) — \
         got {} hits",
        engine(),
        embedder(),
        recalled_back.hits.len()
    );

    // ── The negative control, kept: an unknown id restores nothing. Without it, a
    // `restore` that returned `true` unconditionally would pass everything above.
    let bogus = c
        .restore_memory(proto::RestoreMemoryRequest {
            memory_id: vec![0u8; 32],
            namespace: String::new(),
        })
        .await
        .expect("restore_memory (unknown id)")
        .into_inner();
    assert!(
        !bogus.restored,
        "an id that was never tombstoned restores nothing [{}]",
        engine()
    );

    eprintln!(
        "✓ memory lifecycle [{}] embedder={}: evicted=2 spared=1 restored=1 \
         (survivor = the recalled memory); decay_dryrun_ms={decay_ms:.1}",
        engine(),
        embedder()
    );
    running.shutdown().await.unwrap();
}

/// The consolidation chain must WRITE a semantic memory and that memory must be
/// RECALLABLE. Both were previously soft: the only hard assertion was that the journaled
/// `instance_id` is 16 bytes, and the outcome the test exists to check was `eprintln!`ed.
/// M13 = the consolidation-round wall-clock (private trend).
#[tokio::test(flavor = "multi_thread")]
#[ignore = "real LLM inference; needs a served model; opt in with --ignored"]
async fn consolidate_chain_distills_a_semantic_memory() {
    configure_serve_with_memory();
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    // react-memory must be provisioned (model + hnsw + KX_SERVE_MEMORY). This is a
    // PRECONDITION, not a skip: "the recipe was absent" and "the chain worked" used to
    // produce the same green.
    let recipes = c
        .list_recipes(proto::ListRecipesRequest {})
        .await
        .unwrap()
        .into_inner();
    assert!(
        recipes
            .recipes
            .iter()
            .any(|r| r.handle == REACT_MEMORY_RECIPE_HANDLE),
        "PRECONDITION: {REACT_MEMORY_RECIPE_HANDLE} is not provisioned on [{}] — it needs a \
         served model, hnsw and KX_SERVE_MEMORY=1. Provisioned: {:?}",
        engine(),
        recipes
            .recipes
            .iter()
            .map(|r| &r.handle)
            .collect::<Vec<_>>()
    );
    store_episodics(&mut c).await;
    let before = semantic_count(&mut c).await;

    // Drive the consolidation chain: bundle → distill → remember(kind=semantic).
    let t = Instant::now();
    let resp = c
        .invoke(proto::InvokeRequest {
            handle: REACT_MEMORY_RECIPE_HANDLE.to_string(),
            // ⚠ The instruction is part of the FIXTURE and it took a rewrite to get right.
            // Its first version opened "you have episodic memories you cannot see until you
            // RETRIEVE them", and the model duly called `recall` — twice, the second time
            // rejected as a duplicate — and then answered without ever consolidating. The
            // word primed the wrong tool. Both tools are granted and registered; the menu
            // was never the problem. Name the tools, in order, and say what not to do.
            args: br#"{"instruction":"Do these two steps in order, using the tools.\nSTEP 1: call the `consolidate` tool with kind_filter=\"episodic\" to bundle your recent episodic memories about the Q3 launch. Do NOT call `recall` - consolidate is the tool that bundles them.\nSTEP 2: read the bundled entries, distill the durable facts into ONE concise sentence, and call `remember` with kind=\"semantic\" and that sentence as the content.\nOnly after both tool calls, answer with what you saved.","max_turns":6,"max_tool_calls":4}"#
                .to_vec(),
            context_bundles: vec![],
            context_refs: vec![],
        })
        .await
        .expect("invoke react-memory consolidation")
        .into_inner();
    assert_eq!(resp.instance_id.len(), 16, "journaled instance_id is 16B");

    // Poll (≤90s) for a NEW semantic memory — the distilled summary.
    let mut wrote_semantic = false;
    for _ in 0..90 {
        if semantic_count(&mut c).await > before {
            wrote_semantic = true;
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    let round_ms = t.elapsed().as_secs_f64() * 1000.0;
    // Diagnostic: dump what the model actually proposed each turn — did it call
    // consolidate → remember, or answer without the tools? Printed BEFORE the assertion
    // so a failure carries its own diagnosis.
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
    assert!(
        wrote_semantic,
        "the consolidation chain must WRITE a semantic memory on [{}] (embedder {}) — \
         semantic count stayed at {before} for 90s. The turn dump above says whether the \
         model called consolidate → remember or answered without the tools.",
        engine(),
        embedder()
    );

    // And the distilled summary must be RECALLABLE by a paraphrase of the launch plan —
    // a semantic row that no query can reach is not a consolidation.
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
    let semantic_ids: Vec<Vec<u8>> = list_memories(&mut c, false)
        .await
        .into_iter()
        .filter(|m| m.kind == "semantic")
        .map(|m| m.memory_id)
        .collect();
    assert!(
        hits.hits
            .iter()
            .any(|h| semantic_ids.contains(&h.memory_id)),
        "the consolidated summary is recallable by a paraphrase on [{}] (embedder {}) — \
         {} hits, none of them the {} semantic memor(y/ies)",
        engine(),
        embedder(),
        hits.hits.len(),
        semantic_ids.len()
    );

    eprintln!(
        "✓ consolidation chain [{}] embedder={}: wrote_semantic={wrote_semantic} \
         (before={before}) and the summary is recallable",
        engine(),
        embedder()
    );
    // M13 — copy into the private `docs/benchmarks/` trend.
    eprintln!(
        "M13 consolidation | engine={} | round_ms={round_ms:.1}",
        engine()
    );

    running.shutdown().await.unwrap();
}
