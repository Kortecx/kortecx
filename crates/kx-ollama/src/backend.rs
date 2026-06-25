//! [`OllamaBackend`] — the [`InferenceBackend`] (+ [`EmbeddingBackend`]) impl over
//! [`OllamaClient`].
//!
//! It enforces the SAME warrant gates as the in-process llama backend, in the SAME
//! order (SN-8 / D35): the grammar reservation, the model-route authorization, the
//! `max_output_tokens` ceiling, then the served-set membership — all BEFORE any
//! HTTP egress. The inherent `warm`/`evict`/`resident` mirror the llama backend's
//! lifecycle surface so the host can drive both engines through one trait.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Instant;

use kx_inference::{
    EmbeddingBackend, EmbeddingOutput, EmbeddingPooling, InferenceBackend, InferenceError,
    InferenceInput, InferenceOutput, InferenceParams, TokenSink,
};
use kx_mote::ModelId;
use kx_warrant::WarrantSpec;

use crate::client::OllamaClient;
use crate::error::OllamaError;

/// The backend identity echoed in [`InferenceOutput::backend_name`] (audit-only,
/// never journaled).
pub const BACKEND_NAME: &str = "kx-ollama";

/// An [`InferenceBackend`] that serves a fixed, discovered set of Ollama model tags
/// over HTTP.
#[derive(Debug)]
pub struct OllamaBackend {
    client: Arc<OllamaClient>,
    /// The tags this backend serves (the `/api/tags` set, optionally narrowed by an
    /// operator allowlist). Membership is the `supports()` gate.
    models: BTreeSet<String>,
    /// Per-tag declared context window from `/api/show` (populated best-effort at
    /// discovery; `0` when the daemon doesn't report one). Display/discovery only
    /// (SN-8) — it never authorizes a route, and it is never journaled.
    context_len: BTreeMap<String, u32>,
}

impl OllamaBackend {
    /// Construct a backend that serves exactly `models` through `client`. Context
    /// windows are left empty (reported `0`); [`Self::discover`] is the path that
    /// populates them from `/api/show`.
    #[must_use]
    pub fn new(client: Arc<OllamaClient>, models: BTreeSet<String>) -> Self {
        Self {
            client,
            models,
            context_len: BTreeMap::new(),
        }
    }

    /// Discover the served set from the daemon's `/api/tags`, optionally narrowed to
    /// an operator allowlist (an empty / absent allowlist serves every installed tag).
    /// Also fetches each served tag's declared context window via `/api/show`
    /// (best-effort: a failed/absent window honest-degrades to `0` and never blocks
    /// serving).
    ///
    /// # Errors
    /// Propagates the [`OllamaClient::tags`] transport / status / protocol failure.
    pub fn discover(
        client: Arc<OllamaClient>,
        allowlist: Option<&[String]>,
    ) -> Result<Self, OllamaError> {
        let tags = client.tags()?;
        let models: BTreeSet<String> = match allowlist {
            Some(allow) if !allow.is_empty() => {
                let allow: BTreeSet<&str> = allow.iter().map(String::as_str).collect();
                tags.into_iter()
                    .filter(|t| allow.contains(t.as_str()))
                    .collect()
            }
            _ => tags.into_iter().collect(),
        };
        // One `/api/show` per served tag, best-effort. `unwrap_or(0)` so a daemon
        // that errors / omits the window never blocks startup (honest-degrade).
        let context_len = models
            .iter()
            .map(|tag| (tag.clone(), client.show_context_length(tag).unwrap_or(0)))
            .collect();
        Ok(Self {
            client,
            models,
            context_len,
        })
    }

    /// The model ids this backend serves (for the host's model catalog).
    #[must_use]
    pub fn model_ids(&self) -> Vec<ModelId> {
        self.models.iter().cloned().map(ModelId).collect()
    }

    /// The declared context window for `model_id` (fetched from `/api/show` at
    /// discovery), or `0` when unknown. Display/discovery only (SN-8) — for parity
    /// with the llama backend's GGUF `n_ctx`, surfaced in `kx models list`.
    #[must_use]
    pub fn context_len(&self, model_id: &ModelId) -> u32 {
        self.context_len.get(&model_id.0).copied().unwrap_or(0)
    }

    /// Warm `model_id` into the daemon's memory (`keep_alive = -1`). Fail-closed:
    /// [`InferenceError::ModelNotFound`] for a tag this backend does not serve (a
    /// lifecycle control can never warm an arbitrary model).
    ///
    /// # Errors
    /// [`InferenceError::ModelNotFound`] for an unserved id, or
    /// [`InferenceError::BackendFailure`] on a daemon error.
    pub fn warm(&self, model_id: &ModelId) -> Result<(), InferenceError> {
        self.ensure_served(model_id)?;
        self.client
            .set_keep_alive(&model_id.0, -1)
            .map_err(backend_failure)
    }

    /// Evict `model_id` from the daemon's memory (`keep_alive = 0`). Returns whether
    /// the model was resident before the call. Fail-closed like [`Self::warm`].
    ///
    /// # Errors
    /// [`InferenceError::ModelNotFound`] for an unserved id, or
    /// [`InferenceError::BackendFailure`] on a daemon error.
    pub fn evict(&self, model_id: &ModelId) -> Result<bool, InferenceError> {
        self.ensure_served(model_id)?;
        let was_resident = self.resident().iter().any(|m| m == model_id);
        self.client
            .set_keep_alive(&model_id.0, 0)
            .map_err(backend_failure)?;
        Ok(was_resident)
    }

    /// The served model ids currently resident in the daemon (`/api/ps`). An empty
    /// vec on a transient `ps` failure (honest, never a fabricated residency).
    #[must_use]
    pub fn resident(&self) -> Vec<ModelId> {
        match self.client.ps() {
            Ok(names) => names
                .into_iter()
                .filter(|n| self.models.contains(n))
                .map(ModelId)
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    /// Fail-closed served-set gate.
    fn ensure_served(&self, model_id: &ModelId) -> Result<(), InferenceError> {
        if self.models.contains(&model_id.0) {
            Ok(())
        } else {
            Err(InferenceError::ModelNotFound {
                model_id: model_id.0.clone(),
            })
        }
    }

    /// Shared body for [`InferenceBackend::dispatch`] (`sink` `None`) and
    /// [`InferenceBackend::dispatch_streaming`] (`Some`) — every warrant gate is
    /// byte-identical, so the committed bytes are unchanged whether or not a sink
    /// is passed.
    fn dispatch_inner(
        &self,
        model_id: &ModelId,
        input: &InferenceInput,
        params: &InferenceParams,
        warrant: &WarrantSpec,
        sink: Option<TokenSink>,
    ) -> Result<InferenceOutput, InferenceError> {
        // ---- Grammar reservation gate (fires before any model work) ----------
        if params.grammar.is_some() {
            return Err(InferenceError::Unsupported {
                reason: "constrained generation (grammar) reserved; see HANDOFF",
            });
        }
        // ---- Warrant gates (D30 + D35) — authorize BEFORE any egress ---------
        if model_id != &warrant.model_route.model_id {
            return Err(InferenceError::WarrantDeniesModel {
                model_id: model_id.0.clone(),
                route: warrant.model_route.model_id.0.clone(),
            });
        }
        if params.max_output_tokens > warrant.model_route.max_output_tokens {
            return Err(InferenceError::ScopeViolation {
                field: "max_output_tokens",
                requested: u64::from(params.max_output_tokens),
                ceiling: u64::from(warrant.model_route.max_output_tokens),
            });
        }
        self.ensure_served(model_id)?;

        // ---- Dispatch by input modality --------------------------------------
        let prompt = match input {
            InferenceInput::Text(prompt) => prompt.clone(),
            InferenceInput::Multimodal { .. } => {
                return Err(InferenceError::Unsupported {
                    reason: "multimodal input not yet supported by the Ollama backend (PR-B)",
                });
            }
            InferenceInput::TextForEmbedding { .. } => {
                return Err(InferenceError::Unsupported {
                    reason: "embedding input on the completion path; use \
                             EmbeddingBackend::dispatch_embedding",
                });
            }
        };

        let options = options_from_params(params);
        let started = Instant::now();
        let outcome = self
            .client
            .generate(
                &model_id.0,
                &prompt,
                &options,
                warrant.resource_ceiling.wall_clock_ms,
                sink,
            )
            .map_err(map_dispatch_err)?;
        Ok(InferenceOutput {
            bytes: outcome.text.into_bytes(),
            output_tokens: outcome.eval_count,
            backend_name: BACKEND_NAME,
            model_id: model_id.clone(),
            elapsed: started.elapsed(),
        })
    }
}

impl InferenceBackend for OllamaBackend {
    fn dispatch(
        &self,
        model_id: &ModelId,
        input: &InferenceInput,
        params: &InferenceParams,
        warrant: &WarrantSpec,
    ) -> Result<InferenceOutput, InferenceError> {
        self.dispatch_inner(model_id, input, params, warrant, None)
    }

    fn dispatch_streaming(
        &self,
        model_id: &ModelId,
        input: &InferenceInput,
        params: &InferenceParams,
        warrant: &WarrantSpec,
        token_sink: Option<TokenSink>,
    ) -> Result<InferenceOutput, InferenceError> {
        self.dispatch_inner(model_id, input, params, warrant, token_sink)
    }

    // `render_chat` keeps the trait default (`None`): the serve path renders the
    // chat prompt itself and the backend dispatches the rendered string verbatim
    // (`/api/generate` `raw:true`), exactly as the llama backend consumes its
    // already-rendered prompt — so no second template pass is applied.

    fn supports(&self, model_id: &ModelId) -> bool {
        self.models.contains(&model_id.0)
    }

    fn name(&self) -> &'static str {
        BACKEND_NAME
    }
}

impl EmbeddingBackend for OllamaBackend {
    /// Embed `text` for `model_id`, authorizing the model route BEFORE any egress
    /// (SN-8 / D35). `pooling` is ignored — the daemon applies the model's own
    /// pooling — but the seam is exercised so wiring it for the RAG path (PR-B) is
    /// additive.
    ///
    /// # Errors
    /// [`InferenceError::WarrantDeniesModel`] when the route does not authorize the
    /// model, [`InferenceError::ModelNotFound`] for an unserved id, or
    /// [`InferenceError::BackendFailure`] on a daemon error.
    fn dispatch_embedding(
        &self,
        model_id: &ModelId,
        text: &str,
        pooling: EmbeddingPooling,
        warrant: &WarrantSpec,
    ) -> Result<EmbeddingOutput, InferenceError> {
        let _ = pooling;
        if model_id != &warrant.model_route.model_id {
            return Err(InferenceError::WarrantDeniesModel {
                model_id: model_id.0.clone(),
                route: warrant.model_route.model_id.0.clone(),
            });
        }
        self.ensure_served(model_id)?;
        let started = Instant::now();
        let vector = self
            .client
            .embed(&model_id.0, text)
            .map_err(backend_failure)?;
        let dim = u32::try_from(vector.len()).unwrap_or(u32::MAX);
        Ok(EmbeddingOutput {
            vector,
            dim,
            backend_name: BACKEND_NAME,
            model_id: model_id.clone(),
            elapsed: started.elapsed(),
        })
    }
}

/// Map [`InferenceParams`] to the Ollama `options` object. Temperature / top-p are
/// stored in basis points (×10 000); top-k / top-p apply only when sampling.
fn options_from_params(params: &InferenceParams) -> serde_json::Value {
    let mut opts = serde_json::Map::new();
    opts.insert("num_predict".to_string(), params.max_output_tokens.into());
    opts.insert("seed".to_string(), params.seed.into());
    let temperature = f64::from(params.temperature_bps) / 10_000.0;
    opts.insert(
        "temperature".to_string(),
        serde_json::Number::from_f64(temperature)
            .map_or(serde_json::Value::Null, serde_json::Value::Number),
    );
    if params.temperature_bps > 0 {
        let top_p = f64::from(params.top_p_bps) / 10_000.0;
        opts.insert(
            "top_p".to_string(),
            serde_json::Number::from_f64(top_p)
                .map_or(serde_json::Value::Null, serde_json::Value::Number),
        );
        if params.top_k > 0 {
            opts.insert("top_k".to_string(), params.top_k.into());
        }
    }
    // The serve path renders a chat prompt (ChatML fallback, or a model's own
    // template) and dispatches it RAW (`/api/generate raw:true`), so — unlike the
    // in-process llama backend, which gets stops from the GGUF chat template — these
    // common assistant-turn-end markers must be passed explicitly, or generation
    // runs past the turn boundary (e.g. emitting a trailing `<|im_end|>`). Merged
    // with any recipe-declared stops, deduped. Harmless for non-chat prompts (these
    // are control tokens models do not emit as normal content).
    let mut stop: Vec<String> = DEFAULT_STOPS.iter().map(|s| (*s).to_string()).collect();
    for s in &params.stop_tokens {
        if !stop.contains(s) {
            stop.push(s.clone());
        }
    }
    opts.insert("stop".to_string(), serde_json::json!(stop));
    serde_json::Value::Object(opts)
}

/// Common assistant-turn-end markers across the chat templates the serve path
/// renders (`ChatML`, Gemma, `Llama-3`), passed as raw-mode `stop` tokens so a turn
/// ends cleanly even when the recipe declares no stops.
const DEFAULT_STOPS: &[&str] = &["<|im_end|>", "<end_of_turn>", "<|eot_id|>"];

/// Map a generate failure: a daemon timeout becomes [`InferenceError::Timeout`]
/// (the warrant's wall-clock honored), every other class a
/// [`InferenceError::BackendFailure`].
fn map_dispatch_err(err: OllamaError) -> InferenceError {
    match err {
        OllamaError::Timeout { wall_clock_ms } => InferenceError::Timeout { wall_clock_ms },
        other => backend_failure(other),
    }
}

/// Map an [`OllamaError`] to [`InferenceError::BackendFailure`]. By value so it is
/// usable directly as a `.map_err(backend_failure)` adapter.
#[allow(clippy::needless_pass_by_value)]
fn backend_failure(err: OllamaError) -> InferenceError {
    InferenceError::BackendFailure {
        backend: BACKEND_NAME,
        message: err.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_always_carry_the_turn_end_stops() {
        let opts = options_from_params(&InferenceParams::default());
        let stop = opts
            .get("stop")
            .and_then(|v| v.as_array())
            .expect("stop array");
        let stops: Vec<&str> = stop.iter().filter_map(|v| v.as_str()).collect();
        for marker in DEFAULT_STOPS {
            assert!(
                stops.contains(marker),
                "default stop {marker} missing: {stops:?}"
            );
        }
        // num_predict + seed are always present (mapped from the params).
        assert!(opts.get("num_predict").is_some());
        assert!(opts.get("seed").is_some());
    }

    #[test]
    fn recipe_stops_merge_without_duplicates() {
        let mut params = InferenceParams::default();
        params.stop_tokens.push("<|im_end|>".to_string()); // already a default
        params.stop_tokens.push("DONE".to_string());
        let opts = options_from_params(&params);
        let stops: Vec<String> = opts["stop"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        assert_eq!(stops.iter().filter(|s| *s == "<|im_end|>").count(), 1);
        assert!(stops.contains(&"DONE".to_string()));
    }
}
