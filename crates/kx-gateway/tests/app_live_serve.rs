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
