//! Attach-mode chat spikes (Golden Rule 10 + GR24 dual-engine parity): time a real
//! chat turn against a LIVE `kx serve` over gRPC, engine-agnostically. Unlike the
//! other spikes (which host a fresh in-process echo gateway), this dials an EXTERNAL
//! serve the operator launched — whichever inference engine it runs (the FFI-free
//! Ollama backend OR the in-process llama.cpp backend). The harness binary itself
//! stays FFI-free: it is a pure gRPC client over the frozen `KxGatewayClient` stubs
//! (no `kx-gateway` `serve-engine`/`inference` feature, no new dependency).
//!
//! It calls `ListModels` ONCE to learn which model + engine answers (the PR-A
//! `engine` field), so the captured JSON self-labels the run; then per iteration it
//! times `Invoke` → poll `GetProjection` until the terminal Mote commits (the honest
//! "answer complete" signal), and best-effort records time-to-first-token from
//! `StreamModelTokens` (an empty/immediate stream — a non-streaming engine or an
//! unwired broker — yields NO ttft sample rather than a misleading `0`).

use std::time::{Duration, Instant};

use tonic::transport::Channel;

use kx_proto::proto;
use kx_proto::proto::kx_gateway_client::KxGatewayClient;

use crate::error::ProfileError;

/// Poll cadence for the commit wait. Tight on purpose: the CLI polls at 250 ms for
/// UX, which would quantize the measured total to ±250 ms — a measurement artifact,
/// not an engine property. 25 ms reflects the engine, not the poll.
const COMMIT_POLL: Duration = Duration::from_millis(25);

/// Per-chat wall-clock ceiling. Generous — a cold 12B model's first turn (load +
/// decode) can run for many seconds. A run that exceeds this invalidates the sample
/// (a hung generation's "latency" is not a measurement).
const MAX_CHAT_MS: u64 = 300_000;

/// The default prompt (a short, deterministic factual turn) when none is given.
pub const DEFAULT_PROMPT: &str = "What is the capital of France? Answer in one short sentence.";

/// Options for an attach-mode chat run.
#[derive(Debug, Clone)]
pub struct ChatOpts {
    /// Number of TIMED iterations (after one discarded cold warm-up chat).
    pub iterations: usize,
    /// The chat prompt sent each iteration.
    pub prompt: String,
    /// Optional explicit model id; `None` ⇒ the primary (`serving == true`) model.
    pub model: Option<String>,
    /// Optional bearer token (a `--dev-allow-local` serve needs none).
    pub token: Option<String>,
}

/// Raw per-iteration chat latency samples (milliseconds) + the self-describing
/// labels read from `ListModels`.
#[derive(Debug, Clone)]
pub struct ChatSamples {
    /// The serving engine that answered (`"kx-ollama"` / `"kx-llamacpp"`), used as
    /// the metric-id prefix so an Ollama capture and a llama.cpp capture never
    /// collide in the trend record.
    pub engine: String,
    /// The model id that answered.
    pub model_id: String,
    /// The model's declared context window (tokens), for the record.
    pub context_len: u32,
    /// The cold FIRST chat's total latency (model load included) — recorded
    /// separately so it never skews the warm p50/p99.
    pub warmup_first_ms: f64,
    /// Per-iteration total latency (Invoke → terminal Mote Committed), warm.
    pub total_ms: Vec<f64>,
    /// Per-iteration time-to-first-token (best-effort; may be EMPTY when the engine
    /// does not stream / the broker is unwired).
    pub ttft_ms: Vec<f64>,
}

/// Dial an external `kx serve` endpoint. Accepts a bare `host:port` (assumed
/// plaintext loopback) or a full `http://host:port` URL. TLS (`https://`) attach is
/// a follow-up — the focused dual-engine baseline runs against a loopback
/// `--dev-allow-local` serve.
///
/// # Errors
/// [`ProfileError::Client`] if the endpoint is malformed or unreachable.
pub async fn connect(endpoint: &str) -> Result<Channel, ProfileError> {
    let uri = if endpoint.starts_with("http://") || endpoint.starts_with("https://") {
        endpoint.to_string()
    } else {
        format!("http://{endpoint}")
    };
    let ep = Channel::from_shared(uri)
        .map_err(|e| ProfileError::Client(format!("bad --serve endpoint: {e}")))?;
    // Bounded connect-retry (25 ms → 400 ms): a just-started `kx serve` accepts TCP
    // before the HTTP/2 stack is ready, so the first dial can transport-error.
    let mut delay = Duration::from_millis(25);
    let mut last = String::new();
    for _ in 0..8 {
        match ep.connect().await {
            Ok(channel) => return Ok(channel),
            Err(e) => {
                last = e.to_string();
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(Duration::from_millis(400));
            }
        }
    }
    Err(ProfileError::Client(format!(
        "could not reach {endpoint}: {last}"
    )))
}

/// Measure attach-mode chat latency over `opts.iterations` against a connected
/// `channel` (an external `kx serve`).
///
/// # Errors
/// [`ProfileError::Client`] if `ListModels` returns no model, a chat fails, or a
/// chat exceeds the per-chat wall-clock ceiling.
pub async fn measure(channel: &Channel, opts: &ChatOpts) -> Result<ChatSamples, ProfileError> {
    let mut client = client(channel);

    // Learn which model + engine answers (self-labels the capture).
    let models = client
        .list_models(authed(proto::ListModelsRequest {}, opts.token.as_deref())?)
        .await
        .map_err(|s| ProfileError::Client(s.to_string()))?
        .into_inner()
        .models;
    let model = match &opts.model {
        Some(id) => models.iter().find(|m| &m.model_id == id),
        None => models.iter().find(|m| m.serving),
    }
    .ok_or_else(|| {
        ProfileError::Client(
            "no served model on the attached serve (an FFI-free build with no engine \
             answers ListModels empty; launch `kx serve` with --features serve-engine \
             [+ a running Ollama daemon] or --features inference [+ a GGUF])"
                .to_string(),
        )
    })?;
    let handle = if model.chat_handle.is_empty() {
        "kx/recipes/chat".to_string()
    } else {
        model.chat_handle.clone()
    };
    let engine = if model.engine.is_empty() {
        "unknown".to_string()
    } else {
        model.engine.clone()
    };
    let model_id = model.model_id.clone();
    let context_len = model.context_len;

    // Each iteration sends a UNIQUE prompt (the base + a per-iteration tag) so the
    // runtime memoizer never short-circuits a model Mote whose inputs already
    // committed — we want to measure the engine's GENERATION latency, not a cache
    // hit. (A fixed prompt would memoize after the first turn and report sub-ms
    // "latency" that is the projection fold, not inference.) The tag is a short
    // trailing clause, so the answer stays a one-liner across iterations.
    let warmup_prompt = format!("{} (warm-up)", opts.prompt);
    let (warmup_first_ms, _) =
        one_chat(channel, &handle, &warmup_prompt, opts.token.as_deref()).await?;

    let mut total_ms = Vec::with_capacity(opts.iterations);
    let mut ttft_ms = Vec::with_capacity(opts.iterations);
    for i in 0..opts.iterations {
        let prompt = format!("{} (variant {i})", opts.prompt);
        let (total, ttft) = one_chat(channel, &handle, &prompt, opts.token.as_deref()).await?;
        total_ms.push(total);
        if let Some(ttft) = ttft {
            ttft_ms.push(ttft);
        }
    }

    Ok(ChatSamples {
        engine,
        model_id,
        context_len,
        warmup_first_ms,
        total_ms,
        ttft_ms,
    })
}

/// Run one chat turn: `Invoke` → (best-effort first-token) → poll until the terminal
/// Mote commits. Returns `(total_ms, ttft_ms)`.
async fn one_chat(
    channel: &Channel,
    handle: &str,
    prompt: &str,
    token: Option<&str>,
) -> Result<(f64, Option<f64>), ProfileError> {
    let args = serde_json::to_vec(&serde_json::json!({ "prompt": prompt }))
        .map_err(|e| ProfileError::Client(e.to_string()))?;
    let mut client = client(channel);
    let t0 = Instant::now();
    let inv = client
        .invoke(authed(
            proto::InvokeRequest {
                handle: handle.to_string(),
                args,
                context_bundles: Vec::new(),
                context_refs: Vec::new(),
            },
            token,
        )?)
        .await
        .map_err(|s| ProfileError::Client(s.to_string()))?
        .into_inner();

    // TTFT (best-effort): the first non-empty streamed piece. Returns as soon as the
    // first token arrives, then we poll for commit — t0 is fixed, so total stays
    // honest (Invoke → commit).
    let budget = Duration::from_millis(MAX_CHAT_MS);
    let ttft = first_token_ms(
        channel,
        &inv.instance_id,
        &inv.terminal_mote_id,
        token,
        t0,
        budget,
    )
    .await;

    let total = wait_committed_ms(
        &mut client,
        &inv.instance_id,
        &inv.terminal_mote_id,
        token,
        t0,
        budget,
    )
    .await?;
    Ok((total, ttft))
}

/// Poll `GetProjection` until the run's `terminal_mote_id` is `Committed`; return the
/// elapsed-since-`t0` total. A `Failed`/anomaly terminal or a budget overrun
/// invalidates the sample (a failed generation's latency is not a measurement).
async fn wait_committed_ms(
    client: &mut KxGatewayClient<Channel>,
    instance_id: &[u8],
    terminal_mote_id: &[u8],
    token: Option<&str>,
    t0: Instant,
    budget: Duration,
) -> Result<f64, ProfileError> {
    let committed = proto::MoteSnapshotState::Committed as i32;
    let pending = proto::MoteSnapshotState::Pending as i32;
    let scheduled = proto::MoteSnapshotState::Scheduled as i32;
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
                    "chat terminal Mote reached non-committed state {state} (failed run)"
                )));
            }
        }
        if t0.elapsed() >= budget {
            return Err(ProfileError::Timeout {
                what: "the chat terminal Mote to commit".to_string(),
                elapsed_ms: u64::try_from(budget.as_millis()).unwrap_or(u64::MAX),
            });
        }
        tokio::time::sleep(COMMIT_POLL).await;
    }
}

/// Best-effort time-to-first-token: open `StreamModelTokens` and return the elapsed
/// at the first non-empty `text_piece`. `None` on ANY difficulty (no stream, the
/// broker unwired → an immediately-ending empty stream, error, or budget) — the
/// caller then records NO ttft sample rather than a misleading `0`.
async fn first_token_ms(
    channel: &Channel,
    instance_id: &[u8],
    mote_id: &[u8],
    token: Option<&str>,
    t0: Instant,
    budget: Duration,
) -> Option<f64> {
    let mut client = client(channel);
    let req = authed(
        proto::StreamModelTokensRequest {
            instance_id: instance_id.to_vec(),
            mote_id: mote_id.to_vec(),
            since_seq: 0,
        },
        token,
    )
    .ok()?;
    let mut stream = tokio::time::timeout(budget, client.stream_model_tokens(req))
        .await
        .ok()?
        .ok()?
        .into_inner();
    loop {
        match tokio::time::timeout(budget, stream.message()).await {
            Ok(Ok(Some(chunk))) => {
                if !chunk.text_piece.is_empty() {
                    return Some(t0.elapsed().as_secs_f64() * 1000.0);
                }
                if chunk.done {
                    return None;
                }
            }
            // End of stream / transport error / budget — no first-token signal.
            _ => return None,
        }
    }
}

/// A gateway client over a cloned channel with a generous decode limit (chat
/// answers + projection views can exceed the default 4 MiB cap).
fn client(channel: &Channel) -> KxGatewayClient<Channel> {
    KxGatewayClient::new(channel.clone()).max_decoding_message_size(64 * 1024 * 1024)
}

/// Wrap a request with the `authorization: Bearer <token>` metadata when a token is
/// configured (mirrors the CLI's per-request auth). A `--dev-allow-local` serve
/// needs none.
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

    /// The attach spike against an FFI-free in-process gateway (no served model)
    /// fails CLEANLY with a "no served model" client error — the client path is
    /// exercised deterministically without any engine. (The live both-engine
    /// numbers are captured separately and persisted to the private benchmarks.)
    #[tokio::test]
    async fn measure_errors_cleanly_when_no_model_is_served() {
        let dir = tempfile::TempDir::new().unwrap();
        let running = start(spikes::config(dir.path()).unwrap()).await.unwrap();
        let channel = connect(&running.local_addr().to_string()).await.unwrap();
        let opts = ChatOpts {
            iterations: 1,
            prompt: DEFAULT_PROMPT.to_string(),
            model: None,
            token: None,
        };
        let err = measure(&channel, &opts).await.unwrap_err();
        match err {
            ProfileError::Client(msg) => assert!(
                msg.contains("no served model"),
                "expected a no-served-model error, got: {msg}"
            ),
            other => panic!("expected a Client error, got: {other:?}"),
        }
        running.shutdown().await.unwrap();
    }
}
