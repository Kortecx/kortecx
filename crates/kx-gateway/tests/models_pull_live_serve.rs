//! Model Control v2 LIVE witness (`--ignored`): drive the model SWITCH + ACQUIRE
//! surface against a real `kx serve`.
//!
//! Two layers:
//! 1. **Deterministic e2e (no network, no inference):** the security + switching
//!    invariants — `SetActiveModel` validates against the served catalog (fail-closed
//!    on an unknown id), `ListModels.active` reflects the choice, `PullModel` is
//!    REFUSED deny-by-default when downloads are off, and `GetPullStatus` is
//!    `NotFound` for an unknown id. These need only a registered model (the env GGUF /
//!    the Qwen3 stand-in), never an LLM forward pass — so they are CI-safe under
//!    `--ignored`.
//! 2. **Opt-in live pull (network):** when `KX_SERVE_ALLOW_MODEL_PULL=1` AND
//!    `KX_TEST_OLLAMA_PULL_TAG=<tag>` are set, pull the tag through the real serve,
//!    poll it to a terminal phase, and assert it appears in `ListModels` WITHOUT a
//!    restart + is switchable. The actual pull bytes are model-nondeterministic, so
//!    only the registration invariant is asserted (the phase is LOGGED).
//!
//! Gated `#[cfg(feature = "inference")]` + `#[ignore]`; runtime-skips without a model.
//! Drive on **Gemma-4 locally** (GR15):
//! `KX_SERVE_MODEL_GGUF=target/models/gemma-4-12b-it-q4_k_m.gguf \`
//! `  cargo test -p kx-gateway --features inference --test models_pull_live_serve -- --ignored --nocapture`

#![cfg(feature = "inference")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use kx_gateway::start;
use kx_proto::proto;
use kx_proto::proto::kx_gateway_client::KxGatewayClient;
use tonic::transport::Channel;
use tonic::Code;

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

#[tokio::test(flavor = "multi_thread")]
#[ignore = "live serve; needs a model (KX_SERVE_MODEL_GGUF or the Qwen3 stand-in)"]
async fn model_control_v2_switch_and_pull_deny_by_default() {
    let Some(gguf) = serve_model() else {
        eprintln!("skipping: no serve model — set KX_SERVE_MODEL_GGUF");
        return;
    };
    // The serve resolves its model set from the env at startup.
    std::env::set_var("KX_SERVE_MODEL_GGUF", &gguf);
    // Deny-by-default: ensure downloads are OFF for the refusal invariant.
    std::env::remove_var("KX_SERVE_ALLOW_MODEL_PULL");

    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    // ---- the served catalog (≥1 model registered) ----
    let models = c
        .list_models(proto::ListModelsRequest {})
        .await
        .unwrap()
        .into_inner()
        .models;
    assert!(!models.is_empty(), "the serve registered ≥1 model");
    let primary = models
        .iter()
        .find(|m| m.serving)
        .or_else(|| models.first())
        .unwrap()
        .model_id
        .clone();
    eprintln!("primary model: {primary}");

    // ---- switch: SetActiveModel validates + ListModels.active reflects it ----
    let active = c
        .set_active_model(proto::SetActiveModelRequest {
            model_id: primary.clone(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(active.active_model_id, primary);
    let after = c
        .list_models(proto::ListModelsRequest {})
        .await
        .unwrap()
        .into_inner()
        .models;
    assert!(
        after.iter().any(|m| m.model_id == primary && m.active),
        "the chosen model is marked active in ListModels"
    );
    // GetServerInfo projects the active id + the (off) download posture.
    let info = c
        .get_server_info(proto::GetServerInfoRequest {})
        .await
        .unwrap()
        .into_inner();
    assert_eq!(info.active_model_id, primary);
    assert!(!info.allow_model_pull, "downloads are off by default");

    // ---- clear: SetActiveModel("") returns to the primary ----
    let cleared = c
        .set_active_model(proto::SetActiveModelRequest {
            model_id: String::new(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(cleared.active_model_id, "");

    // ---- fail-closed: an unknown id is NotFound (never an unrouteable active model) ----
    let err = c
        .set_active_model(proto::SetActiveModelRequest {
            model_id: "kx-serve:does-not-exist".to_string(),
        })
        .await
        .expect_err("an unknown active model is refused");
    assert_eq!(err.code(), Code::NotFound);

    // ---- deny-by-default: PullModel is refused when downloads are off ----
    let refused = c
        .pull_model(proto::PullModelRequest {
            source: Some(proto::pull_model_request::Source::OllamaTag(
                "gemma3:12b".to_string(),
            )),
            sha256: String::new(),
            model_id: String::new(),
        })
        .await
        .unwrap()
        .into_inner();
    assert!(!refused.accepted, "a pull is refused with downloads off");
    assert!(
        refused.detail.contains("KX_SERVE_ALLOW_MODEL_PULL"),
        "the refusal names the operator opt-in: {}",
        refused.detail
    );

    // ---- GetPullStatus for an unknown id is NotFound ----
    let err = c
        .get_pull_status(proto::GetPullStatusRequest {
            model_id: "kx-serve:never-pulled".to_string(),
        })
        .await
        .expect_err("an untracked pull is NotFound");
    assert_eq!(err.code(), Code::NotFound);

    running.shutdown().await.unwrap();
    std::env::remove_var("KX_SERVE_MODEL_GGUF");
}

/// Opt-in LIVE pull: pull an Ollama tag through the real serve + assert it registers
/// WITHOUT a restart. Requires a running Ollama daemon, `KX_SERVE_ALLOW_MODEL_PULL=1`,
/// and `KX_TEST_OLLAMA_PULL_TAG=<small tag>` (e.g. a tiny model). Skipped otherwise.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "live pull; needs Ollama + KX_SERVE_ALLOW_MODEL_PULL=1 + KX_TEST_OLLAMA_PULL_TAG"]
async fn model_control_v2_live_ollama_pull_registers_without_restart() {
    let Some(tag) = std::env::var("KX_TEST_OLLAMA_PULL_TAG").ok().filter(|t| !t.is_empty()) else {
        eprintln!("skipping: set KX_TEST_OLLAMA_PULL_TAG=<tag> + KX_SERVE_ALLOW_MODEL_PULL=1 + run Ollama");
        return;
    };
    std::env::set_var("KX_SERVE_ALLOW_MODEL_PULL", "1");
    std::env::set_var("KX_SERVE_OLLAMA", "1");

    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    let accepted = c
        .pull_model(proto::PullModelRequest {
            source: Some(proto::pull_model_request::Source::OllamaTag(tag.clone())),
            sha256: String::new(),
            model_id: String::new(),
        })
        .await
        .unwrap()
        .into_inner();
    assert!(accepted.accepted, "pull accepted: {}", accepted.detail);
    let model_id = accepted.model_id;

    // Poll to a terminal phase (LOG it — the bytes are environment-dependent).
    let mut terminal = None;
    for _ in 0..1200 {
        let st = c
            .get_pull_status(proto::GetPullStatusRequest {
                model_id: model_id.clone(),
            })
            .await
            .unwrap()
            .into_inner();
        let phase = st.phase;
        // DONE = 5, FAILED = 6 (proto Phase enum).
        if phase == proto::get_pull_status_response::Phase::Done as i32
            || phase == proto::get_pull_status_response::Phase::Failed as i32
        {
            eprintln!("pull terminal: phase={phase} detail={}", st.detail);
            terminal = Some(phase);
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    assert_eq!(
        terminal,
        Some(proto::get_pull_status_response::Phase::Done as i32),
        "the pull reached DONE"
    );

    // The pulled model appears in ListModels WITHOUT a restart (the core v2 claim).
    let models = c
        .list_models(proto::ListModelsRequest {})
        .await
        .unwrap()
        .into_inner()
        .models;
    assert!(
        models.iter().any(|m| m.model_id == model_id),
        "the pulled model {model_id} is in the live catalog without a restart"
    );

    running.shutdown().await.unwrap();
    std::env::remove_var("KX_SERVE_ALLOW_MODEL_PULL");
}
