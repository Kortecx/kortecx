//! POC-5d LIVE witness (`--ignored`): drive the single-App IDE write paths against a
//! live model end-to-end — (1) a LINEAGE structure edit (`GetApp` → mutate the
//! blueprint → `SaveApp` → `GetApp` reflects it), (2) a DIRECT in-CAS file edit
//! (`PutContent` → `AdvanceBranch` → `GetBranchContent` shows the new body), (3) the
//! per-App LOCK freezing BOTH a file edit AND a structure save (`LOCKED_BRANCH`), then
//! (4) RUN the edited agentic App's blueprint on the served model and assert it settles
//! (whether a `tool` round fires is model-nondeterministic, so it is LOGGED — the
//! deterministic fire-commit proofs live in `kx-coordinator`/`kx-toolcall`).
//!
//! The agentic propose→diff→approve review gate is a CLIENT-SIDE decomposition of the
//! already-live `editBranch` (propose = invoke `react-edit` WITHOUT advancing; approve
//! = `AdvanceBranch`) — its server behaviour is unchanged, so it is covered by the
//! deterministic UI tests + the live console walk-through rather than re-proven here.
//!
//! Gated `#[cfg(feature = "inference")]` AND `#[ignore]`; runtime-skips without a GGUF.
//! **Drive on Gemma-4 locally** (the deep-test model, GR15):
//! `KX_SERVE_MODEL_GGUF=target/models/gemma-4-12b-it-q4_k_m.gguf \`
//! `  cargo test -p kx-gateway --features inference --test app_ide_live_serve -- --ignored --nocapture`

#![cfg(feature = "inference")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use kx_gateway::{start, REACT_RECIPE_HANDLE};
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

/// PutContent a body, returning its server-derived 32-byte ref (the advance target).
async fn put(c: &mut KxGatewayClient<Channel>, body: &[u8]) -> Vec<u8> {
    c.put_content(proto::PutContentRequest {
        payload: body.to_vec(),
        media_type: String::new(),
        filename: "README.md".into(),
    })
    .await
    .unwrap()
    .into_inner()
    .content_ref
}

/// An agentic echo App envelope (a MODEL step granting the bundled `mcp-echo/echo`).
fn echo_app_envelope(prompt: &str) -> Vec<u8> {
    let blueprint = serde_json::json!({
        "seed": 0,
        "steps": [{
            "kind": "model",
            "prompt": prompt,
            "tool_contract": { "mcp-echo/echo": "1" },
            "params": { "max_turns": "4", "max_tool_calls": "2" }
        }]
    });
    let mut env = kx_app::AppEnvelope::new("Echo IDE", blueprint);
    env.description = "an agentic App edited through the single-App IDE".to_string();
    env.to_canonical_json().unwrap()
}

/// Build the `SubmitWorkflow` request `kx app run` compiles a 1-step agentic blueprint
/// into (the `prompt` is whatever the stored/edited envelope carries).
fn run_request(prompt: &str) -> proto::SubmitWorkflowRequest {
    proto::SubmitWorkflowRequest {
        seed: 0,
        steps: vec![proto::WorkflowStep {
            kind: proto::WorkflowStepKind::Model as i32,
            model_id: String::new(),
            prompt: prompt.to_string(),
            body_signature_id: Vec::new(),
            tool_contract: [("mcp-echo/echo".to_string(), "1".to_string())]
                .into_iter()
                .collect(),
            params: [
                ("max_turns".to_string(), b"4".to_vec()),
                ("max_tool_calls".to_string(), b"2".to_vec()),
            ]
            .into_iter()
            .collect(),
        }],
        edges: vec![],
        execution_mode: proto::WorkflowExecutionMode::Frozen as i32,
        context_bundles: vec![],
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "real in-process LLM inference; needs a GGUF (Gemma-4 locally); opt in with --ignored"]
async fn app_ide_edit_lineage_files_and_run_on_a_live_model() {
    let Some(gguf) = serve_model() else {
        eprintln!("skipping: no serve model — set KX_SERVE_MODEL_GGUF (Gemma-4 locally)");
        return;
    };
    std::env::set_var("KX_SERVE_MODEL_GGUF", &gguf);
    std::env::set_var("KX_SERVE_AUTOGRANT", "1");

    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;
    let handle = "apps/local/echo-ide".to_string();

    // ---- (0) save the agentic App ----
    let v1_prompt = "Echo the word 'ping' with the echo tool, then answer with it.";
    c.save_app(proto::SaveAppRequest {
        handle: handle.clone(),
        envelope_json: echo_app_envelope(v1_prompt),
        source_digest: Vec::new(),
    })
    .await
    .expect("SaveApp v1")
    .into_inner();

    // ---- (1) LINEAGE structure edit: GetApp → mutate the blueprint → SaveApp ----
    let v2_prompt = "Echo the word 'pong' with the echo tool, then answer with it.";
    let got = c
        .get_app(proto::GetAppRequest {
            handle: handle.clone(),
        })
        .await
        .unwrap()
        .into_inner();
    assert!(got.found);
    let mut env = kx_app::AppEnvelope::from_json_slice(&got.envelope_json).unwrap();
    // mutate the (opaque) blueprint's first step prompt — what the lineage editor does.
    env.blueprint.as_mut().unwrap()["steps"][0]["prompt"] =
        serde_json::Value::String(v2_prompt.to_string());
    c.save_app(proto::SaveAppRequest {
        handle: handle.clone(),
        envelope_json: env.to_canonical_json().unwrap(),
        source_digest: Vec::new(),
    })
    .await
    .expect("SaveApp v2 (lineage edit)")
    .into_inner();
    let got2 = c
        .get_app(proto::GetAppRequest {
            handle: handle.clone(),
        })
        .await
        .unwrap()
        .into_inner();
    let env2 = kx_app::AppEnvelope::from_json_slice(&got2.envelope_json).unwrap();
    assert_eq!(
        env2.blueprint.as_ref().unwrap()["steps"][0]["prompt"].as_str(),
        Some(v2_prompt),
        "the lineage structure edit persisted"
    );

    // ---- (2) DIRECT in-CAS file edit (PutContent → AdvanceBranch → read back) ----
    c.create_branch(proto::CreateBranchRequest {
        handle: handle.clone(),
        description: String::new(),
        parent_handle: String::new(),
    })
    .await
    .unwrap();
    let r1 = put(&mut c, b"# v1\n").await;
    c.advance_branch(proto::AdvanceBranchRequest {
        handle: handle.clone(),
        path: "README.md".into(),
        content_ref: r1,
    })
    .await
    .unwrap();
    let r2 = put(&mut c, b"# v2 edited\n").await;
    c.advance_branch(proto::AdvanceBranchRequest {
        handle: handle.clone(),
        path: "README.md".into(),
        content_ref: r2,
    })
    .await
    .unwrap();
    let read = c
        .get_branch_content(proto::GetBranchContentRequest {
            handle: handle.clone(),
            path: "README.md".into(),
        })
        .await
        .unwrap()
        .into_inner();
    assert!(read.found);
    assert_eq!(
        read.payload, b"# v2 edited\n",
        "the direct file edit committed"
    );

    // ---- (3) LOCK freezes BOTH a file edit AND a structure save ----
    c.lock_app(proto::LockAppRequest {
        branch_handle: handle.clone(),
    })
    .await
    .unwrap();
    let r3 = put(&mut c, b"# blocked\n").await;
    let file_err = c
        .advance_branch(proto::AdvanceBranchRequest {
            handle: handle.clone(),
            path: "README.md".into(),
            content_ref: r3,
        })
        .await
        .unwrap_err();
    assert_eq!(file_err.code(), tonic::Code::FailedPrecondition);
    let struct_err = c
        .save_app(proto::SaveAppRequest {
            handle: handle.clone(),
            envelope_json: echo_app_envelope("blocked"),
            source_digest: Vec::new(),
        })
        .await
        .unwrap_err();
    assert_eq!(struct_err.code(), tonic::Code::FailedPrecondition);
    c.unlock_app(proto::UnlockAppRequest {
        branch_handle: handle.clone(),
    })
    .await
    .unwrap();

    // ---- (4) RUN the edited agentic App on the live model ----
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
            "skipping the run leg: kx/recipes/react not provisioned (bundled kx-mcp-echo missing)"
        );
        running.shutdown().await.unwrap();
        std::env::remove_var("KX_SERVE_AUTOGRANT");
        return;
    }
    let run = c
        .submit_workflow(run_request(v2_prompt))
        .await
        .expect("SubmitWorkflow of the edited App blueprint")
        .into_inner();
    let mut tool_fired = false;
    let mut settled = false;
    for _ in 0..900 {
        let turns = c
            .list_react_turns(proto::ListReactTurnsRequest {
                limit: None,
                instance_id: Some(run.instance_id.clone()),
                step_salt: None,
            })
            .await
            .unwrap()
            .into_inner();
        if turns.turns.iter().any(|t| t.branch == "tool") {
            tool_fired = true;
        }
        if turns
            .turns
            .iter()
            .any(|t| t.branch == "answer" || t.branch == "dead_letter")
        {
            settled = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    eprintln!("LIVE App IDE run: tool_fired={tool_fired} settled={settled}");
    assert!(
        settled,
        "the edited agentic App settled to a terminal on the live model"
    );

    running.shutdown().await.unwrap();
    std::env::remove_var("KX_SERVE_AUTOGRANT");
}
