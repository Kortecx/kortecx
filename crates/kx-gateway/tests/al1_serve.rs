//! The GR15 real-model behavioral gate (`real-model-e2e`): `kx serve --features
//! inference` runs a REAL in-process model dispatch through the embedded worker.
//!
//! `Invoke` the server-provisioned `kx/recipes/chat` model recipe by handle+args
//! → the embedded worker leases the bound model Mote → the `ModelRouterExecutor`
//! ChatML-wraps the prompt, runs greedy inference through the in-process
//! `LlamaInferenceBackend`, publishes the completion into the shared store →
//! the coordinator commits it → the client awaits its `terminal_mote_id` and
//! fetches the completion via `GetContent`. The assertions are ROBUST (GR15): the
//! completion is non-empty valid `UTF-8`, CLEAN (no `ChatML` scaffolding leak — the
//! §2.199 `parse_special` guard at the e2e level — and no `kx demo result`
//! placeholder), and greedy decode is DETERMINISTIC across two independent
//! gateways (same prompt ⇒ byte-identical committed `result_ref`).
//!
//! Gated `#[cfg(feature = "inference")]` (pulls the llama.cpp FFI) AND `#[ignore]`,
//! and it runtime-skips: it needs a real GGUF — fetch the public Qwen3 stand-in
//! with `just fetch-agent-model` (or set `KX_SERVE_MODEL_GGUF`), then opt in with
//! `cargo test -p kx-gateway --features inference -- --ignored` (or `just
//! real-model-e2e`).

#![cfg(feature = "inference")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use kx_gateway::{start, MODEL_RECIPE_HANDLE};
use kx_proto::proto;
use kx_proto::proto::kx_gateway_client::KxGatewayClient;
use tonic::transport::Channel;

/// The serve model GGUF: `KX_SERVE_MODEL_GGUF` if set, else the public stand-in
/// `just fetch-agent-model` downloads. `None` ⇒ the test runtime-skips.
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

/// Poll `GetProjection` until `mote_id` is `Committed`; return its `result_ref`.
async fn await_mote_committed(
    c: &mut KxGatewayClient<Channel>,
    instance_id: &[u8],
    mote_id: &[u8],
) -> [u8; 32] {
    // Real LLM inference is slow on a CPU-only CI runner (no GPU offload) — a
    // Qwen3-0.6B turn can run toward its full output-token budget. Poll generously
    // (1200 × 250ms = 300s); on Metal locally this returns in well under a second.
    for _ in 0..1200 {
        let view = c
            .get_projection(proto::GetProjectionRequest {
                instance_id: instance_id.to_vec(),
                at_seq: None,
            })
            .await
            .unwrap()
            .into_inner();
        if let Some(m) = view
            .motes
            .iter()
            .find(|m| m.mote_id == mote_id && m.state == proto::MoteSnapshotState::Committed as i32)
        {
            return m
                .result_ref
                .clone()
                .expect("a committed Mote carries a result_ref")
                .try_into()
                .unwrap();
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    panic!("the invoked model Mote never reached Committed");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "real in-process LLM inference; needs a GGUF (just fetch-agent-model); opt in with --ignored"]
async fn invoke_model_recipe_runs_real_inference_to_committed() {
    let Some(gguf) = serve_model() else {
        eprintln!(
            "skipping: no serve model — run `just fetch-agent-model` (or set \
             KX_SERVE_MODEL_GGUF) first"
        );
        return;
    };
    // resolve_serve_model() reads this env at start().
    std::env::set_var("KX_SERVE_MODEL_GGUF", &gguf);

    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    // Invoke the model recipe with a prompt free-param.
    let resp = c
        .invoke(proto::InvokeRequest {
            handle: MODEL_RECIPE_HANDLE.to_string(),
            args: br#"{"prompt":"What is 2+2? Reply with just the number."}"#.to_vec(),
        })
        .await
        .expect("invoke kx/recipes/chat (is the serve model fit + the feature on?)")
        .into_inner();
    assert_eq!(resp.instance_id.len(), 16, "journaled instance_id is 16B");
    assert_eq!(
        resp.terminal_mote_id.len(),
        32,
        "server-derived terminal Mote"
    );

    // Await THIS invocation's model Mote, then fetch its committed completion.
    let result_ref = await_mote_committed(&mut c, &resp.instance_id, &resp.terminal_mote_id).await;
    let blob = c
        .get_content(proto::GetContentRequest {
            content_ref: result_ref.to_vec(),
            instance_id: resp.instance_id.clone(),
        })
        .await
        .unwrap()
        .into_inner();
    assert!(
        !blob.payload.is_empty(),
        "the model produced a non-empty completion"
    );
    // GR15 + the §2.199 fix at the e2e level: a REAL model completion is CLEAN —
    // valid UTF-8, no ChatML scaffolding leak (the `parse_special` stop-token fix
    // ⇒ the model stops at `<|im_end|>` instead of re-emitting the turn structure),
    // and never the retired `kx demo result` placeholder.
    let completion =
        String::from_utf8(blob.payload.clone()).expect("the completion is valid UTF-8");
    assert!(
        !completion.contains("<|im_start|>") && !completion.contains("<|im_end|>"),
        "no ChatML scaffolding leak in the completion: {completion:?}"
    );
    assert!(
        !completion.contains("kx demo result"),
        "no demo placeholder leak (the honest passthrough / model split): {completion:?}"
    );
    eprintln!(
        "AL1 completion ({} bytes): {completion}",
        blob.payload.len()
    );

    running.shutdown().await.unwrap();
}

/// GR15 real-model determinism gate: greedy decode is DETERMINISTIC end-to-end —
/// the SAME prompt served on two INDEPENDENT gateways (fresh journals/stores)
/// commits a BYTE-IDENTICAL `result_ref` (same content address). Complements the
/// `kx-llamacpp` unit-level determinism suite by proving it through the full serve
/// path (invoke → worker → inference → commit → content-address). Runtime-skips
/// without a GGUF.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "real in-process LLM inference; needs a GGUF (just fetch-agent-model); opt in with --ignored"]
async fn chat_greedy_decode_is_deterministic_across_gateways() {
    let Some(gguf) = serve_model() else {
        eprintln!(
            "skipping: no serve model — run `just fetch-agent-model` (or set \
             KX_SERVE_MODEL_GGUF) first"
        );
        return;
    };
    std::env::set_var("KX_SERVE_MODEL_GGUF", &gguf);
    let prompt = br#"{"prompt":"Name one primary color. Reply with a single word."}"#.to_vec();

    // Serve `kx/recipes/chat` on a FRESH gateway and return the committed result_ref.
    async fn run_chat_on_fresh_gateway(prompt: &[u8]) -> [u8; 32] {
        let dir = tempfile::TempDir::new().unwrap();
        let running = start(common::gateway_config(&dir, true, HashMap::new()))
            .await
            .unwrap();
        let mut c = client(running.local_addr()).await;
        let resp = c
            .invoke(proto::InvokeRequest {
                handle: MODEL_RECIPE_HANDLE.to_string(),
                args: prompt.to_vec(),
            })
            .await
            .expect("invoke kx/recipes/chat")
            .into_inner();
        let result_ref =
            await_mote_committed(&mut c, &resp.instance_id, &resp.terminal_mote_id).await;
        running.shutdown().await.unwrap();
        result_ref
    }

    let ref_a = run_chat_on_fresh_gateway(&prompt).await;
    let ref_b = run_chat_on_fresh_gateway(&prompt).await;
    assert_eq!(
        ref_a, ref_b,
        "greedy decode is deterministic ⇒ the same prompt commits a byte-identical \
         result across independent gateways"
    );
}
