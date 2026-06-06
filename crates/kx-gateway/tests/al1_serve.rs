//! AL1 e2e witness: `kx serve --features inference` runs a REAL in-process model
//! dispatch through the embedded worker.
//!
//! `Invoke` the server-provisioned `kx/recipes/chat` model recipe by handle+args
//! → the embedded worker leases the bound model Mote → the `ModelRouterExecutor`
//! ChatML-wraps the prompt, runs greedy inference through the in-process
//! `LlamaInferenceBackend`, publishes the completion into the shared store →
//! the coordinator commits it → the client awaits its `terminal_mote_id` and
//! fetches the non-empty completion via `GetContent`.
//!
//! Gated `#[cfg(feature = "inference")]` (pulls the llama.cpp FFI) AND `#[ignore]`,
//! and it runtime-skips: it needs a real GGUF — fetch the public Qwen3 stand-in
//! with `just fetch-agent-model` (or set `KX_SERVE_MODEL_GGUF`), then opt in with
//! `cargo test -p kx-gateway --features inference -- --ignored`.

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
    // CPU/Metal inference is slow; allow generously (200 × 100ms = 20s).
    for _ in 0..200 {
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
        tokio::time::sleep(Duration::from_millis(100)).await;
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
    eprintln!(
        "AL1 completion ({} bytes): {}",
        blob.payload.len(),
        String::from_utf8_lossy(&blob.payload)
    );

    running.shutdown().await.unwrap();
}
