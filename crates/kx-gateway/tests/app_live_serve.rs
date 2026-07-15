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
            source_digest: Vec::new(),
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

/// The `SubmitWorkflow` request the CLI `kx chat --tools <tool>@1` + SDK `chat(tools=…)` build
/// (`kx-cli::verbs::chat::build_agentic_request`): ONE agentic MODEL step whose `tool_contract`
/// names ONLY the granted tool, plus the bounded ReAct budget in the canonical react keys. The
/// SERVER builds the SCOPED warrant FROM the contract (SN-8) — no client warrant, no autogrant.
fn chat_tools_request(prompt: &str, granted_tool: &str) -> proto::SubmitWorkflowRequest {
    proto::SubmitWorkflowRequest {
        seed: 0,
        steps: vec![proto::WorkflowStep {
            kind: proto::WorkflowStepKind::Model as i32,
            model_id: String::new(),
            prompt: prompt.to_string(),
            body_signature_id: Vec::new(),
            tool_contract: [(granted_tool.to_string(), "1".to_string())]
                .into_iter()
                .collect(),
            params: [
                ("max_turns".to_string(), b"8".to_vec()),
                ("max_tool_calls".to_string(), b"20".to_vec()),
            ]
            .into_iter()
            .collect(),
        }],
        edges: vec![],
        execution_mode: proto::WorkflowExecutionMode::Frozen as i32,
        context_bundles: vec![],
    }
}

/// (`kx chat --tools` / `chat(tools=…)`): a chat turn with an EXPLICIT `tool_contract` fires
/// the named tool through a SERVER-BUILT SCOPED warrant (SN-8), proven WITHOUT
/// `KX_SERVE_AUTOGRANT` — the bundled `mcp-echo/echo` capability is registered whenever a model
/// is served (`register_echo_capability`, gated on the model, NOT autogrant), so the grant that
/// fires comes from the CONTRACT, never the react-auto blanket. The settle poll is SCOPED by the
/// server-returned `react_chain_salt` so a shared-journal serve surfaces THIS turn's chain.
/// Dual-engine (GR28): Gemma-4 completes fire→answer; gemma3 fires but may honestly dead-letter on
/// a missing required arg (`T-GEMMA3-OLLAMA-TOOL-ARG-SCHEMA`) — settling to a terminal is the
/// robust invariant (GR15); `tool_fired` is LOGGED (model-nondeterministic).
#[tokio::test(flavor = "multi_thread")]
#[ignore = "real in-process LLM inference; needs a served model + bundled kx-mcp-echo; opt in with --ignored"]
async fn chat_with_explicit_tools_fires_scoped_by_salt_no_autogrant() {
    let Some(engine) = resolve_engine() else {
        eprintln!("skipping: no serve model — set KX_SERVE_MODEL_GGUF or KX_SERVE_OLLAMA=on");
        return;
    };
    // Deliberately NO KX_SERVE_AUTOGRANT: prove the EXPLICIT tool_contract grant fires on its own.
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    // Skip the run leg if the react recipe / bundled echo is absent (a model-free / echo-free serve).
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
        return;
    }

    let handle = c
        .submit_workflow(chat_tools_request(
            "Use the echo tool to echo the word 'pong', then answer with it.",
            "mcp-echo/echo",
        ))
        .await
        .expect("SubmitWorkflow of the chat --tools turn")
        .into_inner();
    assert_eq!(handle.instance_id.len(), 16);
    // A single tool-granted (agentic) MODEL step MUST report its 32B chain salt — the key
    // that scopes ListReactTurns on serve's shared journal (an empty salt would strand the poll).
    assert_eq!(
        handle.react_chain_salt.len(),
        32,
        "An agentic tool-granted step reports its react_chain_salt"
    );

    let mut tool_fired = false;
    let mut settled = false;
    for _ in 0..900 {
        let turns = c
            .list_react_turns(proto::ListReactTurnsRequest {
                limit: None,
                instance_id: Some(handle.instance_id.clone()),
                // Scope the poll to THIS turn's chain via the server-returned salt.
                step_salt: Some(handle.react_chain_salt.clone()),
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
    eprintln!("chat --tools LIVE ({engine}): tool_fired={tool_fired} settled={settled}");
    // SN-8: the model could fire ONLY mcp-echo/echo (the sole granted tool) — the scoped warrant
    // fail-closes any other tool at the broker, so a fired tool IS the grant. Settling is robust.
    assert!(
        settled,
        "the explicit-tools chat turn settled to a terminal on the live model"
    );
    running.shutdown().await.unwrap();
}

/// SN-8 (DETERMINISTIC — no model): a `chat --tools` `SubmitWorkflow` that CANNOT be admitted
/// is REFUSED at AUTHORING (`InvalidArgument`), never bound — the agentic path fails closed
/// client-side, so an un-vetted tool is never silently accepted. The specific reason is
/// environment-dependent (a model-free serve refuses the MODEL step "requires a served model";
/// a served serve refuses the unregistered tool by name), so this pins the STABLE invariant —
/// the `InvalidArgument` refusal — not the message. The POSITIVE scoping (the granted tool fires,
/// no autogrant) is the live witness above.
#[tokio::test(flavor = "multi_thread")]
async fn chat_tools_workflow_is_refused_at_authoring_when_unadmittable() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    let status = c
        .submit_workflow(chat_tools_request("echo pong", "no-such/tool"))
        .await
        .expect_err("an un-admittable chat --tools workflow must be refused, not accepted");
    assert_eq!(
        status.code(),
        tonic::Code::InvalidArgument,
        "refused at authoring (never admitted), got: {status:?}"
    );
    running.shutdown().await.unwrap();
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
    /// gemma3 connector-tool-fire (GR28): HARD-assert the tool fired on this run. Set for
    /// cases validated on BOTH engines (the union format makes gemma3 emit a parseable call,
    /// so a tool-NECESSITATING prompt reliably fires on both llama.cpp AND Ollama gemma3).
    require_fire: bool,
    /// `T-GEMMA3-TOOL-LOOP-ANSWER-FORCE` (GR28): OBSERVE (log, never assert) whether the chain
    /// COMPLETES after firing. Set for cases we EXPECT to complete once the separate
    /// missing-required-args gap (`T-GEMMA3-OLLAMA-TOOL-ARG-SCHEMA`) lands. Not a hard gate: a
    /// weak model can honestly dead-letter (e.g. it omits a required tool arg → the observation
    /// fails), which is CORRECT per GR15. The answer-force itself is proven deterministically +
    /// by `answer_only_format_forces_gemma3_to_settle_live`.
    require_answer: bool,
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
        source_digest: Vec::new(),
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
    // GR28: the gemma3 connector-tool-fire proof — a tool-necessitating prompt FIRES the
    // granted tool on BOTH engines (the union format + priming).
    if case.require_fire {
        assert!(
            tool_fired && fired_tool_ids.iter().any(|id| id == case.granted_tool),
            "the {} App must FIRE {} on the live model [{engine}] (union format) — \
             tool_fired={tool_fired} fired={fired_tool_ids:?}",
            case.server_name,
            case.granted_tool
        );
    }
    // T-GEMMA3-TOOL-LOOP-ANSWER-FORCE: loop-completeness is OBSERVED here, not GATED. The
    // answer-force is proven deterministically (unit + coordinator full-seam) and by the
    // `answer_only_format_forces_gemma3_to_settle_live` witness; a hard "always answers" gate
    // would be INVALID — gemma3 has failure modes BEYOND the duplicate-loop this PR fixes
    // (notably MISSING required tool args → the observation fails → an HONEST dead-letter at
    // turn 0, correct per GR15: never fabricate an answer the model couldn't produce; tracked
    // as `T-GEMMA3-OLLAMA-TOOL-ARG-SCHEMA`). So we log completion, never assert it.
    if case.require_answer && !answered {
        eprintln!(
            "[note] {} did NOT complete the loop on [{engine}] (dead_lettered={dead_lettered}) — \
             expected once T-GEMMA3-OLLAMA-TOOL-ARG-SCHEMA (missing-required-args) lands; the \
             answer-force (duplicate/nudge) is proven separately",
            case.server_name
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
        require_fire: false, // observe-only (not re-validated with the union format this PR)
        require_answer: false,
    })
    .await;
}

/// Discord witness — the same generic harness pointed at the bundled Discord sidecar
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
        require_fire: false, // observe-only (validated pre-union in #290; not the focus here)
        require_answer: false,
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
        require_fire: true, // GR28: gemma3 + llama.cpp both FIRE slack/read_channel (union format)
        require_answer: true, // GR28: OBSERVE completion (soft) — expected once the args-schema gap lands
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
        require_fire: true, // GR28: gemma3 + llama.cpp both FIRE notion/search (union format)
        require_answer: true, // GR28: OBSERVE completion (soft) — expected once the args-schema gap lands
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
        source_digest: Vec::new(),
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
    // GR28: the gemma3 connector-tool-fire proof — a tool-necessitating prompt FIRES the
    // granted tool on BOTH engines (the union format + priming).
    if case.require_fire {
        assert!(
            tool_fired && fired_tool_ids.iter().any(|id| id == case.granted_tool),
            "the {} App must FIRE {} on the live model [{engine}] (union format) — \
             tool_fired={tool_fired} fired={fired_tool_ids:?}",
            case.server_name,
            case.granted_tool
        );
    }
    // T-GEMMA3-TOOL-LOOP-ANSWER-FORCE: loop-completeness is OBSERVED here, not GATED. The
    // answer-force is proven deterministically (unit + coordinator full-seam) and by the
    // `answer_only_format_forces_gemma3_to_settle_live` witness; a hard "always answers" gate
    // would be INVALID — gemma3 has failure modes BEYOND the duplicate-loop this PR fixes
    // (notably MISSING required tool args → the observation fails → an HONEST dead-letter at
    // turn 0, correct per GR15: never fabricate an answer the model couldn't produce; tracked
    // as `T-GEMMA3-OLLAMA-TOOL-ARG-SCHEMA`). So we log completion, never assert it.
    if case.require_answer && !answered {
        eprintln!(
            "[note] {} did NOT complete the loop on [{engine}] (dead_lettered={dead_lettered}) — \
             expected once T-GEMMA3-OLLAMA-TOOL-ARG-SCHEMA (missing-required-args) lands; the \
             answer-force (duplicate/nudge) is proven separately",
            case.server_name
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
        require_fire: false, // observe-only (validated pre-union in #290)
        require_answer: false,
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
        require_fire: false, // observe-only (the hard-fire proof rides the RunApp slack witness)
        require_answer: false, // completeness proof rides the RunApp slack witness
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
        require_fire: false, // observe-only (the hard-fire proof rides the RunApp notion witness)
        require_answer: false, // completeness proof rides the RunApp notion witness
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
    grounded_app_envelope_ex(dataset, rule_ref, prompt, &[])
}

/// A 32-byte content ref as the 64-hex an envelope carries.
fn hex32(r: &[u8]) -> String {
    r.iter().map(|b| format!("{b:02x}")).collect()
}

/// [`grounded_app_envelope`] with a CARRIED corpus — `cas_refs` naming the content-store
/// blobs the declared dataset spans (`T-RUNAPP-RAG-SELF-CONTAINED`). Empty ⇒ the
/// reference-existing App.
fn grounded_app_envelope_ex(
    dataset: &str,
    rule_ref: &[u8],
    prompt: &str,
    cas_refs: &[String],
) -> Vec<u8> {
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
        cas_refs: cas_refs.to_vec(),
    });
    env.references.rules.push(kx_app::ArtifactRef {
        name: "brief".to_string(),
        content_ref: hex32(rule_ref),
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
        source_digest: Vec::new(),
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

/// LIVE witness for `T-RUNAPP-RAG-SELF-CONTAINED` — the SELF-CONTAINED sibling of
/// [`runapp_grounded_app_self_grounds_live`]. Same App, one difference that is the whole
/// feature: **nothing is pre-ingested**. The corpus travels inside the envelope as
/// `references.datasets[].cas_refs`, and the App materializes it on first run.
///
/// The test calls `IngestDocuments` **zero times** — `datasets.db` starts EMPTY, exactly
/// as it would on a machine that just imported someone else's bundle. Without the carried
/// corpus this App fails closed at RunApp ("no such dataset is ingested").
///
/// The load-bearing assertion is the DETERMINISTIC one: after the run, a `science.app-*`
/// dataset exists that nobody ingested, and querying it returns NON-EMPTY passages. That
/// is "it retrieves + grounds" without betting on model behavior — and it is the assertion
/// that matters, because `retrieve@1` fails SOFT: a model that miscopied the scoped name
/// would answer confidently UNGROUNDED, and an answer-only assertion would pass anyway.
/// The agentic leg keeps the sibling's robust invariants (settles; only retrieve fires).
#[tokio::test(flavor = "multi_thread")]
#[ignore = "real LLM inference + dataset embedding; needs a served Gemma + --features inference,hnsw; opt in with --ignored"]
async fn runapp_self_contained_app_ingests_its_carried_corpus_live() {
    let Some(engine) = resolve_engine() else {
        eprintln!("skipping: no serve model — set KX_SERVE_MODEL_GGUF or KX_SERVE_OLLAMA=on");
        return;
    };
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    // A serve with no retrieval seam/embedder cannot self-ingest — skip rather than
    // report a false negative. Probe with a throwaway dataset that the App never names.
    if c.ingest_documents(proto::IngestDocumentsRequest {
        dataset: "ingest-probe".to_string(),
        documents: vec![doc(b"probe")],
    })
    .await
    .is_err()
    {
        eprintln!("skipping: ingest unavailable (needs --features hnsw + an embedder)");
        running.shutdown().await.unwrap();
        return;
    }

    // The corpus travels as CONTENT — the same paraphrase set the sibling ingests by
    // hand. The target doc never says "photosynthesis"; only a real retrieve surfaces it.
    let mut cas_refs = Vec::new();
    for body in [
        &b"Plants turn sunlight, water, and carbon dioxide into sugar and oxygen inside their leaves."[..],
        &b"The mitochondria is the powerhouse of the cell, producing ATP from glucose."[..],
        &b"Tectonic plates drift over the mantle, causing earthquakes at their boundaries."[..],
    ] {
        let put = c
            .put_content(proto::PutContentRequest {
                payload: body.to_vec(),
                media_type: "text/plain".to_string(),
                filename: String::new(),
            })
            .await
            .expect("PutContent a corpus doc")
            .into_inner();
        cas_refs.push(hex32(&put.content_ref));
    }
    let rule = c
        .put_content(proto::PutContentRequest {
            payload: b"Answer in ONE sentence, grounded in the retrieved passages.".to_vec(),
            media_type: "text/plain".to_string(),
            filename: String::new(),
        })
        .await
        .expect("PutContent")
        .into_inner();

    // NOTE: `science` is NEVER ingested — the App carries it.
    let before: Vec<String> = c
        .list_datasets(proto::ListDatasetsRequest {})
        .await
        .unwrap()
        .into_inner()
        .datasets
        .into_iter()
        .map(|d| d.name)
        .collect();
    assert!(
        !before.iter().any(|n| n.starts_with("science")),
        "no source dataset exists before the run: {before:?}"
    );

    let handle_str = "apps/local/self-contained-analyst".to_string();
    c.save_app(proto::SaveAppRequest {
        handle: handle_str.clone(),
        envelope_json: grounded_app_envelope_ex(
            "science",
            &rule.content_ref,
            "How do plants make energy from the sun? Use the dataset to answer.",
            &cas_refs,
        ),
        source_digest: Vec::new(),
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
            // Without the carried-corpus path this is exactly where a shared App died.
            panic!("RunApp of a self-contained App must not fail closed: {e}");
        }
    };
    assert_eq!(handle.instance_id.len(), 16, "RunApp returns a run handle");

    // THE PROOF (deterministic): the App materialized its OWN corpus, under the scoped
    // name, with zero IngestDocuments for it — and that index really retrieves.
    let scoped: Vec<String> = c
        .list_datasets(proto::ListDatasetsRequest {})
        .await
        .unwrap()
        .into_inner()
        .datasets
        .into_iter()
        .map(|d| d.name)
        .filter(|n| n.starts_with("science.app-"))
        .collect();
    assert_eq!(
        scoped.len(),
        1,
        "the carried corpus self-ingested under exactly one scoped name: {scoped:?}"
    );
    let hits = c
        .query_dataset(proto::QueryDatasetRequest {
            dataset: scoped[0].clone(),
            query_text: "how do plants make energy from sunlight".to_string(),
            query_embedding: Vec::new(),
            k: 3,
            ..Default::default()
        })
        .await
        .expect("QueryDataset the self-ingested corpus")
        .into_inner();
    assert!(
        !hits.hits.is_empty(),
        "the self-ingested corpus RETRIEVES — non-empty passages [{engine}]"
    );
    let top = String::from_utf8_lossy(&hits.hits[0].content).to_string();
    assert!(
        top.contains("Plants turn sunlight"),
        "the carried doc is what grounds the answer, got: {top}"
    );

    // The agentic leg — the sibling's robust invariants (model behavior is probabilistic).
    let (mut answered, mut dead, mut retrieved) = (false, false, false);
    let mut fired: Vec<String> = Vec::new();
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
            if t.branch == "tool" && !t.tool_id.is_empty() && !fired.contains(&t.tool_id) {
                fired.push(t.tool_id.clone());
            }
            if t.branch == "tool" && t.tool_id == "retrieve" {
                retrieved = true;
            }
        }
        answered = turns.turns.iter().any(|t| t.branch == "answer");
        dead = turns.turns.iter().any(|t| t.branch == "dead_lettered");
        if answered || dead {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    eprintln!(
        "LIVE self-contained RunApp [{engine}]: scoped={} retrieved={retrieved} \
         fired={fired:?} answered={answered} dead_lettered={dead}",
        scoped[0]
    );
    assert!(
        answered || dead,
        "the self-contained App settles to a terminal on the live model [{engine}]"
    );
    assert!(
        fired.iter().all(|id| id.is_empty() || id == "retrieve"),
        "only retrieve fires from the dataset rail; fired={fired:?}"
    );
    // The readable-first naming bet: can the model copy the scoped name back into
    // `retrieve`? Logged, not asserted (tool proposal is probabilistic) — but a run that
    // NEVER retrieves across engines is the signal to move to a declared→physical alias.
    if retrieved {
        eprintln!("✓ self-contained RAG: the model retrieved on the SCOPED name it was steered at");
    } else {
        eprintln!(
            "· note: the model answered without firing `retrieve` (check the scoped-name copy)"
        );
    }

    running.shutdown().await.unwrap();
}

/// The swarm request `kx.swarm(...)` / `flow().swarm(...)` lowers to: N parallel MODEL
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

/// LIVE swarm witness (GR15/GR24): a 2-agent swarm → gather runs end-to-end on a live
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

/// C4 / Rule-41 (D209.3, SN-8): NL authoring end-to-end on a live model. A natural-language
/// GOAL is turned into a PROPOSED multi-step DAG by `ProposeWorkflow` (the served model plans;
/// the gateway decodes + compiles the plan through the vetted `kx-planner` path — the model
/// names only role + intent + edges, every capability axis comes from the vetted role catalog).
/// Then the CONFIRMED DAG (authored from the proposal, exactly the console apply→submit path)
/// runs and settles on the live model. Fresh serve per test (the App/fire dedup — L-097). The
/// proposal is model-nondeterministic, so a `Rejected` outcome is LOGGED + the run leg skipped;
/// a `Plan` MUST be a real multi-step DAG that then commits. Per-step tools are the C3 surface,
/// so the proposed steps are pure model roles here (real multi-agent reasoning, not a tool fire).
#[tokio::test(flavor = "multi_thread")]
#[ignore = "real in-process LLM inference; needs a GGUF (Gemma-4 locally); opt in with --ignored"]
async fn propose_workflow_authors_a_multistep_dag_and_runs_live() {
    let Some(engine) = resolve_engine() else {
        eprintln!("skipping: no serve model — set KX_SERVE_MODEL_GGUF or KX_SERVE_OLLAMA=on");
        return;
    };
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    // ---- NL → a proposed multi-step DAG (propose-then-confirm, VALIDATE-ONLY) ----
    let goal = "Research the top 3 durable-execution engines and write a short comparison.";
    let resp = c
        .propose_workflow(proto::ProposeWorkflowRequest {
            goal: goal.to_string(),
        })
        .await
        .expect("ProposeWorkflow")
        .into_inner();
    let plan = match resp.result {
        Some(proto::propose_workflow_response::Result::Plan(p)) => p,
        Some(proto::propose_workflow_response::Result::Rejected(r)) => {
            // Honest (D142): the model on this build did not return an admissible plan. The
            // RPC + decode + compile path was still exercised; skip the run leg.
            eprintln!("LIVE propose [{engine}]: rejected — {}", r.reason);
            running.shutdown().await.unwrap();
            return;
        }
        None => panic!("ProposeWorkflow returned neither a plan nor a rejection"),
    };
    eprintln!(
        "LIVE propose [{engine}]: {} steps, {} edges — roles {:?}",
        plan.steps.len(),
        plan.edges.len(),
        plan.steps
            .iter()
            .map(|s| s.role.as_str())
            .collect::<Vec<_>>()
    );
    assert!(
        plan.steps.len() >= 2,
        "the model proposed a MULTI-step plan on the live model [{engine}] (got {})",
        plan.steps.len()
    );
    // Every proposed step resolved a vetted role (SN-8 — the model cannot invent one).
    for s in &plan.steps {
        assert!(!s.role.is_empty(), "each proposed step names a vetted role");
    }

    // ---- confirm: author the proposed DAG (the console apply→submit path) + run it ----
    // Prepend a brevity directive to each step's prompt: the proposal's INTENTS can be
    // verbose, and 5 sequential 12B generations would blow the poll window — a test-speed
    // knob only (the DAG shape + roles are the model's proposal). Empty model_id ⇒ the
    // served default binds (the swarm witness precedent).
    let steps: Vec<proto::WorkflowStep> = plan
        .steps
        .iter()
        .map(|s| proto::WorkflowStep {
            kind: proto::WorkflowStepKind::Model as i32,
            model_id: String::new(),
            prompt: format!("Reply in one or two short sentences.\n\n{}", s.intent),
            body_signature_id: Vec::new(),
            tool_contract: HashMap::new(), // pure model roles (per-step tools = C3)
            params: HashMap::new(),
        })
        .collect();
    let n = steps.len();
    let edges: Vec<proto::WorkflowEdge> = plan
        .edges
        .iter()
        .map(|e| orch_edge(e.parent, e.child))
        .collect();
    let handle = c
        .submit_workflow(proto::SubmitWorkflowRequest {
            seed: 0,
            steps,
            edges,
            execution_mode: proto::WorkflowExecutionMode::Frozen as i32,
            context_bundles: vec![],
        })
        .await
        .expect("SubmitWorkflow of the proposed DAG")
        .into_inner();
    assert_eq!(handle.instance_id.len(), 16);

    // Poll the projection: pure model steps commit directly, so the DAG settles when every
    // step's mote commits (the swarm-witness observation pattern).
    let mut committed = 0usize;
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
            .count();
        if committed >= n {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    eprintln!("LIVE propose→run [{engine}]: {committed}/{n} steps committed");
    assert!(
        committed >= n,
        "the NL-proposed DAG's {n} steps all committed on the live model [{engine}] (got {committed})"
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

/// LIVE pool witness (GR15/GR24): the SAME wide swarm run under a single worker
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

// -- orchestration LIVE witnesses (supervisor / consensus) --------------------
//
// These prove the ORCHESTRATION layer end-to-end on a live model (both engines): a
// hierarchical supervisor, a best-of-N judge, and an exact-equality majority vote all
// run to a committed terminal producing REAL output (GR15). Each lowers to the SAME
// fan-out/fan-in DAG the SDK `supervisor()`/`consensus()` methods and the `kx swarm`
// verb author (raw proto built here); the deterministic shapes are pinned by the SDK +
// UI unit tests and the golden corpus. Pure composition of existing step kinds — no new
// wire shape (the majority sink's server-side reduce lives in `model_exec::
// run_consensus_majority`, gated on `config_subset[kx.consensus.vote] == "majority"`).

/// One MODEL leaf/sink step routed to the served model (empty `model_id`).
fn orch_model(prompt: &str) -> proto::WorkflowStep {
    proto::WorkflowStep {
        kind: proto::WorkflowStepKind::Model as i32,
        model_id: String::new(),
        prompt: prompt.to_string(),
        body_signature_id: Vec::new(),
        tool_contract: Default::default(),
        params: Default::default(),
    }
}

/// A Data edge `parent -> child`.
fn orch_edge(parent: u32, child: u32) -> proto::WorkflowEdge {
    proto::WorkflowEdge {
        parent,
        child,
        edge_kind: proto::EdgeKind::Data as i32,
        non_cascade: false,
    }
}

/// `supervisor()` lowers to `planner(0) > [worker(1) & worker(2)] > gather(3)`: the
/// planner's committed plan is a Data-parent of every worker, and every worker feeds the
/// gather (the mote with two parents).
fn supervisor_request() -> proto::SubmitWorkflowRequest {
    proto::SubmitWorkflowRequest {
        seed: 0,
        steps: vec![
            orch_model(
                "You are the supervisor. Split the task into two one-sentence subtasks: \
                 (A) one concrete BENEFIT and (B) one concrete RISK of durable agentic \
                 execution. State subtask A and subtask B, each in one sentence.",
            ),
            orch_model(
                "Do subtask A from the plan above: in one sentence, give a concrete BENEFIT \
                 of durable agentic execution.",
            ),
            orch_model(
                "Do subtask B from the plan above: in one sentence, give a concrete RISK of \
                 durable agentic execution.",
            ),
            orch_model("Integrate the workers' results above into one two-sentence summary."),
        ],
        edges: vec![
            orch_edge(0, 1),
            orch_edge(0, 2),
            orch_edge(1, 3),
            orch_edge(2, 3),
        ],
        execution_mode: proto::WorkflowExecutionMode::Frozen as i32,
        context_bundles: vec![],
    }
}

/// `consensus(vote="judge")` lowers to `[voter(0) & voter(1)] > judge(2)`: a MODEL judge
/// that SELECTS the single best candidate (distinct from a swarm's merge).
fn consensus_judge_request() -> proto::SubmitWorkflowRequest {
    proto::SubmitWorkflowRequest {
        seed: 0,
        steps: vec![
            orch_model(
                "In one sentence, argue that durable agentic execution is worth its complexity.",
            ),
            orch_model(
                "In one sentence, argue that durable agentic execution is NOT worth its complexity.",
            ),
            orch_model(
                "You are the judge. Read the two candidate answers above and reply with the \
                 single best one VERBATIM, without merging or editing them.",
            ),
        ],
        edges: vec![orch_edge(0, 2), orch_edge(1, 2)],
        execution_mode: proto::WorkflowExecutionMode::Frozen as i32,
        context_bundles: vec![],
    }
}

/// `consensus(vote="majority")` lowers to `[voter(0) & voter(1) & voter(2)] > sink(3)`,
/// where the sink is a PURE step carrying `config_subset[kx.consensus.vote] = "majority"`;
/// the server reduces the voters to the exact-equality plurality winner (SN-8). The three
/// voters are given the SAME constrained prompt so a real majority is likely.
fn consensus_majority_request() -> proto::SubmitWorkflowRequest {
    let voter = || {
        orch_model(
            "Reply with exactly one lowercase word and NOTHING else — either 'yes' or 'no': \
             is a durable, append-only journal a sound basis for exactly-once execution?",
        )
    };
    let sink = proto::WorkflowStep {
        kind: proto::WorkflowStepKind::Pure as i32,
        model_id: String::new(),
        prompt: String::new(),
        body_signature_id: Vec::new(),
        tool_contract: Default::default(),
        // The consensus-vote marker (raw bytes; `consensus_vote_from_config` decodes
        // JSON-quoted-or-raw). Free params fold verbatim into the sink's config_subset.
        params: std::iter::once(("kx.consensus.vote".to_string(), b"majority".to_vec())).collect(),
    };
    proto::SubmitWorkflowRequest {
        seed: 0,
        steps: vec![voter(), voter(), voter(), sink],
        edges: vec![orch_edge(0, 3), orch_edge(1, 3), orch_edge(2, 3)],
        execution_mode: proto::WorkflowExecutionMode::Frozen as i32,
        context_bundles: vec![],
    }
}

/// Drive a fan-in orchestration to completion on the live engine and return the RAW
/// committed payloads: the terminal sink's bytes + every leaf's bytes (the 0-parent
/// voters/agents). Polls until all `total` motes commit and the terminal (the mote with
/// `terminal_parents` Data-parents) commits; panics on timeout (the non-flaky settle
/// invariant). The gateway is torn down before returning.
async fn run_fanin(
    engine: &str,
    req: proto::SubmitWorkflowRequest,
    total: usize,
    terminal_parents: usize,
) -> (Vec<u8>, Vec<Vec<u8>>) {
    const COMMITTED: i32 = proto::MoteSnapshotState::Committed as i32;
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;
    let iid = c
        .submit_workflow(req)
        .await
        .expect("SubmitWorkflow of the fan-in orchestration")
        .into_inner()
        .instance_id;
    assert_eq!(iid.len(), 16);

    let mut settled = false;
    let mut committed = 0usize;
    for i in 0..2400 {
        let view = c
            .get_projection(proto::GetProjectionRequest {
                instance_id: iid.clone(),
                at_seq: None,
            })
            .await
            .unwrap()
            .into_inner();
        committed = view.motes.iter().filter(|m| m.state == COMMITTED).count();
        let terminal_ok = view.motes.iter().any(|m| {
            m.parents.len() == terminal_parents && m.state == COMMITTED && m.result_ref.is_some()
        });
        if committed >= total && terminal_ok {
            settled = true;
            break;
        }
        // Diagnostic (every ~10s): per-mote `p{parents}:s{state}` so a stuck DAG is
        // visible (which mote never leaves Pending / dead-letters).
        if i % 40 == 0 {
            let states = view
                .motes
                .iter()
                .map(|m| format!("p{}:s{}", m.parents.len(), m.state))
                .collect::<Vec<_>>()
                .join(",");
            eprintln!("[{engine}] poll {i}: committed={committed}/{total} motes=[{states}]");
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert!(
        settled,
        "the {terminal_parents}-parent fan-in settled with all {total} motes committed on \
         [{engine}] (got committed={committed}/{total})"
    );

    let view = c
        .get_projection(proto::GetProjectionRequest {
            instance_id: iid.clone(),
            at_seq: None,
        })
        .await
        .unwrap()
        .into_inner();
    let terminal_ref: [u8; 32] = view
        .motes
        .iter()
        .find(|m| m.parents.len() == terminal_parents && m.state == COMMITTED)
        .and_then(|m| m.result_ref.clone())
        .and_then(|r| r.try_into().ok())
        .expect("the terminal sink committed with a result ref");
    let leaf_refs: Vec<[u8; 32]> = view
        .motes
        .iter()
        .filter(|m| m.parents.is_empty() && m.state == COMMITTED)
        .filter_map(|m| m.result_ref.clone().and_then(|r| r.try_into().ok()))
        .collect();

    let fetch = |content_ref: [u8; 32]| proto::GetContentRequest {
        content_ref: content_ref.to_vec(),
        instance_id: iid.clone(),
    };
    let terminal = c
        .get_content(fetch(terminal_ref))
        .await
        .expect("GetContent of the terminal sink")
        .into_inner()
        .payload;
    let mut leaves = Vec::with_capacity(leaf_refs.len());
    for r in leaf_refs {
        leaves.push(
            c.get_content(fetch(r))
                .await
                .expect("GetContent of a leaf")
                .into_inner()
                .payload,
        );
    }

    running.shutdown().await.unwrap();
    (terminal, leaves)
}

/// LIVE supervisor witness (GR15/GR24): planner → 2 workers → gather runs end-to-end on a
/// live model; the gather (the 2-parent terminal) integrates a REAL non-empty answer.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "real in-process LLM inference; needs a GGUF (Gemma-4 locally) or KX_SERVE_OLLAMA=on"]
async fn supervisor_plans_delegates_and_integrates_live() {
    let Some(engine) = resolve_engine() else {
        eprintln!("skipping: no serve model — set KX_SERVE_MODEL_GGUF or KX_SERVE_OLLAMA=on");
        return;
    };
    let (integration, _leaves) = run_fanin(engine, supervisor_request(), 4, 2).await;
    let text = String::from_utf8_lossy(&integration);
    eprintln!("LIVE supervisor [{engine}] integration: {}", text.trim());
    assert!(
        !text.trim().is_empty(),
        "the supervisor integrated a non-empty answer on the live model [{engine}]"
    );
}

/// LIVE consensus(judge) witness (GR15/GR24): 2 voters → a model judge that SELECTS the
/// best; the judge (the 2-parent terminal) commits a REAL non-empty selection.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "real in-process LLM inference; needs a GGUF (Gemma-4 locally) or KX_SERVE_OLLAMA=on"]
async fn consensus_judge_selects_best_of_n_live() {
    let Some(engine) = resolve_engine() else {
        eprintln!("skipping: no serve model — set KX_SERVE_MODEL_GGUF or KX_SERVE_OLLAMA=on");
        return;
    };
    let (selected, _cands) = run_fanin(engine, consensus_judge_request(), 3, 2).await;
    let text = String::from_utf8_lossy(&selected);
    eprintln!("LIVE consensus(judge) [{engine}] selected: {}", text.trim());
    assert!(
        !text.trim().is_empty(),
        "the judge selected a non-empty answer on the live model [{engine}]"
    );
}

/// LIVE consensus(majority) witness (GR15/GR24/SN-8): 3 constrained voters → a PURE
/// server-reduced majority. Proves the EXACT-equality reduce: the committed winner is
/// byte-equal to one of the voter outputs VERBATIM (never a merged/similar answer).
#[tokio::test(flavor = "multi_thread")]
#[ignore = "real in-process LLM inference; needs a GGUF (Gemma-4 locally) or KX_SERVE_OLLAMA=on"]
async fn consensus_majority_reduces_to_a_voter_verbatim_live() {
    let Some(engine) = resolve_engine() else {
        eprintln!("skipping: no serve model — set KX_SERVE_MODEL_GGUF or KX_SERVE_OLLAMA=on");
        return;
    };
    let (winner, voters) = run_fanin(engine, consensus_majority_request(), 4, 3).await;
    let show = |b: &[u8]| String::from_utf8_lossy(b).trim().to_string();
    eprintln!(
        "LIVE consensus(majority) [{engine}] winner={:?} voters={:?}",
        show(&winner),
        voters.iter().map(|v| show(v)).collect::<Vec<_>>()
    );
    assert!(
        !String::from_utf8_lossy(&winner).trim().is_empty(),
        "the majority sink committed a non-empty winner on [{engine}]"
    );
    // SN-8: the reduce is EXACT byte-equality — the winner MUST equal a voter output VERBATIM.
    assert!(
        voters.contains(&winner),
        "the majority winner must be byte-equal to one of the voter outputs on [{engine}]"
    );
}
