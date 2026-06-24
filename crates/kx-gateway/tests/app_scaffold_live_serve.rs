//! POC-5a LIVE witness (`--ignored`): drive the FULL server-side App scaffold on a
//! served model — save a minimal App, `ScaffoldApp`, poll `GetScaffoldStatus` to
//! terminal, and assert the FIXED skeleton landed in the app's content-addressed
//! branch with non-empty model-authored bodies (the host is never written), then prove the
//! POC-5b lock: a locked App refuses a further agentic in-CAS edit.
//!
//! Gated `#[cfg(feature = "inference")]` AND `#[ignore]`; runtime-skips without a
//! GGUF. **Drive on Gemma-4 locally** (the deep-test model, GR15 — never Qwen3 for a
//! real scaffold; a small model authors degenerate file bodies):
//! `KX_SERVE_MODEL_GGUF=target/models/gemma-4-12b-it-q4_k_m.gguf \`
//! `  cargo test -p kx-gateway --features inference,hnsw --test app_scaffold_live_serve -- --ignored --nocapture`

#![cfg(feature = "inference")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use kx_gateway::start;
use kx_proto::proto;
use kx_proto::proto::kx_gateway_client::KxGatewayClient;
use tonic::transport::Channel;

const SKELETON: &[&str] = &[
    "README.md",
    "app.json",
    "prompts/system.md",
    "rules/guardrails.md",
    "skills/main.md",
];

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

/// A minimal `kortecx.app/v1` envelope — a single greedy model step. The scaffold
/// writes the project files INTO the branch; the blueprint is what `kx app run`
/// executes (secondary to the scaffolded tree).
fn minimal_app_envelope(name: &str, goal: &str) -> Vec<u8> {
    let blueprint = serde_json::json!({
        "seed": 0,
        "steps": [{ "kind": "model", "prompt": goal }]
    });
    let mut env = kx_app::AppEnvelope::new(name, blueprint);
    env.description = goal.to_string();
    env.to_canonical_json().unwrap()
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "real in-process LLM inference; needs a GGUF (Gemma-4 locally); opt in with --ignored"]
async fn scaffold_writes_the_skeleton_then_lock_refuses_edit() {
    let Some(gguf) = serve_model() else {
        eprintln!(
            "skipping: no serve model — set KX_SERVE_MODEL_GGUF (a real GGUF, Gemma-4 locally)"
        );
        return;
    };
    std::env::set_var("KX_SERVE_MODEL_GGUF", &gguf);

    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    // ---- save a minimal App, then scaffold its project tree ----
    let handle = "apps/local/pdf-summarizer".to_string();
    c.save_app(proto::SaveAppRequest {
        handle: handle.clone(),
        envelope_json: minimal_app_envelope("PDF Summarizer", "Summarize uploaded PDF documents"),
    })
    .await
    .expect("SaveApp")
    .into_inner();

    let launched = c
        .scaffold_app(proto::ScaffoldAppRequest {
            handle: handle.clone(),
            branch_handle: String::new(),
            instruction: "Summarize uploaded PDF documents into a short brief".to_string(),
        })
        .await
        .expect("ScaffoldApp (a served model is present)")
        .into_inner();
    let branch = launched.branch_handle.clone();
    assert_eq!(
        branch, handle,
        "one-App-one-branch: the branch is the App handle"
    );

    // ---- poll to terminal (5 greedy 12B write steps — generous bound) ----
    let deadline = Instant::now() + Duration::from_secs(20 * 60);
    let done_phase = proto::get_scaffold_status_response::Phase::Done as i32;
    let failed_phase = proto::get_scaffold_status_response::Phase::Failed as i32;
    let mut last_done = 0usize;
    loop {
        let status = c
            .get_scaffold_status(proto::GetScaffoldStatusRequest {
                branch_handle: branch.clone(),
            })
            .await
            .unwrap()
            .into_inner();
        if status.files_done.len() != last_done {
            last_done = status.files_done.len();
            eprintln!(
                "scaffold: {}/{} files written (phase={})",
                last_done,
                SKELETON.len(),
                status.phase
            );
        }
        if status.phase == done_phase {
            break;
        }
        assert_ne!(
            status.phase, failed_phase,
            "scaffold failed on the live model: {}",
            status.detail
        );
        assert!(
            Instant::now() < deadline,
            "scaffold did not finish within the 20-minute bound"
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    // ---- assert the FIXED skeleton landed with non-empty model-authored bodies ----
    let manifest = c
        .get_branch(proto::GetBranchRequest {
            handle: branch.clone(),
        })
        .await
        .unwrap()
        .into_inner()
        .branch
        .expect("the scaffolded branch resolves");
    let paths: Vec<&str> = manifest.items.iter().map(|i| i.path.as_str()).collect();
    for f in SKELETON {
        assert!(
            paths.contains(f),
            "skeleton file {f} is in the branch manifest"
        );
        let body = c
            .get_branch_content(proto::GetBranchContentRequest {
                handle: branch.clone(),
                path: (*f).to_string(),
            })
            .await
            .unwrap()
            .into_inner();
        assert!(body.found, "{f} body resolves");
        assert!(
            !body.payload.iter().all(u8::is_ascii_whitespace),
            "{f} has a non-empty model-authored body (GR15 fail-closed never advances empties)"
        );
        eprintln!("  ✓ {f} ({} bytes)", body.payload.len());
    }

    // ---- POC-5b: lock the App, then a further agentic edit is REFUSED ----
    let locked = c
        .lock_app(proto::LockAppRequest {
            branch_handle: branch.clone(),
        })
        .await
        .unwrap()
        .into_inner();
    assert!(locked.locked);
    // A direct AdvanceBranch (re-point README to its own current ref) is the
    // agent-write chokepoint — locked ⇒ refused with the structured code.
    let readme_ref = manifest
        .items
        .iter()
        .find(|i| i.path == "README.md")
        .map(|i| i.content_ref.clone())
        .unwrap();
    let err = c
        .advance_branch(proto::AdvanceBranchRequest {
            handle: branch.clone(),
            path: "README.md".into(),
            content_ref: readme_ref,
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), tonic::Code::FailedPrecondition);
    assert_eq!(
        err.metadata()
            .get(kx_gateway_core::REFUSAL_CODE_METADATA_KEY)
            .and_then(|v| v.to_str().ok()),
        Some("LOCKED_BRANCH"),
        "a locked App refuses the agentic edit at the AdvanceBranch chokepoint",
    );
    eprintln!("LIVE scaffold+lock witness PASSED");

    running.shutdown().await.unwrap();
}
