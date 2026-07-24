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

    let corpus = kx_eval::load_bench_v1().expect("bench-v1 corpus loads");
    eprintln!(
        "eval-bench: scoring {} live task(s) on [{env_label}] (capable={capable})",
        corpus.suite.tasks.len()
    );

    // THE WITNESS: drive every bench task on the served model + score its REAL output.
    let report = score_live_suite(&mut c, &corpus, env_label.clone(), git_sha, SETTLE_TIMEOUT)
        .await
        .expect("score bench-v1 over live runs");

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
    // Measure-first: the raw committed tool-id form, to pin `expected_tools` against.
    eprintln!(
        "eval-bench: observed committed tool ids = {:?}",
        observed_tool_ids(&mut c).await
    );

    // Persist the env-labelled trend record (the gitignored docs/benchmarks sink).
    std::fs::create_dir_all(benchmarks_dir()).unwrap();
    let trend = benchmarks_dir().join(format!("bench-v1.{engine}.json"));
    std::fs::write(&trend, report.to_json().unwrap()).unwrap();
    eprintln!("eval-bench: trend record → {}", trend.display());

    // Capture mode: write the committed per-engine baseline and stop (a deliberate
    // re-baseline, mirroring `kx-eval run --update-baseline`).
    if std::env::var("KX_BENCH_UPDATE_BASELINE").is_ok() {
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
            .expect("no corpus drift");
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
            if capable {
                panic!("eval-bench regressed below the committed baseline");
            }
        }
    } else {
        eprintln!(
            "eval-bench: no committed baseline at {} — record-only (capture with \
             KX_BENCH_UPDATE_BASELINE=1)",
            bpath.display()
        );
    }

    if capable {
        assert!(
            task_success >= CAPABLE_TASK_SUCCESS_FLOOR,
            "capable-model task_success {task_success} < floor {CAPABLE_TASK_SUCCESS_FLOOR}"
        );
    } else {
        eprintln!(
            "eval-bench: RECORD-ONLY (weak stand-in {model:?} — the oracle floor is not gated)"
        );
    }

    running.shutdown().await.unwrap();
    std::env::remove_var("KX_SERVE_AUTOGRANT");
}
