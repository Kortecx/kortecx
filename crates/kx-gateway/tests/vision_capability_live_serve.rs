//! PR-B2 LIVE witness (`--ignored`): image→text vision on a served model, the
//! DUAL-ENGINE parity proof (GR24). Drives whichever engine the serve provisioned a
//! vision recipe for — restart between engines and run once each:
//!
//! ```sh
//! # llama.cpp (mmproj):
//! KX_SERVE_MODEL_GGUF=.../gemma-4-12b-it-q4_k_m.gguf \
//! KX_SERVE_MMPROJ_GGUF=.../gemma-4-mmproj.gguf \
//!   cargo test -p kx-gateway --features inference --test vision_capability_live_serve \
//!     -- --ignored --nocapture
//!
//! # Ollama (vision tag): `ollama pull gemma3` first, then
//! KX_SERVE_OLLAMA=1 cargo test -p kx-gateway --features inference \
//!   --test vision_capability_live_serve -- --ignored --nocapture
//! ```
//!
//! The test uploads (`PutContent`) a committed 96×96 red-square PNG, binds
//! `kx/recipes/vision` exactly as the SDK/CLI do (form-gated `{prompt, image_ref,
//! model}`), and asserts a
//! NON-EMPTY committed answer (the non-flaky invariant). Whether the model says "red"
//! is a SOFT signal (model quality is not what this gates — GR15 log-don't-assert), as
//! is the OCR shape (the same dispatch with a text image + a "transcribe" prompt).
//! Honest-skips when the serve provisioned NO vision model (no image-capable model is
//! served) — that path is covered deterministically by the `kx-ollama` mock gate tests.

#![cfg(feature = "inference")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Duration;

use kx_gateway::start;
use kx_proto::proto;
use kx_proto::proto::kx_gateway_client::KxGatewayClient;
use tonic::transport::Channel;

/// The committed 96×96 PNG (a red square on white) reused from the llama.cpp VLM smoke
/// — small, deterministic, recognizable.
const RED_SQUARE_PNG: &[u8] = include_bytes!("fixtures/red_square.png");

const VISION_HANDLE: &str = "kx/recipes/vision";

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

/// Poll `GetProjection` until `mote` commits; return its decoded UTF-8 answer.
async fn await_answer(
    c: &mut KxGatewayClient<Channel>,
    instance_id: Vec<u8>,
    mote: Vec<u8>,
) -> String {
    for _ in 0..600 {
        let proj = c
            .get_projection(proto::GetProjectionRequest {
                instance_id: instance_id.clone(),
                at_seq: Some(0),
            })
            .await
            .unwrap()
            .into_inner();
        if let Some(m) = proj
            .motes
            .iter()
            .find(|m| m.mote_id == mote && m.state == proto::MoteSnapshotState::Committed as i32)
        {
            let result_ref = m
                .result_ref
                .clone()
                .expect("committed mote carries a result_ref");
            let blob = c
                .get_content(proto::GetContentRequest {
                    content_ref: result_ref,
                    instance_id: instance_id.clone(),
                })
                .await
                .unwrap()
                .into_inner();
            return String::from_utf8_lossy(&blob.payload).into_owned();
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the vision mote never reached Committed");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "live serve + a vision model; see the module header for the dual-engine drive"]
async fn vision_image_to_text_on_the_served_engine() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    // Which engine + model is serving (for the operator's restart-between-engines log).
    let models = c
        .list_models(proto::ListModelsRequest {})
        .await
        .unwrap()
        .into_inner();
    let vision_model = models
        .models
        .iter()
        .find(|m| m.modalities.iter().any(|x| x == "image"));
    let Some(vm) = vision_model else {
        eprintln!("(skip) no image-capable model is served — provision a vision model and re-run");
        return;
    };
    eprintln!(
        "vision model: {} [{}] modalities={:?}",
        vm.model_id, vm.engine, vm.modalities
    );

    // The vision recipe must be provisioned (the form-gate the SDK/CLI use).
    let form = match c
        .get_recipe_form(proto::GetRecipeFormRequest {
            handle: VISION_HANDLE.to_string(),
        })
        .await
    {
        Ok(r) => r.into_inner(),
        Err(_) => {
            eprintln!("(skip) kx/recipes/vision is not provisioned on this serve");
            return;
        }
    };
    let has = |n: &str| form.fields.iter().find(|f| f.name == n);
    assert!(
        has("image_ref").is_some(),
        "the vision form declares image_ref"
    );

    // Upload the test image and bind {prompt, image_ref, model} (the SDK/CLI shape).
    let put = c
        .put_content(proto::PutContentRequest {
            payload: RED_SQUARE_PNG.to_vec(),
            media_type: "image/png".to_string(),
            filename: "red_square.png".to_string(),
        })
        .await
        .unwrap()
        .into_inner();
    let image_ref: String = put.content_ref.iter().map(|b| format!("{b:02x}")).collect();

    let mut args = serde_json::Map::new();
    args.insert("image_ref".to_string(), serde_json::json!(image_ref));
    if has("prompt").is_some() {
        args.insert(
            "prompt".to_string(),
            serde_json::json!("What color is the shape in this image? Answer in one word."),
        );
    }
    if let Some(model) = has("model") {
        args.insert(
            "model".to_string(),
            serde_json::json!(model.allowed.first().cloned().unwrap_or_default()),
        );
    }
    let args_bytes = serde_json::to_vec(&serde_json::Value::Object(args)).unwrap();

    let resp = c
        .invoke(proto::InvokeRequest {
            handle: VISION_HANDLE.to_string(),
            args: args_bytes,
            context_bundles: Vec::new(),
            context_refs: Vec::new(),
        })
        .await
        .expect("invoke kx/recipes/vision")
        .into_inner();

    let answer = await_answer(&mut c, resp.instance_id, resp.terminal_mote_id).await;
    eprintln!("vision answer ({}): {answer:?}", vm.engine);

    // The non-flaky invariant: a real, non-empty answer (the image reached the model).
    assert!(
        !answer.trim().is_empty(),
        "vision produced a non-empty answer"
    );
    // Soft signal (model quality is not what this gates): a correct VLM says "red".
    if answer.to_lowercase().contains("red") {
        eprintln!("(model correctly identified the red square)");
    } else {
        eprintln!("(note: model did not say 'red'; the gate only requires a non-empty answer)");
    }
    eprintln!(
        "OCR is the SAME dispatch — re-run with a text image + a \"transcribe the text\" prompt."
    );
}
