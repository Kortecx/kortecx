//! A LIVE model fires a real third-party tool that only an environment map could configure.
//!
//! The deterministic sibling (`connector_env_map`) proves the plumbing: a two-entry map
//! reaches the child, and the tool returns a payload only the loopback stub could produce.
//! What it does not show is a MODEL choosing to call it. This does, and it **asserts** the
//! fire rather than observing it.
//!
//! ⚠ That distinction is why this file exists. The pre-existing connector witnesses say in
//! their own header that they "assert the fire (real-or-fail)", and they do not — they
//! assert boundedness, print `fired=… (observed)`, and pass whether or not anything fired.
//! The justification recorded beside them names `gemma3:12b`, a model the project no longer
//! runs. So there was no live assertion that a third-party tool had ever fired, in either
//! direction, and the header said otherwise.
//!
//! The instruction here names the full tool id and the exact argument, which is the same
//! steering the deterministic path uses. If a live run cannot make this assertion hold on
//! `gemma4:12b`, THAT is the finding and it gets published with its n — an observed-only
//! witness would just reproduce the gap this file was written to close.
//!
//! Gated on `serve-engine`, never `inference`: `inference` implies `serve-engine` and not
//! the reverse, so an `inference`-gated file compiles to an EMPTY harness under the release
//! feature set, which is exactly the set the live proofs build. Runs under `--ignored`, so
//! the suite digest is untouched.

#![cfg(all(feature = "serve-engine", feature = "mcp-gateway"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;

use common::gitlab_stub::GitLabStub;
use kx_gateway::{start, REACT_AUTO_RECIPE_HANDLE};
use kx_proto::proto;
use kx_proto::proto::kx_gateway_client::KxGatewayClient;
use tonic::transport::Channel;

const VAR_TOKEN: &str = "GITLAB_PERSONAL_ACCESS_TOKEN";
const VAR_API_URL: &str = "GITLAB_API_URL";
const TOKEN_REF: &str = "KX_W3_LIVE_TOKEN_REF";
const API_URL_REF: &str = "KX_W3_LIVE_API_URL_REF";
const GOOD_TOKEN: &str = "glpat-w3-live-0123456789";
const MARKER: &str = "kortecx/w3-live-env-map";

fn gitlab_server_bin() -> Option<PathBuf> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
        "../kx-extension-sdk/tests/fixtures/real-connector/node_modules/.bin/mcp-server-gitlab",
    );
    p.is_file().then_some(p)
}

fn serve_model() -> Option<PathBuf> {
    std::env::var_os("KX_SERVE_MODEL_GGUF").and_then(|p| {
        let p = PathBuf::from(p);
        p.is_file().then_some(p)
    })
}

async fn client(addr: SocketAddr) -> KxGatewayClient<Channel> {
    common::connect_client(addr).await
}

/// ★ THE LIVE PROOF. A model, given tools it did not know about until this connector was
/// configured, calls the third-party one — and the third party really talks to the stub.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "live model: needs gemma4:12b on Ollama (KX_SERVE_OLLAMA=1) or a GGUF, plus the pinned MCP server"]
async fn a_live_model_fires_a_third_party_tool_configured_by_an_env_map() {
    let bin = gitlab_server_bin().expect(
        "the pinned third-party MCP server is missing — run `just test-connector-real`. \
         Failing rather than skipping: a live proof that silently does not run is not a proof.",
    );

    // ONE ENGINE PER RUN, chosen explicitly. An unset `KX_SERVE_OLLAMA` means `auto`, which
    // silently picks whichever engine happens to be available and mislabels the result.
    let ollama = matches!(
        std::env::var("KX_SERVE_OLLAMA").as_deref(),
        Ok("1") | Ok("on")
    );
    if ollama {
        std::env::remove_var("KX_SERVE_MODEL_GGUF");
    } else {
        let gguf = serve_model().expect(
            "no engine selected: set KX_SERVE_OLLAMA=1, or KX_SERVE_MODEL_GGUF to a readable GGUF",
        );
        std::env::set_var("KX_SERVE_MODEL_GGUF", &gguf);
    }
    std::env::set_var("KX_SERVE_AUTOGRANT", "1");

    let stub = GitLabStub::start(GOOD_TOKEN, MARKER);
    std::env::set_var(TOKEN_REF, GOOD_TOKEN);
    std::env::set_var(API_URL_REF, stub.url());

    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    let reg = c
        .register_mcp_server_with_env(proto::RegisterMcpServerEnvRequest {
            base: Some(proto::RegisterMcpServerRequest {
                server_name: "gitlab".to_string(),
                transport: "stdio".to_string(),
                endpoint: bin.to_string_lossy().into_owned(),
                args: vec![],
                tls_required: false,
                credential_ref: String::new(),
                session_mode: "stateless".to_string(),
            }),
            env: vec![
                proto::McpEnvEntry {
                    name: VAR_TOKEN.to_string(),
                    credential_ref: TOKEN_REF.to_string(),
                },
                proto::McpEnvEntry {
                    name: VAR_API_URL.to_string(),
                    credential_ref: API_URL_REF.to_string(),
                },
            ],
        })
        .await
        .expect("register the third-party connector")
        .into_inner();
    assert_eq!(
        reg.health, "connected",
        "the connector dials with both variables"
    );
    assert!(reg.discovered > 0, "its tools are discovered");
    eprintln!(
        "live env-map witness — registered gitlab: {} tool(s)",
        reg.discovered
    );

    let recipes = c
        .list_recipes(proto::ListRecipesRequest {})
        .await
        .unwrap()
        .into_inner();
    assert!(
        recipes
            .recipes
            .iter()
            .any(|r| r.handle == REACT_AUTO_RECIPE_HANDLE),
        "the agent recipe is not provisioned — the bundled stdio tool is missing. Failing \
         rather than skipping, because a skipped live proof reads exactly like a passing one."
    );

    let instruction = "Use the tool named gitlab/search_repositories with \
                       {\"search\":\"kortecx\"} to search the repositories, then report \
                       the full path of the project you found.";
    let args_json = format!(
        r#"{{"instruction":{},"max_turns":6,"max_tool_calls":3}}"#,
        serde_json::to_string(instruction).unwrap()
    );
    let resp = c
        .invoke(proto::InvokeRequest {
            handle: REACT_AUTO_RECIPE_HANDLE.to_string(),
            args: args_json.into_bytes(),
            context_bundles: vec![],
            context_refs: vec![],
        })
        .await
        .expect("invoke the agent recipe")
        .into_inner();

    let mut fired_tool_ids: Vec<String> = Vec::new();
    let mut answered = false;
    let mut last = String::new();
    // ~180 s: a 12B model running a multi-turn tool loop on local hardware.
    for _ in 0..1800 {
        let turns = c
            .list_react_turns(proto::ListReactTurnsRequest {
                limit: None,
                instance_id: Some(resp.instance_id.clone()),
                step_salt: None,
            })
            .await
            .unwrap()
            .into_inner();
        let branches: Vec<&str> = turns.turns.iter().map(|t| t.branch.as_str()).collect();
        let snap = format!("{branches:?}");
        if snap != last {
            eprintln!("live env-map witness — trajectory: {snap}");
            last = snap;
        }
        fired_tool_ids = turns
            .turns
            .iter()
            .filter(|t| t.branch == "tool")
            .map(|t| t.tool_id.clone())
            .collect();
        answered = turns.turns.iter().any(|t| t.branch == "answer");
        let settled = answered
            || turns
                .turns
                .iter()
                .any(|t| t.branch == "dead_lettered" || t.branch == "bounded");
        // Stop once the chain has settled — whether or not a tool fired. Waiting for a fire
        // would turn a genuine "the model never called it" into a timeout, and a timeout is
        // a much less useful thing to read than a settled chain with an empty tool list.
        if settled {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    eprintln!(
        "live env-map witness — engine={} fired_tools={:?} answered={} stub_hits={}",
        if ollama { "ollama" } else { "llamacpp" },
        fired_tool_ids,
        answered,
        stub.hits()
    );

    // ★ HARD assertions. Not "observed" — if a live model on the standing model cannot do
    // this, the refutation is the finding and belongs in the record, not softened away.
    assert!(
        fired_tool_ids
            .iter()
            .any(|id| id == "gitlab/search_repositories"),
        "the live model fired the third-party tool. fired={fired_tool_ids:?}"
    );
    // And the third party really talked to the stub, with the token the map supplied —
    // the tool id alone would only prove the runtime dispatched something.
    assert!(
        stub.hits() > 0,
        "the third-party server made a real request to the stub, so BOTH variables were \
         live: the base URL routed it here and the token got past the 401"
    );

    running.shutdown().await.unwrap();
    std::env::remove_var("KX_SERVE_AUTOGRANT");
}
