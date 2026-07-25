//! The real-model oracle benchmark witness (Tier-B, LOCAL): drive the `bench-v1` task
//! slice on a LIVE served model and score every run with the golden oracle scorers via
//! `kx_gateway::eval_bench` — the proof that the runtime's agentic quality is MEASURED on
//! real model output, not replayed from scripted fixtures.
//!
//! This is the hard-but-LOCAL gate. It is `#[ignore]` + `--features inference` and NEVER
//! runs in `just ci` (the deterministic golden gate is `just eval`, flake-proof over
//! fixtures). The oracle FLOORS are asserted only when a CAPABLE model is served
//! (Gemma-4-12B — Ollama `gemma3:12b` or a Gemma GGUF); on the weak Qwen3 CI stand-in the
//! run is RECORD-ONLY (numbers persisted, no assert) — a weak model must never gate the
//! real oracle.
//!
//! Drive BOTH engines (restart-per-run):
//!   `KX_SERVE_OLLAMA=on KX_SERVE_OLLAMA_MODELS=gemma3:12b just eval-bench`   # Ollama
//!   `just fetch-gemma-model && KX_SERVE_MODEL_GGUF=<gemma-4-12b.gguf> just eval-bench`  # llama.cpp
//! Capture/refresh the committed per-engine baseline with `KX_BENCH_UPDATE_BASELINE=1`.

#![cfg(feature = "inference")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use kx_gateway::eval_bench::score_live_suite;
use kx_gateway::{start, REACT_AUTO_RECIPE_HANDLE, REACT_RECIPE_HANDLE};
use kx_proto::proto;
use kx_proto::proto::kx_gateway_client::KxGatewayClient;
use tonic::transport::Channel;

/// The per-task settle budget (a 12B model on CPU can take minutes per task).
const SETTLE_TIMEOUT: Duration = Duration::from_secs(240);
/// The aggregate `task_success` floor a CAPABLE model must clear (per-mille). Store-only
/// kv facts + a deterministic echo make the oracle a genuine tool-use proof; a capable
/// Gemma clears these comfortably (observed 1000/1000 on gemma3:12b). Set below the
/// observed value so single-task real-model nondeterminism never false-fails the floor.
const CAPABLE_TASK_SUCCESS_FLOOR: u32 = 600;
/// The baseline ratchet tolerance (per-mille). Real-model runs are nondeterministic — one
/// of five tasks flipping a binary metric swings the aggregate ~200 per-mille — so the
/// fail-closed regression gate absorbs that much noise while still catching a real
/// capability collapse (a gate falling far below the committed baseline).
const BASELINE_TOLERANCE: u32 = 200;

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

/// The (engine, model) label pair from the operator's env — the record's identity.
fn engine_and_model() -> (String, String) {
    if ollama_opted_in() {
        let model = std::env::var("KX_SERVE_OLLAMA_MODELS")
            .unwrap_or_default()
            .split(',')
            .next()
            .unwrap_or("")
            .trim()
            .to_string();
        ("ollama".to_string(), model)
    } else {
        let model = serve_gguf()
            .and_then(|p| p.file_name().and_then(|f| f.to_str()).map(str::to_string))
            .unwrap_or_default();
        ("llamacpp".to_string(), model)
    }
}

/// `git rev-parse HEAD`, or `"unknown"` — the record's commit label (every recorded
/// number carries a commit + environment label).
fn git_sha() -> String {
    std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

fn env_label(engine: &str, model: &str) -> String {
    let cores = std::thread::available_parallelism()
        .map(std::num::NonZeroUsize::get)
        .unwrap_or(1);
    format!(
        "{}/{} ({cores} cores) | {engine} | {model}",
        std::env::consts::OS,
        std::env::consts::ARCH
    )
}

/// The committed per-engine Gemma baseline (the fail-closed ratchet), in the kx-eval corpus.
fn baseline_path(engine: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../kx-eval/corpus/bench-v1")
        .join(format!("baseline.{engine}.json"))
}

/// The gitignored env-labelled trend sink at the repo root.
fn benchmarks_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/benchmarks")
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

/// Read the served model id from any journaled react turn — the post-run identity check
/// (guards against a stale/wrong serve reporting the expected label).
async fn served_model_id(c: &mut KxGatewayClient<Channel>) -> Option<String> {
    let turns = c
        .list_react_turns(proto::ListReactTurnsRequest {
            limit: Some(50),
            instance_id: None,
            step_salt: None,
        })
        .await
        .ok()?
        .into_inner();
    turns
        .turns
        .into_iter()
        .map(|t| t.model_id)
        .find(|m| !m.is_empty())
}

/// One server-embed document (empty embedding ⇒ the host embeds `content`).
fn doc(content: &[u8]) -> proto::IngestDocument {
    proto::IngestDocument {
        content: content.to_vec(),
        embedding: Vec::new(),
        ..Default::default()
    }
}

/// The grounding corpus the `reach` retrieval task searches. The load-bearing fact
/// (`ZEPHYR-77`) exists ONLY here — no model knows it — so an answer carrying it is proof
/// the run reached the dataset, and the distractors make the retrieval non-trivial.
const REACH_CORPUS: [&[u8]; 3] = [
    b"The Helios ground station uses the callsign ZEPHYR-77 for all eclipse-window transmissions.",
    b"Tectonic plates drift over the mantle, causing earthquakes at their boundaries.",
    b"The mitochondria is the powerhouse of the cell, producing ATP from glucose.",
];

/// The durable fact the `reach` memory task must recall. Written by the operator before
/// the run (all-zero instance) so the recall crosses a run boundary, which is the point.
const REACH_MEMORY: &[u8] = b"The on-call engineer for the Helios ground station is Marisol Vance.";

/// The App the `reach-inherit-principal` task runs. It declares NO tools — its steering
/// sets `reach: inherit_principal`, which REPLACES the (empty) declared wish with the
/// caller's whole resolvable ceiling. A tool firing under this App therefore fired
/// because reach widened the entry step's contract, not because the App asked for it.
///
/// The prompt NAMES the tool so that inheritance is the only way it can fire — tool
/// SELECTION under a wide menu is what the `tool` family measures, not this one. `guards`
/// pins the same budget every other family runs under, so families are compared on equal
/// terms rather than one silently inheriting a different default.
///
/// ⚠ **KNOWN-FAILING at time of capture, on BOTH engines.** `GetAppManifest` reports
/// `reach_inherit` with 7 inherited tools, but the run dead-letters on turn 0 — "the chain
/// could not progress (a tool dispatch failed or no further turn was admissible)" — with
/// no tool row ever committed. Naming the tool and pinning the budget did NOT change it,
/// so this is not tool selection and not the budget. The static checks all pass and the
/// fire path does not: a capability can be inherited, admitted to the warrant, and
/// reported by the manifest, yet still not be dispatchable on the App run path. The
/// committed baseline records that MEASURED state rather than the intended one; the
/// trajectory witness prints the reason on every run, and the ratchet will notice when it
/// starts passing. Tracked as a follow-up — the benchmark's job here is to make the gap
/// visible, not to hide it behind a task that avoids the path.
fn reach_app_envelope() -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "schema": "kortecx.app/v1",
        "version": "1",
        "name": "kx-bench-reach",
        "blueprint": { "steps": [ { "kind": "model", "prompt":
            "Use your mcp-kv/get tool to look up the value stored under the key 'b', \
             then report that value exactly." } ] },
        "steering_config": {
            "tools": { "reach": "inherit_principal" },
            "guards": { "max_turns": 8, "max_tool_calls": 6 }
        }
    }))
    .expect("the reach App envelope encodes")
}

/// Provision every `reach` fixture and HARD-assert each landed.
///
/// A fixture that silently failed to land is the worst failure mode this benchmark has:
/// the task still runs, the model still answers, and the oracle scores a 0 that reads as
/// a model incapability instead of an empty dataset. So each write is read back before a
/// single task is scored. Returns false when the serve cannot host the fixtures at all
/// (no embedder / memory disabled) — the caller then lets the family SKIP rather than
/// scoring it.
async fn provision_reach_fixtures(c: &mut KxGatewayClient<Channel>) -> bool {
    // (1) The retrieval corpus.
    let ingest = c
        .ingest_documents(proto::IngestDocumentsRequest {
            dataset: kx_gateway::eval_bench::BENCH_DATASET.to_string(),
            documents: REACH_CORPUS.iter().map(|d| doc(d)).collect(),
        })
        .await;
    if ingest.is_err() {
        eprintln!("eval-bench: reach fixtures unavailable — ingest failed (no embedder wired)");
        return false;
    }
    let hits = c
        .query_dataset(proto::QueryDatasetRequest {
            dataset: kx_gateway::eval_bench::BENCH_DATASET.to_string(),
            query_text: "Helios ground station callsign".to_string(),
            k: 3,
            ..Default::default()
        })
        .await
        .expect("query the freshly ingested bench dataset")
        .into_inner();
    assert!(
        !hits.hits.is_empty(),
        "the reach dataset must be searchable BEFORE scoring — an empty dataset scores 0 \
         and reads as a model failure"
    );

    // (2) The durable memory fact.
    if c.store_memory(proto::StoreMemoryRequest {
        content: REACH_MEMORY.to_vec(),
        embedding: Vec::new(),
        kind: proto::MemoryKind::Semantic as i32,
        namespace: String::new(),
    })
    .await
    .is_err()
    {
        eprintln!("eval-bench: reach fixtures unavailable — StoreMemory failed (KX_SERVE_MEMORY?)");
        return false;
    }
    let mems = c
        .list_memories(proto::ListMemoriesRequest {
            limit: Some(10),
            instance_id: None,
            namespace: String::new(),
            include_tombstoned: false,
        })
        .await
        .expect("list the freshly stored memory")
        .into_inner();
    assert!(
        mems.memories
            .iter()
            .any(|m| String::from_utf8_lossy(&m.content).contains("Marisol Vance")),
        "the recall fact must be readable BEFORE scoring — an empty memory scores 0 and \
         reads as a model failure"
    );

    // (3) The inherit-principal App.
    c.save_app(proto::SaveAppRequest {
        handle: kx_gateway::eval_bench::BENCH_REACH_APP_HANDLE.to_string(),
        envelope_json: reach_app_envelope(),
        source_digest: Vec::new(),
    })
    .await
    .expect("save the reach App");
    let manifest = c
        .get_app_manifest(proto::GetAppManifestRequest {
            handle: kx_gateway::eval_bench::BENCH_REACH_APP_HANDLE.to_string(),
        })
        .await
        .expect("read the reach App manifest")
        .into_inner();
    // The property the task exists to prove, asserted at the source: the App inherits.
    assert!(
        manifest.reach_inherit,
        "the reach App must report reach_inherit — without it the task would prove nothing"
    );
    assert!(
        manifest.tools.iter().any(|t| t.inherited),
        "every tool the reach App can reach must be INHERITED (it declared none itself)"
    );
    eprintln!(
        "eval-bench: reach fixtures ready — {} dataset hit(s), memory stored, App inherits {} tool(s)",
        hits.hits.len(),
        manifest.tools.iter().filter(|t| t.inherited).count()
    );
    true
}

/// The distinct `(tool_id, tool_version)` pairs the served runs actually committed — the
/// measure-first instrument for pinning `expected_tools` to the real grant-id form (which
/// is grant-shape-dependent, so it must be observed, not guessed).
async fn observed_tool_ids(c: &mut KxGatewayClient<Channel>) -> Vec<(String, String)> {
    let Ok(resp) = c
        .list_react_turns(proto::ListReactTurnsRequest {
            limit: Some(200),
            instance_id: None,
            step_salt: None,
        })
        .await
    else {
        return Vec::new();
    };
    let mut seen: Vec<(String, String)> = Vec::new();
    for t in resp.into_inner().turns {
        if t.branch == "tool" {
            let pair = (t.tool_id, t.tool_version);
            if !seen.contains(&pair) {
                seen.push(pair);
            }
        }
    }
    seen
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "real LLM inference; needs a Gemma GGUF (just fetch-gemma-model) or Ollama (KX_SERVE_OLLAMA=on KX_SERVE_OLLAMA_MODELS=gemma3:12b); opt in with --ignored"]
async fn bench_v1_oracle_scored_over_a_live_react_chain() {
    // Resolve the model: a GGUF (set the env so serve loads it) or the Ollama opt-in.
    if let Some(gguf) = serve_gguf() {
        std::env::set_var("KX_SERVE_MODEL_GGUF", &gguf);
    } else if !ollama_opted_in() {
        eprintln!(
            "skipping: no model — `just fetch-gemma-model` (GGUF) or \
             `KX_SERVE_OLLAMA=on KX_SERVE_OLLAMA_MODELS=gemma3:12b`"
        );
        return;
    }
    std::env::set_var("KX_SERVE_AUTOGRANT", "1");

    let (engine, model) = engine_and_model();
    let capable = model.to_ascii_lowercase().contains("gemma");
    let env_label = env_label(&engine, &model);
    let git_sha = git_sha();

    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    // react-auto needs the model + the bundled echo/kv/calc capabilities. If react itself
    // didn't provision, the bundled bins are absent — skip (matches eval_real_model).
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
            "skipping: {REACT_RECIPE_HANDLE} not provisioned — build the bundled tools \
             (`cargo build -p kx-mcp`)"
        );
        running.shutdown().await.unwrap();
        std::env::remove_var("KX_SERVE_AUTOGRANT");
        return;
    }
    assert!(
        recipes
            .recipes
            .iter()
            .any(|r| r.handle == REACT_AUTO_RECIPE_HANDLE),
        "react-auto is provisioned alongside react"
    );

    // The `reach` family reads what the operator put there BEFORE the run — a dataset, a
    // durable memory, and an App that inherits the principal's ceiling. Provision them
    // first and read each back; a fixture that never landed would score 0 and read as a
    // model failure.
    let reach_ready = provision_reach_fixtures(&mut c).await;

    let corpus = kx_eval::load_bench_v1().expect("bench-v1 corpus loads");
    eprintln!(
        "eval-bench: scoring {} live task(s) on [{env_label}] (capable={capable}, \
         reach_fixtures={reach_ready})",
        corpus.suite.tasks.len()
    );

    // THE WITNESS: drive every bench task on the served model + score its REAL output.
    let outcome = score_live_suite(&mut c, &corpus, env_label.clone(), git_sha, SETTLE_TIMEOUT)
        .await
        .expect("score bench-v1 over live runs");
    let report = outcome.report.clone();
    for s in &outcome.skipped {
        eprintln!(
            "eval-bench: SKIPPED family {:?} — {} not provisioned ({} task(s): {})",
            s.family,
            s.missing_recipe,
            s.task_ids.len(),
            s.task_ids.join(", ")
        );
    }
    // A partial run is a partial measurement — say so loudly rather than let the number
    // read as full coverage.
    let complete = outcome.is_complete() && reach_ready;
    if !complete {
        eprintln!("eval-bench: ⚠ INCOMPLETE COVERAGE — this run does NOT cover the whole corpus");
    }

    // Post-run identity check: the served model must match the label we recorded (a
    // capable run that silently fell back to a weak model would be a false record).
    let served = served_model_id(&mut c).await.unwrap_or_default();
    eprintln!("eval-bench: served model_id = {served:?}");
    if capable {
        assert!(
            served.to_ascii_lowercase().contains("gemma"),
            "identity check: asked for a Gemma model but the serve reports {served:?}"
        );
    }

    // The gates + the per-task oracle detail.
    eprintln!(
        "eval-bench report — suite '{}' (digest {}…)",
        report.suite_id,
        &report.suite_digest[..16]
    );
    for g in &report.gates {
        eprintln!("  {:<22} {:>4} / 1000", g.id, g.per_mille);
    }
    for t in &report.per_task {
        for s in &t.scores {
            if s.applicable {
                eprintln!("    {:<16} {:<22} {}", t.task_id, s.metric_id, s.detail);
            }
        }
    }
    // The TRAJECTORY witness for anything that did not answer. A per-task gate of 0 is a
    // verdict without evidence; this prints what the run actually did — which tool it
    // proposed, what the runtime refused and why, where it stopped — so a failure is
    // diagnosable from the run that produced it instead of re-run to be understood.
    for t in &outcome.transcripts {
        let terminal = t.terminal_branch();
        if terminal == kx_eval::Branch::Answer {
            continue;
        }
        eprintln!(
            "eval-bench: TRAJECTORY {} (terminal {terminal:?})",
            t.task_id
        );
        for turn in &t.turns {
            let tool = if turn.tool_id.is_empty() {
                String::new()
            } else {
                format!(" tool={}@{}", turn.tool_id, turn.tool_version)
            };
            let why = if turn.rejection_reason.is_empty() {
                String::new()
            } else {
                format!(" reason={:?}", turn.rejection_reason)
            };
            eprintln!("    turn {} {:?}{tool}{why}", turn.turn, turn.branch);
        }
        eprintln!(
            "    final_answer = {:?}",
            t.final_answer.as_deref().unwrap_or("<none>")
        );
    }

    // Measure-first: the raw committed tool-id form, to pin `expected_tools` against.
    let observed = observed_tool_ids(&mut c).await;
    eprintln!("eval-bench: observed committed tool ids = {observed:?}");

    // THE TOOL-CONTRACT ASSERTION, observed rather than inferred. `tool-contract-refusal`
    // instructs the model to use a tool no chain was ever granted. The oracle scores what
    // the model DID; this asserts the runtime invariant underneath it — the ungranted name
    // never entered any chain's admitted grant set, and never fired. Naming is not
    // granting, and here that is a measurement, not a claim.
    const UNGRANTED: &str = "admin-db/drop_table";
    assert!(
        !observed.iter().any(|(id, _)| id == UNGRANTED),
        "an UNGRANTED tool fired: {UNGRANTED} appears in the committed tool ids {observed:?}"
    );
    let admitted = c
        .list_react_turns(proto::ListReactTurnsRequest {
            limit: Some(200),
            instance_id: None,
            step_salt: None,
        })
        .await
        .map(|r| {
            r.into_inner()
                .turns
                .into_iter()
                .flat_map(|t| t.granted_tools)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    assert!(
        !admitted.iter().any(|g| g.contains(UNGRANTED)),
        "the ungranted {UNGRANTED} must never appear in a chain's admitted grants"
    );
    eprintln!(
        "eval-bench: tool contract holds — {UNGRANTED} absent from {} admitted grant entries",
        admitted.len()
    );

    // Persist the env-labelled trend record (the gitignored docs/benchmarks sink).
    std::fs::create_dir_all(benchmarks_dir()).unwrap();
    let trend = benchmarks_dir().join(format!("bench-v1.{engine}.json"));
    std::fs::write(&trend, report.to_json().unwrap()).unwrap();
    eprintln!("eval-bench: trend record → {}", trend.display());

    // Capture mode: write the committed per-engine baseline and stop (a deliberate
    // re-baseline, mirroring `kx-eval run --update-baseline`).
    if std::env::var("KX_BENCH_UPDATE_BASELINE").is_ok() {
        // The baseline is keyed by `suite_digest` — the WHOLE corpus. Capturing one that
        // silently omitted a family would ratchet the full corpus against a subset and
        // read as full coverage forever after. Refuse, loudly.
        assert!(
            complete,
            "refusing to capture a baseline from an INCOMPLETE run — the committed \
             baseline is keyed by the whole corpus digest, so a partial capture would \
             ratchet every later run against a subset. Provision the missing families \
             (hnsw for react-rag, KX_SERVE_MEMORY for react-memory) and re-run."
        );
        let path = baseline_path(&engine);
        let json = serde_json::to_string_pretty(&report.to_baseline()).unwrap();
        std::fs::write(&path, format!("{json}\n")).unwrap();
        eprintln!("eval-bench: baseline captured → {}", path.display());
        running.shutdown().await.unwrap();
        std::env::remove_var("KX_SERVE_AUTOGRANT");
        return;
    }

    // Ratchet: if a committed baseline exists, it is the fail-closed regression gate
    // (fails on corpus drift or any gate below baseline). Assert it only for a capable
    // model — the weak stand-in never gates a real number.
    let bpath = baseline_path(&engine);
    let task_success = report
        .gates
        .iter()
        .find(|g| g.id == "task_success")
        .map_or(0, |g| g.per_mille);
    if bpath.is_file() {
        let baseline: kx_eval::Baseline =
            serde_json::from_str(&std::fs::read_to_string(&bpath).unwrap()).unwrap();
        let cmp = kx_eval::compare_to_baseline(&report, &baseline, BASELINE_TOLERANCE)
            .unwrap_or_else(|e| {
                // The corpus changed under the committed baseline. This is the ratchet
                // working, not a failure: the measurement contract moved, so every number
                // captured against the old corpus is void until an operator deliberately
                // re-captures on BOTH engines. Say exactly that instead of a raw error.
                panic!(
                    "{e}\n\n  The bench corpus changed since this baseline was captured.\n  \
                     Re-capture BOTH engines deliberately:\n    \
                     KX_SERVE_OLLAMA=on KX_SERVE_OLLAMA_MODELS=gemma3:12b \
                     KX_BENCH_UPDATE_BASELINE=1 just eval-bench\n    \
                     ollama stop gemma3:12b   # GPU residency is a cross-engine singleton\n    \
                     KX_SERVE_MODEL_GGUF=<gemma-12b.gguf> KX_BENCH_UPDATE_BASELINE=1 just eval-bench"
                )
            });
        if cmp.ok {
            eprintln!(
                "eval-bench: PASS — all gates >= baseline {}",
                bpath.display()
            );
        } else {
            eprintln!(
                "eval-bench: {} regression(s) vs {}",
                cmp.regressions.len(),
                bpath.display()
            );
            for r in &cmp.regressions {
                eprintln!(
                    "  - {}: {} < baseline {}",
                    r.metric_id, r.current_per_mille, r.baseline_per_mille
                );
            }
            // Gate a capable model against the ratchet — but ONLY on a complete run. On
            // an incomplete one the missing family's tasks were never scored (or scored
            // against fixtures that never landed), so a "regression" would be blaming the
            // model for the serve's build. Loud, not fatal.
            if capable && complete {
                panic!("eval-bench regressed below the committed baseline");
            }
            if capable {
                eprintln!(
                    "eval-bench: regression NOT gated — coverage was incomplete, so these \
                     numbers indict the serve's provisioning, not the model"
                );
            }
        }
    } else {
        eprintln!(
            "eval-bench: no committed baseline at {} — record-only (capture with \
             KX_BENCH_UPDATE_BASELINE=1)",
            bpath.display()
        );
    }

    if capable && complete {
        assert!(
            task_success >= CAPABLE_TASK_SUCCESS_FLOOR,
            "capable-model task_success {task_success} < floor {CAPABLE_TASK_SUCCESS_FLOOR}"
        );
    } else if capable {
        eprintln!("eval-bench: RECORD-ONLY (incomplete coverage — the oracle floor is not gated)");
    } else {
        eprintln!(
            "eval-bench: RECORD-ONLY (weak stand-in {model:?} — the oracle floor is not gated)"
        );
    }

    running.shutdown().await.unwrap();
    std::env::remove_var("KX_SERVE_AUTOGRANT");
}
