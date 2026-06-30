//! Attach-mode embedding / RAG spikes (Golden Rule 10 + GR24 dual-engine parity):
//! time a real datasets **server-embed** round-trip against a LIVE `kx serve` over
//! gRPC, engine-agnostically. Mirrors [`crate::chat_spikes`] — a pure FFI-free gRPC
//! client over the frozen `KxGatewayClient` stubs (no `kx-gateway`
//! `serve-engine`/`inference` feature, no new dependency).
//!
//! It calls `ListModels` ONCE to find the CONFIGURED embed model (the `can_embed`
//! entry — `KX_SERVE_EMBED_MODEL` else the primary) and its engine (the PR-A `engine`
//! field), so the captured JSON self-labels the run. Then per iteration it times
//! `IngestDocuments` (one UNIQUE text doc, server-embedded) and `QueryDataset` (a text
//! query, server-embedded). Each input is unique so the runtime memoizer never
//! short-circuits an already-committed embed — we measure the engine's embed latency,
//! not a cache hit.

use std::time::Instant;

use tonic::transport::Channel;

use kx_proto::proto;
use kx_proto::proto::kx_gateway_client::KxGatewayClient;

use crate::error::ProfileError;

/// The default ingest/query text (a short, deterministic sentence) when none is given.
pub const DEFAULT_TEXT: &str =
    "The kortecx runtime is a durable, local-first agentic engine for SOTA agents.";

/// Options for an attach-mode embedding run.
#[derive(Debug, Clone)]
pub struct EmbedOpts {
    /// Number of TIMED iterations (after one discarded cold warm-up ingest).
    pub iterations: usize,
    /// The base ingest/query text (a per-iteration tag is appended for uniqueness).
    pub text: String,
    /// Optional explicit embed model id; `None` ⇒ the `can_embed` model.
    pub model: Option<String>,
    /// Optional bearer token (a `--dev-allow-local` serve needs none).
    pub token: Option<String>,
}

/// Raw per-iteration embedding latency samples (milliseconds) + the self-describing
/// labels read from `ListModels`.
#[derive(Debug, Clone)]
pub struct EmbedSamples {
    /// The serving engine that embeds (`"kx-ollama"` / `"kx-llamacpp"`), used as the
    /// metric-id prefix so an Ollama capture and a llama.cpp capture never collide.
    pub engine: String,
    /// The configured embed model id.
    pub model_id: String,
    /// Per-iteration `IngestDocuments` latency (one text doc, server-embedded), warm.
    pub ingest_ms: Vec<f64>,
    /// Per-iteration `QueryDataset` latency (a text query, server-embedded), warm.
    pub query_ms: Vec<f64>,
}

/// The dataset name the harness ingests into (a fresh corpus per profile process; the
/// store is off-journal and discarded with the serve's catalog dir).
const PROFILE_DATASET: &str = "kx-profile-embed";

/// Measure attach-mode embedding latency over `opts.iterations` against a connected
/// `channel` (an external `kx serve`). Dial with [`crate::chat_spikes::connect`].
///
/// # Errors
/// [`ProfileError::Client`] if `ListModels` exposes no embed model, or an ingest /
/// query RPC fails.
pub async fn measure(channel: &Channel, opts: &EmbedOpts) -> Result<EmbedSamples, ProfileError> {
    let mut c = client(channel);

    // Learn which model embeds + on which engine (self-labels the capture).
    let models = c
        .list_models(authed(proto::ListModelsRequest {}, opts.token.as_deref())?)
        .await
        .map_err(|s| ProfileError::Client(s.to_string()))?
        .into_inner()
        .models;
    let model = match &opts.model {
        Some(id) => models.iter().find(|m| &m.model_id == id),
        None => models.iter().find(|m| m.can_embed),
    }
    .ok_or_else(|| {
        ProfileError::Client(
            "no embed model on the attached serve (set KX_SERVE_EMBED_MODEL to an \
             embedding-capable model and launch `kx serve --features hnsw,serve-engine` \
             [+ a running Ollama daemon] or --features hnsw,inference [+ an embedding GGUF])"
                .to_string(),
        )
    })?;
    let engine = if model.engine.is_empty() {
        "unknown".to_string()
    } else {
        model.engine.clone()
    };
    let model_id = model.model_id.clone();

    // One discarded warm-up (cold embed-model load) before the timed loop.
    let _ = ingest_one(
        channel,
        &format!("{} (warm-up)", opts.text),
        opts.token.as_deref(),
    )
    .await?;

    let mut ingest_ms = Vec::with_capacity(opts.iterations);
    let mut query_ms = Vec::with_capacity(opts.iterations);
    for i in 0..opts.iterations {
        let text = format!("{} (variant {i})", opts.text);
        ingest_ms.push(ingest_one(channel, &text, opts.token.as_deref()).await?);
        query_ms.push(query_one(channel, &text, opts.token.as_deref()).await?);
    }

    Ok(EmbedSamples {
        engine,
        model_id,
        ingest_ms,
        query_ms,
    })
}

/// Time one `IngestDocuments` of a single text doc (empty client vector ⇒ server-embed).
async fn ingest_one(
    channel: &Channel,
    text: &str,
    token: Option<&str>,
) -> Result<f64, ProfileError> {
    let mut c = client(channel);
    let t0 = Instant::now();
    c.ingest_documents(authed(
        proto::IngestDocumentsRequest {
            dataset: PROFILE_DATASET.to_string(),
            documents: vec![proto::IngestDocument {
                content: text.as_bytes().to_vec(),
                embedding: Vec::new(), // empty ⇒ server-embed
                ..Default::default()
            }],
        },
        token,
    )?)
    .await
    .map_err(|s| ProfileError::Client(s.to_string()))?;
    Ok(t0.elapsed().as_secs_f64() * 1000.0)
}

/// Time one `QueryDataset` text query (empty client vector ⇒ server-embed the query).
async fn query_one(
    channel: &Channel,
    text: &str,
    token: Option<&str>,
) -> Result<f64, ProfileError> {
    let mut c = client(channel);
    let t0 = Instant::now();
    c.query_dataset(authed(
        proto::QueryDatasetRequest {
            dataset: PROFILE_DATASET.to_string(),
            query_text: text.to_string(),
            query_embedding: Vec::new(), // empty ⇒ server-embed
            k: 5,
            // RC4a: profile the hybrid (BM25 + dense, RRF-fused) retrieval path.
            retrieval_mode: proto::RetrievalMode::Hybrid as i32,
            // RC4c: server default rerank (the profile measures the standard path).
            rerank: None,
        },
        token,
    )?)
    .await
    .map_err(|s| ProfileError::Client(s.to_string()))?;
    Ok(t0.elapsed().as_secs_f64() * 1000.0)
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

    /// The attach embed spike against an FFI-free in-process gateway (no embed model)
    /// fails CLEANLY with a "no embed model" client error — the client path is
    /// exercised deterministically without any engine. (The live both-engine numbers
    /// are captured separately and persisted to the private benchmarks.)
    #[tokio::test]
    async fn measure_errors_cleanly_when_no_embed_model() {
        let dir = tempfile::TempDir::new().unwrap();
        let running = start(spikes::config(dir.path()).unwrap()).await.unwrap();
        let channel = crate::chat_spikes::connect(&running.local_addr().to_string())
            .await
            .unwrap();
        let opts = EmbedOpts {
            iterations: 1,
            text: DEFAULT_TEXT.to_string(),
            model: None,
            token: None,
        };
        let err = measure(&channel, &opts).await.unwrap_err();
        match err {
            ProfileError::Client(msg) => assert!(
                msg.contains("no embed model"),
                "expected a no-embed-model error, got: {msg}"
            ),
            other => panic!("expected a Client error, got: {other:?}"),
        }
        running.shutdown().await.unwrap();
    }
}
