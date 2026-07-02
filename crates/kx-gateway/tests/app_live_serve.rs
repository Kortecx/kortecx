//! POC-4 LIVE witness (`--ignored`): author an App whose blueprint is an agentic
//! tool-calling step, persist it through the real `SaveApp`/`ListApps`/`GetApp`
//! catalog, then RUN its blueprint on a served model and assert the chain settles
//! to a terminal (the non-flaky invariant; whether a `tool` round actually fires is
//! model-nondeterministic, so it is LOGGED, not asserted — the CI-deterministic
//! fire-commit proofs live in `kx-coordinator`/`kx-toolcall`).
//!
//! This exercises the FULL App path end-to-end against a live model: the envelope
//! canonicalizes + round-trips through the off-journal catalog (server-derived
//! `app_ref`, SN-8), and the stored blueprint — submitted exactly as `kx app run`
//! compiles it (`SubmitWorkflow` of the agentic step) — drives the live loop.
//!
//! Gated `#[cfg(feature = "inference")]` AND `#[ignore]`; runtime-skips without a
//! GGUF. **Drive on Gemma-4 locally** (the deep-test model, GR15):
//! `KX_SERVE_MODEL_GGUF=target/models/gemma-4-12b-it-q4_k_m.gguf \`
//! `  cargo test -p kx-gateway --features inference --test app_live_serve -- --ignored --nocapture`

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

/// The `kortecx.app/v1` envelope for an agentic echo App — one MODEL step granting
/// the bundled `mcp-echo/echo` tool. The blueprint is exactly what
/// `Chain.to_blueprint()` / `kx chain --emit-blueprint` produce.
fn echo_app_envelope() -> Vec<u8> {
    let blueprint = serde_json::json!({
        "seed": 0,
        "steps": [{
            "kind": "model",
            "prompt": "Use the echo tool to echo the word 'pong', then answer with it.",
            "tool_contract": { "mcp-echo/echo": "1" },
            "params": { "max_turns": "4", "max_tool_calls": "2" }
        }]
    });
    let mut env = kx_app::AppEnvelope::new("Echo Agent", blueprint);
    env.description = "an agentic App that fires the bundled echo tool".to_string();
    env.tags = vec!["agentic".to_string(), "demo".to_string()];
    env.to_canonical_json().unwrap()
}

/// The `SubmitWorkflow` request `kx app run` compiles the stored blueprint into: an
/// agentic MODEL step (a non-empty `tool_contract` ⇒ the bounded reason→tool loop),
/// with the budget folded into `params` (the canonical react keys).
fn echo_app_run_request() -> proto::SubmitWorkflowRequest {
    proto::SubmitWorkflowRequest {
        seed: 0,
        steps: vec![proto::WorkflowStep {
            kind: proto::WorkflowStepKind::Model as i32,
            model_id: String::new(),
            prompt: "Use the echo tool to echo the word 'pong', then answer with it.".to_string(),
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
async fn app_catalog_round_trips_and_runs_on_a_live_model() {
    let Some(gguf) = serve_model() else {
        eprintln!(
            "skipping: no serve model — set KX_SERVE_MODEL_GGUF (a real GGUF, Gemma-4 locally)"
        );
        return;
    };
    std::env::set_var("KX_SERVE_MODEL_GGUF", &gguf);
    // The agentic step's echo grant resolves from the live registry (autogrant).
    std::env::set_var("KX_SERVE_AUTOGRANT", "1");

    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    // ---- the App catalog round-trip (save → list → get) ----
    let envelope = echo_app_envelope();
    let saved = c
        .save_app(proto::SaveAppRequest {
            handle: "apps/local/echo-agent".to_string(),
            envelope_json: envelope.clone(),
        })
        .await
        .expect("SaveApp")
        .into_inner();
    assert_eq!(
        saved.app_ref.len(),
        16,
        "app_ref is the 16B server-derived id"
    );

    let listed = c
        .list_apps(proto::ListAppsRequest {
            limit: 0,
            after_handle: String::new(),
        })
        .await
        .unwrap()
        .into_inner();
    assert!(
        listed
            .apps
            .iter()
            .any(|a| a.handle == "apps/local/echo-agent" && a.step_count == 1),
        "the saved App appears in the catalog"
    );

    let got = c
        .get_app(proto::GetAppRequest {
            handle: "apps/local/echo-agent".to_string(),
        })
        .await
        .unwrap()
        .into_inner();
    assert!(got.found);
    assert_eq!(
        got.envelope_json, envelope,
        "the envelope round-trips byte-identically"
    );

    // If the bundled echo tool / react recipe is absent, the agentic grant cannot
    // resolve — skip the run leg (the catalog round-trip above still proves POC-4).
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

    // ---- run the App's blueprint on the live model (`kx app run` path) ----
    let handle = c
        .submit_workflow(echo_app_run_request())
        .await
        .expect("SubmitWorkflow of the App blueprint")
        .into_inner();
    assert_eq!(handle.instance_id.len(), 16);

    // The non-flaky invariant: the chain reaches a terminal (answer or honest
    // dead-letter) — never hangs. Whether a `tool` round fired is LOGGED.
    let mut tool_fired = false;
    let mut settled = false;
    for _ in 0..900 {
        let turns = c
            .list_react_turns(proto::ListReactTurnsRequest {
                limit: None,
                instance_id: Some(handle.instance_id.clone()),
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
    eprintln!("LIVE App run: tool_fired={tool_fired} settled={settled}");
    assert!(
        settled,
        "the App's agentic blueprint settled to a terminal on the live model"
    );

    running.shutdown().await.unwrap();
    std::env::remove_var("KX_SERVE_AUTOGRANT");
}

/// Resolve the bundled `kx-connector-gmail` sidecar binary (release preferred), if built.
fn gmail_connector_bin() -> Option<PathBuf> {
    for profile in ["release", "debug"] {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../../target/{profile}/kx-connector-gmail"));
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

/// The `kortecx.app/v1` envelope for a Gmail-integration App: an agentic MODEL step
/// granting the registered `gmail/search` tool, a by-reference connection pointer, and
/// (when `scope_secret`) the connection's credential in `guards.secret_scope`. G2's
/// load-bearing bit: with the secret in scope the run warrant permits dialing the
/// credentialed connector; without it the dial fails closed at the broker.
fn gmail_app_envelope(connector_path: &str, scope_secret: bool) -> Vec<u8> {
    let blueprint = serde_json::json!({
        "seed": 0,
        "steps": [{
            "kind": "model",
            "prompt": "Search my Gmail for unread messages using the gmail/search tool, \
                       then briefly answer with what you found.",
            "tool_contract": { "gmail/search": "1" },
            // A tight budget so the loop converges to an answer quickly (one search →
            // answer): max_tool_calls=1 forces an answer turn after the single dial.
            "params": { "max_turns": "3", "max_tool_calls": "1" }
        }]
    });
    let mut env = kx_app::AppEnvelope::new("Gmail Agent", blueprint);
    env.description = "an agentic App that dials the bundled Gmail connector".to_string();
    env.references.connections.push(kx_app::ConnectionRef {
        descriptor: connector_path.to_string(),
        credential_ref: "KX_GMAIL_CREDENTIAL".to_string(),
    });
    if scope_secret {
        env.steering_config.guards.secret_scope = vec!["KX_GMAIL_CREDENTIAL".to_string()];
    }
    env.to_canonical_json().unwrap()
}

/// G2 LIVE witness (GR15/GR24): an App that references the bundled Gmail connector +
/// declares its credential in `secret_scope`, RUN via the new server-side `RunApp`, on
/// a live model. Proves the whole vertical: RunApp reads the stored envelope, resolves
/// the connection against the caller's own registry, grants the declared secret scope
/// to the agentic warrant, and the agent can dial the credentialed connector (FAKE
/// mode — a real MCP subprocess + real JSON-RPC round-trip, canned upstream). The
/// non-flaky invariant is settlement; whether `gmail/search` fired is LOGGED (model-
/// nondeterministic). Drive on Gemma-4 locally (never Qwen3).
#[tokio::test(flavor = "multi_thread")]
#[ignore = "real in-process LLM inference + the built kx-connector-gmail; opt in with --ignored"]
async fn runapp_gmail_connection_and_secret_scope_live() {
    let Some(gguf) = serve_model() else {
        eprintln!("skipping: no serve model — set KX_SERVE_MODEL_GGUF (Gemma-4 locally)");
        return;
    };
    let Some(connector) = gmail_connector_bin() else {
        eprintln!("skipping: kx-connector-gmail not built — `cargo build -p kx-connector-gmail`");
        return;
    };
    std::env::set_var("KX_SERVE_MODEL_GGUF", &gguf);
    std::env::set_var("KX_SERVE_AUTOGRANT", "1");
    // FAKE mode: the connector answers with canned data (no network, no real creds) —
    // inherited by the sidecar the gateway spawns.
    std::env::set_var("KX_GMAIL_FAKE", "1");

    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    // Register the Gmail connection (the connector namespaces its tools under `gmail/*`).
    let reg = c
        .register_mcp_server(proto::RegisterMcpServerRequest {
            server_name: "gmail".to_string(),
            transport: "stdio".to_string(),
            endpoint: connector.to_string_lossy().into_owned(),
            args: vec![],
            tls_required: false,
            credential_ref: "KX_GMAIL_CREDENTIAL".to_string(),
            session_mode: String::new(),
        })
        .await
        .expect("RegisterMcpServer")
        .into_inner();
    eprintln!("gmail connection: health={} discovered={}", reg.health, reg.discovered);
    // Set the credential secret (FAKE mode ignores its value; the scope must resolve).
    let _ = c
        .put_secret(proto::PutSecretRequest {
            name: "KX_GMAIL_CREDENTIAL".to_string(),
            value: r#"{"client_id":"x","client_secret":"y","refresh_token":"z"}"#.to_string(),
        })
        .await;

    // Save the App (references the connection + scopes its credential) and RUN it via
    // the NEW RunApp (which honors references.connections + guards.secret_scope).
    let envelope = gmail_app_envelope(&connector.to_string_lossy(), true);
    c.save_app(proto::SaveAppRequest {
        handle: "apps/local/gmail-agent".to_string(),
        envelope_json: envelope,
    })
    .await
    .expect("SaveApp")
    .into_inner();

    if !c
        .list_recipes(proto::ListRecipesRequest {})
        .await
        .unwrap()
        .into_inner()
        .recipes
        .iter()
        .any(|r| r.handle == REACT_RECIPE_HANDLE)
    {
        eprintln!("skipping the run leg: kx/recipes/react not provisioned");
        running.shutdown().await.unwrap();
        std::env::remove_var("KX_SERVE_AUTOGRANT");
        std::env::remove_var("KX_GMAIL_FAKE");
        return;
    }

    let handle = c
        .run_app(proto::RunAppRequest {
            handle: "apps/local/gmail-agent".to_string(),
            args: Vec::new(),
        })
        .await
        .expect("RunApp of the Gmail App")
        .into_inner();
    assert_eq!(handle.instance_id.len(), 16, "RunApp returns a run handle");

    // A generous window: a 12B model on Metal takes seconds per turn, and a
    // reason→tool→observe→answer chain is a few turns. The invariant is settlement,
    // not latency (LOGGED below).
    let mut tool_fired = false;
    let mut settled = false;
    for _ in 0..3000 {
        let turns = c
            .list_react_turns(proto::ListReactTurnsRequest {
                limit: None,
                instance_id: Some(handle.instance_id.clone()),
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
    eprintln!("LIVE RunApp Gmail: tool_fired={tool_fired} settled={settled}");
    assert!(settled, "the Gmail App settled to a terminal via RunApp on the live model");

    running.shutdown().await.unwrap();
    std::env::remove_var("KX_SERVE_AUTOGRANT");
    std::env::remove_var("KX_GMAIL_FAKE");
}
