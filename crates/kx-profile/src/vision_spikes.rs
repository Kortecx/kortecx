//! Attach-mode vision spikes (Golden Rule 10 + GR24 dual-engine parity): time a real
//! image→text turn against a LIVE `kx serve` over gRPC, engine-agnostically. Mirrors
//! [`crate::chat_spikes`] — a pure FFI-free gRPC client over the frozen
//! `KxGatewayClient` stubs (no `kx-gateway` `serve-engine`/`inference` feature, no new
//! dependency).
//!
//! It calls `ListModels` ONCE to find a VISION model (an entry whose `modalities`
//! declare `"image"`) + its engine (the PR-A `engine` field), `GetRecipeForm` to learn
//! the vision recipe slots, and `PutContent`s a small fixture image ONCE. Then per
//! iteration it times `Invoke kx/recipes/vision` → poll `GetProjection` until the
//! terminal Mote commits (the honest "answer complete" signal). Each prompt is unique
//! so the runtime memoizer never short-circuits an already-committed turn — we measure
//! the engine's vision-generation latency, not a cache hit.

use std::fmt::Write as _;
use std::time::{Duration, Instant};

use tonic::transport::Channel;

use kx_proto::proto;
use kx_proto::proto::kx_gateway_client::KxGatewayClient;

use crate::error::ProfileError;

/// The vision recipe handle (an image→text turn over a vision-capable model).
const VISION_HANDLE: &str = "kx/recipes/vision";

/// A small committed fixture image (96×96 red square on white) — the same one the
/// gateway/llama.cpp vision tests use. Deterministic + recognizable.
const FIXTURE_PNG: &[u8] = include_bytes!("fixtures/red_square.png");

/// Poll cadence for the commit wait (mirrors `chat_spikes`: 25 ms reflects the engine,
/// not the poll quantization).
const COMMIT_POLL: Duration = Duration::from_millis(25);

/// Per-turn wall-clock ceiling (generous — a cold vision model's first turn includes
/// the projector load + image encode + decode).
const MAX_VISION_MS: u64 = 300_000;

/// The default vision prompt (a short, deterministic question about the image).
pub const DEFAULT_PROMPT: &str = "What color is the shape in this image? Answer in one word.";

/// Options for an attach-mode vision run.
#[derive(Debug, Clone)]
pub struct VisionOpts {
    /// Number of TIMED iterations (after one discarded cold warm-up turn).
    pub iterations: usize,
    /// The base vision prompt (a per-iteration tag is appended for uniqueness).
    pub prompt: String,
    /// Optional explicit vision model id; `None` ⇒ the first model declaring `"image"`.
    pub model: Option<String>,
    /// Optional bearer token (a `--dev-allow-local` serve needs none).
    pub token: Option<String>,
}

/// Raw per-iteration vision latency samples (milliseconds) + the self-describing
/// labels read from `ListModels`.
#[derive(Debug, Clone)]
pub struct VisionSamples {
    /// The serving engine that answered (`"kx-ollama"` / `"kx-llamacpp"`), the metric-id
    /// prefix so an Ollama capture and a llama.cpp capture never collide.
    pub engine: String,
    /// The vision model id that answered.
    pub model_id: String,
    /// The cold FIRST turn's total latency (projector/model load included), recorded
    /// separately so it never skews the warm p50/p99.
    pub warmup_first_ms: f64,
    /// Per-iteration total latency (Invoke → terminal Mote Committed), warm.
    pub total_ms: Vec<f64>,
}

/// Measure attach-mode vision latency over `opts.iterations` against a connected
/// `channel` (an external `kx serve`). Dial with [`crate::chat_spikes::connect`].
///
/// # Errors
/// [`ProfileError::Client`] if `ListModels` exposes no vision model, the vision recipe
/// is not provisioned, or an `Invoke` fails / exceeds the per-turn ceiling.
pub async fn measure(channel: &Channel, opts: &VisionOpts) -> Result<VisionSamples, ProfileError> {
    let mut c = client(channel);

    // Learn which model is vision-capable + on which engine (self-labels the capture).
    let models = c
        .list_models(authed(proto::ListModelsRequest {}, opts.token.as_deref())?)
        .await
        .map_err(|s| ProfileError::Client(s.to_string()))?
        .into_inner()
        .models;
    let model = match &opts.model {
        Some(id) => models.iter().find(|m| &m.model_id == id),
        None => models
            .iter()
            .find(|m| m.modalities.iter().any(|x| x == "image")),
    }
    .ok_or_else(|| {
        ProfileError::Client(
            "no vision model on the attached serve (serve a vision model: an Ollama \
             vision tag e.g. `ollama pull gemma3`, or a llama.cpp model + \
             KX_SERVE_MMPROJ_GGUF)"
                .to_string(),
        )
    })?;
    let engine = if model.engine.is_empty() {
        "unknown".to_string()
    } else {
        model.engine.clone()
    };
    let model_id = model.model_id.clone();

    // Resolve the vision recipe form (the slots the SDK/CLI bind) + pick a legal model.
    let form = c
        .get_recipe_form(authed(
            proto::GetRecipeFormRequest {
                handle: VISION_HANDLE.to_string(),
            },
            opts.token.as_deref(),
        )?)
        .await
        .map_err(|_| {
            ProfileError::Client("kx/recipes/vision is not provisioned on this serve".to_string())
        })?
        .into_inner();
    let chosen_model = form
        .fields
        .iter()
        .find(|f| f.name == "model")
        .and_then(|f| f.allowed.first().cloned())
        .unwrap_or_else(|| model_id.clone());

    // Upload the fixture image ONCE (the ref is reused across iterations; only the
    // prompt varies, so each Mote's identity differs ⇒ no memoizer short-circuit).
    let put = c
        .put_content(authed(
            proto::PutContentRequest {
                payload: FIXTURE_PNG.to_vec(),
                media_type: "image/png".to_string(),
                filename: "red_square.png".to_string(),
            },
            opts.token.as_deref(),
        )?)
        .await
        .map_err(|s| ProfileError::Client(s.to_string()))?
        .into_inner();
    let mut image_ref = String::with_capacity(64);
    for b in &put.content_ref {
        let _ = write!(image_ref, "{b:02x}");
    }

    // One discarded warm-up (cold projector/model load) before the timed loop.
    let warmup_first_ms = one_vision(
        channel,
        &image_ref,
        &chosen_model,
        &format!("{} (warm-up)", opts.prompt),
        opts.token.as_deref(),
    )
    .await?;

    let mut total_ms = Vec::with_capacity(opts.iterations);
    for i in 0..opts.iterations {
        let prompt = format!("{} (variant {i})", opts.prompt);
        total_ms.push(
            one_vision(
                channel,
                &image_ref,
                &chosen_model,
                &prompt,
                opts.token.as_deref(),
            )
            .await?,
        );
    }

    Ok(VisionSamples {
        engine,
        model_id,
        warmup_first_ms,
        total_ms,
    })
}

/// Run one vision turn: `Invoke kx/recipes/vision` → poll until the terminal Mote
/// commits. Returns the total latency in ms.
async fn one_vision(
    channel: &Channel,
    image_ref: &str,
    model: &str,
    prompt: &str,
    token: Option<&str>,
) -> Result<f64, ProfileError> {
    let args = serde_json::to_vec(&serde_json::json!({
        "prompt": prompt,
        "image_ref": image_ref,
        "model": model,
    }))
    .map_err(|e| ProfileError::Client(e.to_string()))?;
    let mut c = client(channel);
    let t0 = Instant::now();
    let inv = c
        .invoke(authed(
            proto::InvokeRequest {
                handle: VISION_HANDLE.to_string(),
                args,
                context_bundles: Vec::new(),
                context_refs: Vec::new(),
            },
            token,
        )?)
        .await
        .map_err(|s| ProfileError::Client(s.to_string()))?
        .into_inner();
    wait_committed_ms(&mut c, &inv.instance_id, &inv.terminal_mote_id, token, t0).await
}

/// Poll `GetProjection` until `terminal_mote_id` is `Committed`; return elapsed-since-
/// `t0`. A non-committed terminal or a budget overrun invalidates the sample.
async fn wait_committed_ms(
    client: &mut KxGatewayClient<Channel>,
    instance_id: &[u8],
    terminal_mote_id: &[u8],
    token: Option<&str>,
    t0: Instant,
) -> Result<f64, ProfileError> {
    let committed = proto::MoteSnapshotState::Committed as i32;
    let pending = proto::MoteSnapshotState::Pending as i32;
    let scheduled = proto::MoteSnapshotState::Scheduled as i32;
    let budget = Duration::from_millis(MAX_VISION_MS);
    loop {
        let view = client
            .get_projection(authed(
                proto::GetProjectionRequest {
                    instance_id: instance_id.to_vec(),
                    at_seq: None,
                },
                token,
            )?)
            .await
            .map_err(|s| ProfileError::Client(s.to_string()))?
            .into_inner();
        if let Some(state) = view
            .motes
            .iter()
            .find(|m| m.mote_id == terminal_mote_id)
            .map(|m| m.state)
        {
            if state == committed {
                return Ok(t0.elapsed().as_secs_f64() * 1000.0);
            }
            if state != pending && state != scheduled {
                return Err(ProfileError::Client(format!(
                    "vision terminal Mote reached non-committed state {state} (failed run)"
                )));
            }
        }
        if t0.elapsed() >= budget {
            return Err(ProfileError::Timeout {
                what: "the vision terminal Mote to commit".to_string(),
                elapsed_ms: u64::try_from(budget.as_millis()).unwrap_or(u64::MAX),
            });
        }
        tokio::time::sleep(COMMIT_POLL).await;
    }
}

/// A gateway client over a cloned channel with a generous decode limit.
fn client(channel: &Channel) -> KxGatewayClient<Channel> {
    KxGatewayClient::new(channel.clone()).max_decoding_message_size(64 * 1024 * 1024)
}

/// Wrap a request with the `authorization: Bearer <token>` metadata when configured.
fn authed<T>(msg: T, token: Option<&str>) -> Result<tonic::Request<T>, ProfileError> {
    let mut req = tonic::Request::new(msg);
    if let Some(token) = token {
        let value = tonic::metadata::MetadataValue::try_from(format!("Bearer {token}"))
            .map_err(|e| ProfileError::Client(format!("bad auth token: {e}")))?;
        req.metadata_mut().insert("authorization", value);
    }
    Ok(req)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spikes;
    use kx_gateway::start;

    /// The attach vision spike against an FFI-free in-process gateway (no vision model)
    /// fails CLEANLY with a "no vision model" client error — the client path is
    /// exercised deterministically without any engine. (The live both-engine numbers
    /// are captured separately and persisted to the private benchmarks.)
    #[tokio::test]
    async fn measure_errors_cleanly_when_no_vision_model() {
        let dir = tempfile::TempDir::new().unwrap();
        let running = start(spikes::config(dir.path()).unwrap()).await.unwrap();
        let channel = crate::chat_spikes::connect(&running.local_addr().to_string())
            .await
            .unwrap();
        let opts = VisionOpts {
            iterations: 1,
            prompt: DEFAULT_PROMPT.to_string(),
            model: None,
            token: None,
        };
        let err = measure(&channel, &opts).await.unwrap_err();
        match err {
            ProfileError::Client(msg) => assert!(
                msg.contains("no vision model"),
                "expected a no-vision-model error, got: {msg}"
            ),
            other => panic!("expected a Client error, got: {other:?}"),
        }
        running.shutdown().await.unwrap();
    }
}
