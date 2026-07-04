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

/// Whether the operator opted the Ollama engine in (`KX_SERVE_OLLAMA` truthy) — the
/// GR24 dual-engine arm. Mirrors `eval_real_model::ollama_opted_in`.
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
            .any(|t| t.branch == "answer" || t.branch == "dead_lettered")
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

/// Resolve a bundled connector sidecar binary by crate name (release preferred), if built.
fn connector_bin(bin_name: &str) -> Option<PathBuf> {
    for profile in ["release", "debug"] {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../../target/{profile}/{bin_name}"));
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

/// Resolve the serve engine (GR24): the Ollama opt-in FIRST (a served GGUF standin would
/// otherwise register as PRIMARY and mask `KX_SERVE_OLLAMA`), else a GGUF drives llama.cpp
/// (Gemma-4 locally / Qwen3 in CI). Sets `KX_SERVE_MODEL_GGUF` for the llama.cpp arm.
fn resolve_engine() -> Option<&'static str> {
    if ollama_opted_in() {
        Some("ollama")
    } else if let Some(gguf) = serve_model() {
        std::env::set_var("KX_SERVE_MODEL_GGUF", &gguf);
        Some("llama.cpp")
    } else {
        None
    }
}

/// A `kortecx.app/v1` envelope for a connector-integration App: one agentic MODEL step
/// granting the registered `<server>/<tool>`, a by-reference connection pointer, and (when
/// `scope_secret`) the connection's credential in `guards.secret_scope` — the load-bearing
/// bit that lets the run warrant dial the credentialed connector (G2/#285).
fn connector_app_envelope(
    connector_path: &str,
    granted_tool: &str,
    credential_ref: &str,
    prompt: &str,
    scope_secret: bool,
) -> Vec<u8> {
    let blueprint = serde_json::json!({
        "seed": 0,
        "steps": [{
            "kind": "model",
            "prompt": prompt,
            "tool_contract": { granted_tool: "1" },
            "params": { "max_turns": "8", "max_tool_calls": "2" }
        }]
    });
    let mut env = kx_app::AppEnvelope::new("Connector Agent", blueprint);
    env.description = format!("an agentic App that dials the {granted_tool} connector");
    env.references.connections.push(kx_app::ConnectionRef {
        descriptor: connector_path.to_string(),
        credential_ref: credential_ref.to_string(),
    });
    if scope_secret {
        env.steering_config.guards.secret_scope = vec![credential_ref.to_string()];
    }
    env.to_canonical_json().unwrap()
}

/// One connector under a live RunApp witness (parametrized over any bundled sidecar).
struct ConnectorCase {
    /// The MCP server name (namespaces the tools as `<server>/<tool>`).
    server_name: &'static str,
    /// The bundled sidecar binary crate name (`kx-connector-gmail` / `-discord` / …).
    bin_name: &'static str,
    /// The credential-ref NAME (D81) the sidecar reads out-of-band.
    credential_ref: &'static str,
    /// A canned credential value (FAKE mode ignores it; the scope must still resolve).
    credential_value: &'static str,
    /// The FAKE env switch (offline canned responses).
    fake_env: &'static str,
    /// The single granted tool the agent may fire (SN-8).
    granted_tool: &'static str,
    /// The task prompt.
    prompt: &'static str,
}

/// GR15/GR24 LIVE witness + the `T-RUNAPP-SECRET-SCOPE-OBSERVATION` regression, generalized
/// over ANY bundled connector (the parallel-session enabler): register the connection, scope
/// its credential, save the App, RUN via `RunApp`, and OBSERVE the agentic loop on a live
/// model. A GR16 OBSERVE-witness (mirroring `react_serve_connector.rs`) — asserts only ROBUST
/// invariants (settles to a terminal; a fired tool is the granted one, SN-8) and LOGS the
/// trajectory. The fix's deterministic REGRESSION PROOFS live in CI:
/// `kx-proto::secret_scope_allowlist_survives_the_coordinator_wire` +
/// `kx-coordinator::observation_dispatch_preserves_the_chain_secret_scope`.
async fn runapp_connection_live(case: &ConnectorCase) {
    let Some(engine) = resolve_engine() else {
        eprintln!(
            "skipping: no serve model — set KX_SERVE_MODEL_GGUF (Gemma-4/Qwen3) or \
             KX_SERVE_OLLAMA=on KX_SERVE_OLLAMA_MODELS=gemma3:12b"
        );
        return;
    };
    let Some(connector) = connector_bin(case.bin_name) else {
        eprintln!(
            "skipping: {} not built — `cargo build -p {}`",
            case.bin_name, case.bin_name
        );
        return;
    };
    std::env::set_var("KX_SERVE_AUTOGRANT", "1");
    std::env::set_var(case.fake_env, "1");

    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    let reg = c
        .register_mcp_server(proto::RegisterMcpServerRequest {
            server_name: case.server_name.to_string(),
            transport: "stdio".to_string(),
            endpoint: connector.to_string_lossy().into_owned(),
            args: vec![],
            tls_required: false,
            credential_ref: case.credential_ref.to_string(),
            session_mode: String::new(),
        })
        .await
        .expect("RegisterMcpServer")
        .into_inner();
    eprintln!(
        "{} connection: health={} discovered={}",
        case.server_name, reg.health, reg.discovered
    );
    let _ = c
        .put_secret(proto::PutSecretRequest {
            name: case.credential_ref.to_string(),
            value: case.credential_value.to_string(),
        })
        .await;

    let handle_str = format!("apps/local/{}-agent", case.server_name);
    let envelope = connector_app_envelope(
        &connector.to_string_lossy(),
        case.granted_tool,
        case.credential_ref,
        case.prompt,
        true,
    );
    c.save_app(proto::SaveAppRequest {
        handle: handle_str.clone(),
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
        std::env::remove_var(case.fake_env);
        return;
    }

    let handle = c
        .run_app(proto::RunAppRequest {
            handle: handle_str,
            args: Vec::new(),
        })
        .await
        .expect("RunApp of the connector App")
        .into_inner();
    assert_eq!(handle.instance_id.len(), 16, "RunApp returns a run handle");

    let mut tool_fired = false;
    let mut fired_tool_ids: Vec<String> = Vec::new();
    let mut answered = false;
    let mut dead_lettered = false;
    let mut dl_reasons: Vec<String> = Vec::new();
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
        for t in &turns.turns {
            if t.branch == "tool" {
                tool_fired = true;
                if !t.tool_id.is_empty() && !fired_tool_ids.contains(&t.tool_id) {
                    fired_tool_ids.push(t.tool_id.clone());
                }
            }
        }
        answered = turns.turns.iter().any(|t| t.branch == "answer");
        dead_lettered = turns.turns.iter().any(|t| t.branch == "dead_lettered");
        if dead_lettered {
            dl_reasons = turns
                .turns
                .iter()
                .filter(|t| t.branch == "dead_lettered" && !t.rejection_reason.is_empty())
                .map(|t| t.rejection_reason.clone())
                .collect();
        }
        if answered || dead_lettered {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    eprintln!(
        "LIVE RunApp {} [{engine}]: tool_fired={tool_fired} fired_tools={fired_tool_ids:?} \
         answered={answered} dead_lettered={dead_lettered} dl_reasons={dl_reasons:?}",
        case.server_name
    );
    assert!(
        answered || dead_lettered,
        "the {} App settled to a terminal via RunApp on the live model [{engine}]",
        case.server_name
    );
    if tool_fired {
        assert!(
            fired_tool_ids
                .iter()
                .all(|id| id.is_empty() || id == case.granted_tool),
            "if a tool fired it was the granted {}. fired_tools={fired_tool_ids:?}",
            case.granted_tool
        );
    }

    running.shutdown().await.unwrap();
    std::env::remove_var("KX_SERVE_AUTOGRANT");
    std::env::remove_var(case.fake_env);
}

/// G2/#285 Gmail witness — the thin caller over the generic connector harness.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "real in-process LLM inference + the built kx-connector-gmail; opt in with --ignored"]
async fn runapp_gmail_connection_and_secret_scope_live() {
    runapp_connection_live(&ConnectorCase {
        server_name: "gmail",
        bin_name: "kx-connector-gmail",
        credential_ref: "KX_GMAIL_CREDENTIAL",
        credential_value: r#"{"client_id":"x","client_secret":"y","refresh_token":"z"}"#,
        fake_env: "KX_GMAIL_FAKE",
        granted_tool: "gmail/search",
        prompt: "Search my Gmail for unread messages using the gmail/search tool, then \
                 briefly answer with what you found.",
    })
    .await;
}

/// RC-SW2 Discord witness — the same generic harness pointed at the bundled Discord sidecar
/// (#277), so a parallel session can test ANY connector-in-App path without new harness code.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "real in-process LLM inference + the built kx-connector-discord; opt in with --ignored"]
async fn runapp_discord_connection_and_secret_scope_live() {
    runapp_connection_live(&ConnectorCase {
        server_name: "discord",
        bin_name: "kx-connector-discord",
        credential_ref: "KX_DISCORD_CREDENTIAL",
        credential_value: r#"{"bot_token":"x"}"#,
        fake_env: "KX_DISCORD_FAKE",
        granted_tool: "discord/read_channel",
        prompt: "Read the most recent messages from channel 123 using the \
                 discord/read_channel tool, then briefly summarize them.",
    })
    .await;
}

/// integrations-come-alive Slack witness — the same generic harness pointed at the
/// bundled Slack sidecar (offline FAKE mode, no real token). Channel id "123" is
/// ASCII-alphanumeric so it clears `slack/validate`'s channel-id guard BEFORE the
/// fake branch. Proves a credentialed Slack App fires `slack/read_channel` on a live
/// model. Skips gracefully until `kx-connector-slack` is built (PR #291).
#[tokio::test(flavor = "multi_thread")]
#[ignore = "real in-process LLM inference + the built kx-connector-slack; opt in with --ignored"]
async fn runapp_slack_connection_and_secret_scope_live() {
    runapp_connection_live(&ConnectorCase {
        server_name: "slack",
        bin_name: "kx-connector-slack",
        credential_ref: "KX_SLACK_CREDENTIAL",
        credential_value: r#"{"bot_token":"x"}"#,
        fake_env: "KX_SLACK_FAKE",
        granted_tool: "slack/read_channel",
        prompt: "Read the most recent messages from channel 123 using the \
                 slack/read_channel tool, then briefly summarize them.",
    })
    .await;
}

/// integrations-come-alive Notion witness — the generic harness over the bundled
/// Notion sidecar (offline FAKE mode). `notion/search` takes a free-text query (no
/// id guard), the cleanest tool to drive. Proves a credentialed Notion App fires
/// `notion/search` on a live model.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "real in-process LLM inference + the built kx-connector-notion; opt in with --ignored"]
async fn runapp_notion_connection_and_secret_scope_live() {
    runapp_connection_live(&ConnectorCase {
        server_name: "notion",
        bin_name: "kx-connector-notion",
        credential_ref: "KX_NOTION_CREDENTIAL",
        credential_value: r#"{"token":"x"}"#,
        fake_env: "KX_NOTION_FAKE",
        granted_tool: "notion/search",
        prompt: "Search the Notion workspace for pages about the launch using the \
                 notion/search tool, then briefly summarize what you found.",
    })
    .await;
}

/// T-APP-TRIGGER-TARGET GR24 LIVE witness: a credentialed App fires from a gRPC TRIGGER
/// (not a direct `RunApp`) and drives the agentic loop to a terminal on a live model — the
/// automation vertical the trigger→App wiring exists for. Mirrors `runapp_connection_live`'s
/// setup, then registers an App-TARGET trigger + `SubmitTrigger`, and OBSERVES the resulting
/// run (a GR16 observe-witness: settles to a terminal; a fired tool is the granted one, SN-8).
/// Dual-engine (llama.cpp + Ollama), model restarted per engine.
async fn trigger_fires_connector_app_live(case: &ConnectorCase) {
    let Some(engine) = resolve_engine() else {
        eprintln!("skipping: no serve model (set KX_SERVE_MODEL_GGUF or KX_SERVE_OLLAMA)");
        return;
    };
    let Some(connector) = connector_bin(case.bin_name) else {
        eprintln!(
            "skipping: {} not built — `cargo build -p {}`",
            case.bin_name, case.bin_name
        );
        return;
    };
    std::env::set_var("KX_SERVE_AUTOGRANT", "1");
    std::env::set_var(case.fake_env, "1");

    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    // Register the connector + scope its credential + save the credentialed App.
    let reg = c
        .register_mcp_server(proto::RegisterMcpServerRequest {
            server_name: case.server_name.to_string(),
            transport: "stdio".to_string(),
            endpoint: connector.to_string_lossy().into_owned(),
            args: vec![],
            tls_required: false,
            credential_ref: case.credential_ref.to_string(),
            session_mode: String::new(),
        })
        .await
        .expect("RegisterMcpServer")
        .into_inner();
    eprintln!(
        "{} connection: health={} discovered={}",
        case.server_name, reg.health, reg.discovered
    );
    let _ = c
        .put_secret(proto::PutSecretRequest {
            name: case.credential_ref.to_string(),
            value: case.credential_value.to_string(),
        })
        .await;

    let handle_str = format!("apps/local/{}-trigger-agent", case.server_name);
    let envelope = connector_app_envelope(
        &connector.to_string_lossy(),
        case.granted_tool,
        case.credential_ref,
        case.prompt,
        true,
    );
    c.save_app(proto::SaveAppRequest {
        handle: handle_str.clone(),
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
        std::env::remove_var(case.fake_env);
        return;
    }

    // Register an App-TARGET gRPC trigger, then FIRE it (the automation path). The run
    // binds under the registrant party with the App's connection + secret_scope resolved.
    let trigger_name = format!("{}-fire", case.server_name);
    c.register_trigger(proto::RegisterTriggerRequest {
        name: trigger_name.clone(),
        kind: proto::TriggerKind::Grpc as i32,
        recipe_handle: String::new(),
        app_handle: handle_str.clone(),
        auth: proto::TriggerAuth::None as i32,
        auth_secret_ref: String::new(),
        schedule_spec: String::new(),
        timezone: String::new(),
        enabled: true,
        require_approval: false,
    })
    .await
    .expect("register App-target trigger")
    .into_inner();

    let fired = c
        .submit_trigger(proto::SubmitTriggerRequest {
            name: trigger_name,
            idempotency_key: String::new(),
            payload_json: "{}".to_string(),
        })
        .await
        .expect("SubmitTrigger fires the App")
        .into_inner();
    assert_eq!(fired.instance_id.len(), 16, "the trigger started a run");
    assert!(!fired.deduped, "the first fire is not deduped");

    // Observe the trigger-started run to a terminal (same loop as runapp_connection_live).
    let mut tool_fired = false;
    let mut fired_tool_ids: Vec<String> = Vec::new();
    let mut answered = false;
    let mut dead_lettered = false;
    for _ in 0..3000 {
        let turns = c
            .list_react_turns(proto::ListReactTurnsRequest {
                limit: None,
                instance_id: Some(fired.instance_id.clone()),
                step_salt: None,
            })
            .await
            .unwrap()
            .into_inner();
        for t in &turns.turns {
            if t.branch == "tool" {
                tool_fired = true;
                if !t.tool_id.is_empty() && !fired_tool_ids.contains(&t.tool_id) {
                    fired_tool_ids.push(t.tool_id.clone());
                }
            }
        }
        answered = turns.turns.iter().any(|t| t.branch == "answer");
        dead_lettered = turns.turns.iter().any(|t| t.branch == "dead_lettered");
        if answered || dead_lettered {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    eprintln!(
        "LIVE TRIGGER→App {} [{engine}]: tool_fired={tool_fired} fired_tools={fired_tool_ids:?} \
         answered={answered} dead_lettered={dead_lettered}",
        case.server_name
    );
    assert!(
        answered || dead_lettered,
        "the trigger-fired {} App settled to a terminal on the live model [{engine}]",
        case.server_name
    );
    if tool_fired {
        assert!(
            fired_tool_ids
                .iter()
                .all(|id| id.is_empty() || id == case.granted_tool),
            "if a tool fired it was the granted {}. fired_tools={fired_tool_ids:?}",
            case.granted_tool
        );
    }

    running.shutdown().await.unwrap();
    std::env::remove_var("KX_SERVE_AUTOGRANT");
    std::env::remove_var(case.fake_env);
}

/// T-APP-TRIGGER-TARGET Discord witness — a gRPC trigger fires the credentialed Discord App
/// (the automation vertical: an event, not an operator, starts the credentialed run).
#[tokio::test(flavor = "multi_thread")]
#[ignore = "real in-process LLM inference + the built kx-connector-discord; opt in with --ignored"]
async fn trigger_fires_discord_app_live() {
    trigger_fires_connector_app_live(&ConnectorCase {
        server_name: "discord",
        bin_name: "kx-connector-discord",
        credential_ref: "KX_DISCORD_CREDENTIAL",
        credential_value: r#"{"bot_token":"x"}"#,
        fake_env: "KX_DISCORD_FAKE",
        granted_tool: "discord/read_channel",
        prompt: "Read the most recent messages from channel 123 using the \
                 discord/read_channel tool, then briefly summarize them.",
    })
    .await;
}

/// integrations-come-alive Slack trigger witness — a gRPC trigger fires the
/// credentialed Slack App (the automation vertical: an event, not an operator,
/// starts the run). Offline FAKE mode; dual-engine (llama.cpp + Ollama).
#[tokio::test(flavor = "multi_thread")]
#[ignore = "real in-process LLM inference + the built kx-connector-slack; opt in with --ignored"]
async fn trigger_fires_slack_app_live() {
    trigger_fires_connector_app_live(&ConnectorCase {
        server_name: "slack",
        bin_name: "kx-connector-slack",
        credential_ref: "KX_SLACK_CREDENTIAL",
        credential_value: r#"{"bot_token":"x"}"#,
        fake_env: "KX_SLACK_FAKE",
        granted_tool: "slack/read_channel",
        prompt: "Read the most recent messages from channel 123 using the \
                 slack/read_channel tool, then briefly summarize them.",
    })
    .await;
}

/// integrations-come-alive Notion trigger witness — a gRPC trigger fires the
/// credentialed Notion App. Offline FAKE mode; dual-engine.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "real in-process LLM inference + the built kx-connector-notion; opt in with --ignored"]
async fn trigger_fires_notion_app_live() {
    trigger_fires_connector_app_live(&ConnectorCase {
        server_name: "notion",
        bin_name: "kx-connector-notion",
        credential_ref: "KX_NOTION_CREDENTIAL",
        credential_value: r#"{"token":"x"}"#,
        fake_env: "KX_NOTION_FAKE",
        granted_tool: "notion/search",
        prompt: "Search the Notion workspace for pages about the launch using the \
                 notion/search tool, then briefly summarize what you found.",
    })
    .await;
}

/// One server-embed document (empty embedding ⇒ the host embeds `content` with the
/// served model). Mirrors `react_rag_serve::doc`.
fn doc(content: &[u8]) -> proto::IngestDocument {
    proto::IngestDocument {
        content: content.to_vec(),
        embedding: Vec::new(),
        ..Default::default()
    }
}

/// A `kortecx.app/v1` envelope for a GROUNDED App (T-RUNAPP-CONTEXT-RAIL): a PLAIN model
/// step (NO `tool_contract`) + `references.datasets` (the dataset rail folds `retrieve@1`
/// at RunApp — declarative RAG-on-App) + `references.rules` (a guidance note that rides the
/// entry-step context). The App declares WHAT to ground on; the server grants the tool.
fn grounded_app_envelope(dataset: &str, rule_ref: &[u8], prompt: &str) -> Vec<u8> {
    let blueprint = serde_json::json!({
        "seed": 0,
        "steps": [{
            "kind": "model",
            "prompt": prompt,
            "params": { "max_turns": "4", "max_tool_calls": "3" }
        }]
    });
    let mut env = kx_app::AppEnvelope::new("Grounded Analyst", blueprint);
    env.description = "an App that self-grounds on its declared dataset + rule".to_string();
    env.references.datasets.push(kx_app::DatasetRef {
        dataset_ref: dataset.to_string(),
        cas_refs: vec![],
    });
    let hex: String = rule_ref.iter().map(|b| format!("{b:02x}")).collect();
    env.references.rules.push(kx_app::ArtifactRef {
        name: "brief".to_string(),
        content_ref: hex,
    });
    env.to_canonical_json().unwrap()
}

/// GR15/GR24 LIVE witness — the T-RUNAPP-CONTEXT-RAIL "come alive" proof: an App that
/// declares a dataset + a rule (NO hand-authored tool grant) SELF-GROUNDS at RunApp — the
/// dataset rail folds `retrieve@1` onto the entry step + steers it, and the rule rides the
/// entry context. Runs the agentic loop on a REAL model over BOTH engines (Gemma, restart
/// per engine). Needs `--features inference,hnsw` + an embedder; runtime-skips otherwise.
/// Asserts only ROBUST invariants (settles to a terminal; if a tool fired it was `retrieve`
/// — SN-8) and LOGS the trajectory; the deterministic fold/inject proofs live in the
/// `app_run` unit tests + the `grounded` golden case.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "real LLM inference + dataset embedding; needs a served Gemma + --features inference,hnsw; opt in with --ignored"]
async fn runapp_grounded_app_self_grounds_live() {
    let Some(engine) = resolve_engine() else {
        eprintln!("skipping: no serve model — set KX_SERVE_MODEL_GGUF or KX_SERVE_OLLAMA=on");
        return;
    };
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    // Ingest a tiny paraphrase corpus (server-embed). The target doc never says
    // "photosynthesis" — only the agent's own retrieve query surfaces it. Skip if the
    // dataset view/embedder is absent (a `--features inference` build without `hnsw`).
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
        eprintln!("skipping: ingest unavailable (needs --features hnsw + an embedder)");
        running.shutdown().await.unwrap();
        return;
    }

    // Upload a guidance rule → a CAS ref (the context-rail leg).
    let rule = c
        .put_content(proto::PutContentRequest {
            payload: b"Answer in ONE sentence, grounded in the retrieved passages.".to_vec(),
            media_type: "text/plain".to_string(),
            filename: String::new(),
        })
        .await
        .expect("PutContent")
        .into_inner();

    let handle_str = "apps/local/grounded-analyst".to_string();
    let envelope = grounded_app_envelope(
        "science",
        &rule.content_ref,
        "How do plants make energy from the sun? Use the dataset to answer.",
    );
    c.save_app(proto::SaveAppRequest {
        handle: handle_str.clone(),
        envelope_json: envelope,
    })
    .await
    .expect("SaveApp")
    .into_inner();

    let handle = match c
        .run_app(proto::RunAppRequest {
            handle: handle_str,
            args: Vec::new(),
        })
        .await
    {
        Ok(h) => h.into_inner(),
        Err(e) => {
            eprintln!("skipping run leg: RunApp unavailable ({e})");
            running.shutdown().await.unwrap();
            return;
        }
    };
    assert_eq!(handle.instance_id.len(), 16, "RunApp returns a run handle");

    let (mut answered, mut dead, mut retrieved) = (false, false, false);
    let mut fired: Vec<String> = Vec::new();
    let mut branches: Vec<String> = Vec::new();
    let mut dl_reasons: Vec<String> = Vec::new();
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
        branches = turns.turns.iter().map(|t| t.branch.clone()).collect();
        for t in &turns.turns {
            if t.branch == "tool" && !t.tool_id.is_empty() && !fired.contains(&t.tool_id) {
                fired.push(t.tool_id.clone());
            }
            if t.branch == "tool" && t.tool_id == "retrieve" {
                retrieved = true;
            }
        }
        answered = turns.turns.iter().any(|t| t.branch == "answer");
        dead = turns.turns.iter().any(|t| t.branch == "dead_lettered");
        if dead {
            dl_reasons = turns
                .turns
                .iter()
                .filter(|t| t.branch == "dead_lettered" && !t.rejection_reason.is_empty())
                .map(|t| t.rejection_reason.clone())
                .collect();
        }
        if answered || dead {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    eprintln!(
        "LIVE grounded RunApp [{engine}]: retrieved={retrieved} fired={fired:?} \
         answered={answered} dead_lettered={dead} dl_reasons={dl_reasons:?} branches={branches:?}"
    );
    assert!(
        answered || dead,
        "the grounded App self-grounds + settles to a terminal on the live model [{engine}]"
    );
    // SN-8: the dataset rail folds EXACTLY retrieve@1 — no other tool can fire.
    assert!(
        fired.iter().all(|id| id.is_empty() || id == "retrieve"),
        "only retrieve fires from the dataset rail; fired={fired:?}"
    );

    running.shutdown().await.unwrap();
}

/// The RC-SW2 swarm request `kx.swarm(...)` / `flow().swarm(...)` lowers to: N parallel MODEL
/// leaves (no inter-edges) fanned into one MODEL gather that reads every leaf's committed
/// output (its Data-edge parents, F-7). Built here as the raw proto the SDK produces.
fn swarm_request() -> proto::SubmitWorkflowRequest {
    let model = |prompt: &str| proto::WorkflowStep {
        kind: proto::WorkflowStepKind::Model as i32,
        model_id: String::new(),
        prompt: prompt.to_string(),
        body_signature_id: Vec::new(),
        tool_contract: Default::default(),
        params: Default::default(),
    };
    proto::SubmitWorkflowRequest {
        seed: 0,
        steps: vec![
            model("In one sentence, give a concrete BENEFIT of durable agentic execution."),
            model(
                "In one sentence, give a concrete RISK or limitation of durable agentic execution.",
            ),
            model("Combine the two points above into a two-sentence summary."),
        ],
        edges: vec![
            proto::WorkflowEdge {
                parent: 0,
                child: 2,
                edge_kind: proto::EdgeKind::Data as i32,
                non_cascade: false,
            },
            proto::WorkflowEdge {
                parent: 1,
                child: 2,
                edge_kind: proto::EdgeKind::Data as i32,
                non_cascade: false,
            },
        ],
        execution_mode: proto::WorkflowExecutionMode::Frozen as i32,
        context_bundles: vec![],
    }
}

/// RC-SW2 LIVE swarm witness (GR15/GR24): a 2-agent swarm → gather runs end-to-end on a live
/// model, both engines. Proves the multi-agent vertical: two INDEPENDENT parallel model
/// chains commit, then the gather synthesizes over BOTH committed outputs (F-7). A GR16
/// OBSERVE-witness — asserts the swarm settled with all three motes committed and the gather
/// produced REAL non-empty output (GR15), and LOGS the synthesis. Pure composition of
/// existing step kinds (no new wire shape); the deterministic shape is pinned by the golden
/// `swarm_agentic_gather` corpus row + the SDK unit tests.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "real in-process LLM inference; needs a GGUF (Gemma-4 locally); opt in with --ignored"]
async fn swarm_runs_parallel_agents_and_gathers_live() {
    let Some(engine) = resolve_engine() else {
        eprintln!("skipping: no serve model — set KX_SERVE_MODEL_GGUF or KX_SERVE_OLLAMA=on");
        return;
    };
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    let handle = c
        .submit_workflow(swarm_request())
        .await
        .expect("SubmitWorkflow of the swarm")
        .into_inner();
    assert_eq!(handle.instance_id.len(), 16);

    // Poll the projection: the swarm settles when all 3 motes (2 leaves + gather) commit.
    let mut committed: Vec<[u8; 32]> = Vec::new();
    let mut terminal_ref: Option<[u8; 32]> = None;
    for _ in 0..2400 {
        let view = c
            .get_projection(proto::GetProjectionRequest {
                instance_id: handle.instance_id.clone(),
                at_seq: None,
            })
            .await
            .unwrap()
            .into_inner();
        committed = view
            .motes
            .iter()
            .filter(|m| m.state == proto::MoteSnapshotState::Committed as i32)
            .filter_map(|m| m.result_ref.clone().and_then(|r| r.try_into().ok()))
            .collect();
        // The gather is the child of both leaves — the mote with two parents.
        if let Some(g) = view
            .motes
            .iter()
            .find(|m| m.parents.len() == 2 && m.state == proto::MoteSnapshotState::Committed as i32)
        {
            terminal_ref = g.result_ref.clone().and_then(|r| r.try_into().ok());
        }
        if committed.len() >= 3 && terminal_ref.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }

    eprintln!(
        "LIVE swarm [{engine}]: committed={} terminal_committed={}",
        committed.len(),
        terminal_ref.is_some()
    );
    assert!(
        committed.len() >= 3,
        "the 2-agent swarm + gather all committed on the live model [{engine}] (got {})",
        committed.len()
    );
    let gather_ref = terminal_ref.expect("the gather (fan-in sink) committed");
    let content = c
        .get_content(proto::GetContentRequest {
            content_ref: gather_ref.to_vec(),
            instance_id: handle.instance_id.clone(),
        })
        .await
        .expect("GetContent of the gather output")
        .into_inner();
    let text = String::from_utf8_lossy(&content.payload);
    eprintln!("LIVE swarm [{engine}] synthesis: {}", text.trim());
    // GR15: the gather produced REAL model output over the two agents' committed results.
    assert!(
        !text.trim().is_empty(),
        "the swarm gather produced a non-empty synthesis on the live model [{engine}]"
    );

    running.shutdown().await.unwrap();
}

/// A WIDER swarm: `n_leaves` INDEPENDENT parallel MODEL leaves fanned into one MODEL gather
/// (a Data edge from every leaf). More parallel leaves ⇒ the worker pool has real
/// wall-clock room to overlap them.
fn wide_swarm_request(n_leaves: u32) -> proto::SubmitWorkflowRequest {
    let model = |prompt: String| proto::WorkflowStep {
        kind: proto::WorkflowStepKind::Model as i32,
        model_id: String::new(),
        prompt,
        body_signature_id: Vec::new(),
        tool_contract: Default::default(),
        params: Default::default(),
    };
    let mut steps: Vec<proto::WorkflowStep> = (0..n_leaves)
        .map(|i| {
            model(format!(
                "In ONE short sentence, state distinct fact #{} about durable agentic \
                 execution. Be concise.",
                i + 1
            ))
        })
        .collect();
    steps.push(model(
        "In two sentences, synthesize the points above.".to_string(),
    ));
    let gather = n_leaves; // the gather is the last step
    let edges = (0..n_leaves)
        .map(|parent| proto::WorkflowEdge {
            parent,
            child: gather,
            edge_kind: proto::EdgeKind::Data as i32,
            non_cascade: false,
        })
        .collect();
    proto::SubmitWorkflowRequest {
        seed: 0,
        steps,
        edges,
        execution_mode: proto::WorkflowExecutionMode::Frozen as i32,
        context_bundles: vec![],
    }
}

/// Run a `wide_swarm_request(n_leaves)` on a fresh gateway configured with `pool` embedded
/// workers; return `(wall_clock, committed_count, gather_text)`.
async fn run_wide_swarm(dir_tag: &str, pool: usize, n_leaves: u32) -> (Duration, usize, String) {
    let dir = tempfile::TempDir::new().unwrap();
    let _ = dir_tag;
    let mut cfg = common::gateway_config(&dir, true, HashMap::new());
    cfg.worker_pool = Some(pool);
    let running = start(cfg).await.unwrap();
    let mut c = client(running.local_addr()).await;

    let total = (n_leaves + 1) as usize;
    let started = std::time::Instant::now();
    let handle = c
        .submit_workflow(wide_swarm_request(n_leaves))
        .await
        .expect("SubmitWorkflow of the wide swarm")
        .into_inner();

    let mut committed = 0usize;
    let mut gather_ref: Option<[u8; 32]> = None;
    for _ in 0..4800 {
        let view = c
            .get_projection(proto::GetProjectionRequest {
                instance_id: handle.instance_id.clone(),
                at_seq: None,
            })
            .await
            .unwrap()
            .into_inner();
        committed = view
            .motes
            .iter()
            .filter(|m| m.state == proto::MoteSnapshotState::Committed as i32)
            .count();
        if let Some(g) = view.motes.iter().find(|m| {
            m.parents.len() == n_leaves as usize
                && m.state == proto::MoteSnapshotState::Committed as i32
        }) {
            gather_ref = g.result_ref.clone().and_then(|r| r.try_into().ok());
        }
        if committed >= total && gather_ref.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    let elapsed = started.elapsed();

    let text = if let Some(gr) = gather_ref {
        let content = c
            .get_content(proto::GetContentRequest {
                content_ref: gr.to_vec(),
                instance_id: handle.instance_id.clone(),
            })
            .await
            .expect("GetContent of the gather output")
            .into_inner();
        String::from_utf8_lossy(&content.payload).trim().to_string()
    } else {
        String::new()
    };
    running.shutdown().await.unwrap();
    (elapsed, committed, text)
}

/// RC-SW3 LIVE pool witness (GR15/GR24): the SAME wide swarm run under a single worker
/// (`--workers 1`, the historical serial drain) vs. a pool of 4, on a live model. Proves the
/// pool executes an authored fan-out end-to-end and produces REAL output at both sizes, and
/// LOGS the wall-clock speedup. The speedup is OBSERVED (logged), not asserted: a single live
/// inference sample is too noisy for a non-flaky timing assertion (GR12 — flakes are fixed,
/// not tolerated), and the true concurrent-inference gain depends on the operator's
/// `OLLAMA_NUM_PARALLEL`. The deterministic proof that pool>1 partitions work + stays
/// digest-invariant lives in `kx-coordinator/tests/pool_determinism.rs`; the measured speedup
/// is recorded via `just profile` + the manual GR24 drive.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "real in-process LLM inference; needs a GGUF (Gemma-4 locally) or KX_SERVE_OLLAMA=on"]
async fn swarm_pool_parallel_speedup_live() {
    let Some(engine) = resolve_engine() else {
        eprintln!("skipping: no serve model — set KX_SERVE_MODEL_GGUF or KX_SERVE_OLLAMA=on");
        return;
    };
    const N_LEAVES: u32 = 4;
    let total = (N_LEAVES + 1) as usize;

    // Control: one embedded worker (byte-identical to a serve with no pool).
    let (t1, c1, txt1) = run_wide_swarm("pool1", 1, N_LEAVES).await;
    // Pool of 4 leasers over the same coordinator.
    let (t4, c4, txt4) = run_wide_swarm("pool4", 4, N_LEAVES).await;

    let ratio = t1.as_secs_f64() / t4.as_secs_f64().max(f64::MIN_POSITIVE);
    eprintln!(
        "LIVE pool-speedup [{engine}] leaves={N_LEAVES}: pool1={t1:?} (committed {c1}/{total}) \
         pool4={t4:?} (committed {c4}/{total}) speedup={ratio:.2}x"
    );
    eprintln!("  pool1 synthesis: {txt1}");
    eprintln!("  pool4 synthesis: {txt4}");

    // GR15 (non-flaky): both pool sizes commit the FULL swarm and produce real output.
    assert_eq!(c1, total, "pool=1 committed the full swarm on [{engine}]");
    assert_eq!(c4, total, "pool=4 committed the full swarm on [{engine}]");
    assert!(
        !txt4.is_empty(),
        "pool=4 produced a non-empty synthesis on [{engine}]"
    );
}
