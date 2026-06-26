//! [`OllamaBackend`] — the [`InferenceBackend`] (+ [`EmbeddingBackend`]) impl over
//! [`OllamaClient`].
//!
//! It enforces the SAME warrant gates as the in-process llama backend, in the SAME
//! order (SN-8 / D35): the grammar reservation, the model-route authorization, the
//! `max_output_tokens` ceiling, then the served-set membership — all BEFORE any
//! HTTP egress. The inherent `warm`/`evict`/`resident` mirror the llama backend's
//! lifecycle surface so the host can drive both engines through one trait.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, PoisonError, RwLock};
use std::time::Instant;

use base64::Engine as _;
use kx_content::{sniff_image_format, ContentRef};
use kx_inference::{
    ContentFetcher, EmbeddingBackend, EmbeddingOutput, EmbeddingPooling, InferenceBackend,
    InferenceError, InferenceInput, InferenceOutput, InferenceParams, TokenSink, MEDIA_MARKER,
};
use kx_mote::ModelId;
use kx_warrant::WarrantSpec;

use crate::client::OllamaClient;
use crate::error::OllamaError;

/// The backend identity echoed in [`InferenceOutput::backend_name`] (audit-only,
/// never journaled).
pub const BACKEND_NAME: &str = "kx-ollama";

/// An [`InferenceBackend`] that serves a discovered set of Ollama model tags over
/// HTTP. The served set is interior-mutable so a runtime `kx models pull` can add a
/// freshly-downloaded tag WITHOUT restarting `kx serve` (Model Control v2); the
/// `RwLock`s are read-mostly (a `supports()`/dispatch read vs. a rare pull write).
pub struct OllamaBackend {
    client: Arc<OllamaClient>,
    /// The tags this backend serves (the `/api/tags` set, optionally narrowed by an
    /// operator allowlist). Membership is the `supports()` gate. Grown at runtime by
    /// [`Self::register_tag`] after a pull.
    models: RwLock<BTreeSet<String>>,
    /// Per-tag declared context window from `/api/show` (populated best-effort at
    /// discovery; `0` when the daemon doesn't report one). Display/discovery only
    /// (SN-8) — it never authorizes a route, and it is never journaled.
    context_len: RwLock<BTreeMap<String, u32>>,
    /// PR-B2: the tags that declare vision (`/api/show` capability / `projector_info`),
    /// populated best-effort at discovery. Membership is the vision-modality gate the
    /// Multimodal arm checks BEFORE any egress (honest-degrade a non-vision tag).
    /// Display/discovery only (SN-8) — never journaled.
    vision: RwLock<BTreeSet<String>>,
    /// PR-B2: the content store the Multimodal arm fetches an image `content_ref`'s
    /// bytes from (bound by the host via [`Self::with_content_store`] when any served
    /// tag is vision-capable). `None` on a text-only serve ⇒ a Multimodal dispatch
    /// fails closed ("no content store bound").
    content_store: Option<Arc<dyn ContentFetcher>>,
}

impl std::fmt::Debug for OllamaBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OllamaBackend")
            .field("client", &self.client)
            .field("models", &self.models)
            .field("context_len", &self.context_len)
            .field("vision", &self.vision)
            .field("content_store", &self.content_store.is_some())
            .finish()
    }
}

impl OllamaBackend {
    /// Construct a backend that serves exactly `models` through `client`. Context
    /// windows are left empty (reported `0`); [`Self::discover`] is the path that
    /// populates them from `/api/show`.
    #[must_use]
    pub fn new(client: Arc<OllamaClient>, models: BTreeSet<String>) -> Self {
        Self {
            client,
            models: RwLock::new(models),
            context_len: RwLock::new(BTreeMap::new()),
            vision: RwLock::new(BTreeSet::new()),
            content_store: None,
        }
    }

    /// Bind the content store the Multimodal (vision) arm fetches image bytes from
    /// (PR-B2). The host calls this once at startup when any served tag is
    /// vision-capable; a text-only serve leaves it unbound and a Multimodal dispatch
    /// fails closed. Mirrors the in-process llama backend's `with_content_store`.
    #[must_use]
    pub fn with_content_store(mut self, store: Arc<dyn ContentFetcher>) -> Self {
        self.content_store = Some(store);
        self
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
        // One `/api/show` per served tag, best-effort: it carries BOTH the context
        // window AND the vision capability (PR-B2). A daemon that errors / omits a
        // field honest-degrades (ctx `0`, vision `false`) and never blocks startup.
        let mut context_len = BTreeMap::new();
        let mut vision = BTreeSet::new();
        for tag in &models {
            let meta = client.show_meta(tag).unwrap_or(crate::client::ShowMeta {
                context_length: 0,
                vision: false,
            });
            context_len.insert(tag.clone(), meta.context_length);
            if meta.vision {
                vision.insert(tag.clone());
            }
        }
        Ok(Self {
            client,
            models: RwLock::new(models),
            context_len: RwLock::new(context_len),
            vision: RwLock::new(vision),
            content_store: None,
        })
    }

    /// Download `tag` from the Ollama registry via `/api/pull` (Model Control v2),
    /// forwarding byte progress to `on_progress`. The served set is NOT changed here —
    /// call [`Self::register_tag`] after a successful pull to start serving it.
    ///
    /// # Errors
    /// Propagates [`OllamaClient::pull`]'s transport / status / protocol failures.
    pub fn pull(
        &self,
        tag: &str,
        on_progress: &mut dyn FnMut(&str, u64, u64),
    ) -> Result<(), OllamaError> {
        self.client.pull(tag, on_progress)
    }

    /// Register a tag at RUNTIME after `kx models pull` (Model Control v2). Re-probes
    /// `/api/tags` to CONFIRM the daemon now serves the tag (never serve a phantom),
    /// then adds it to the served set + caches its `/api/show` context window. A tag
    /// already served is a benign no-op (idempotent).
    ///
    /// # Errors
    /// [`OllamaError`] from the `/api/tags` re-probe; [`OllamaError::Protocol`] when
    /// the daemon does not report the tag after the pull (fail-closed).
    pub fn register_tag(&self, tag: &str) -> Result<(), OllamaError> {
        if self.supports_tag(tag) {
            return Ok(());
        }
        // Re-probe: only register a tag the daemon actually has now.
        let tags = self.client.tags()?;
        if !tags.iter().any(|t| t == tag) {
            return Err(OllamaError::Protocol(format!(
                "tag {tag} is not present in the daemon after the pull"
            )));
        }
        let meta = self
            .client
            .show_meta(tag)
            .unwrap_or(crate::client::ShowMeta {
                context_length: 0,
                vision: false,
            });
        self.models
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(tag.to_string());
        self.context_len
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(tag.to_string(), meta.context_length);
        if meta.vision {
            self.vision
                .write()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(tag.to_string());
        }
        Ok(())
    }

    /// Whether the served set currently contains `tag` (read-lock).
    fn supports_tag(&self, tag: &str) -> bool {
        self.models
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(tag)
    }

    /// Whether `tag` declared vision at discovery (PR-B2). The catalog uses this to
    /// mark the `"image"` modality; the Multimodal arm gates on it before egress.
    fn is_vision_tag(&self, tag: &str) -> bool {
        self.vision
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(tag)
    }

    /// Whether `model_id` is a vision-capable served tag (PR-B2). For the host
    /// catalog (`ollama_catalog_entry`) to declare the image modality.
    #[must_use]
    pub fn is_vision(&self, model_id: &ModelId) -> bool {
        self.is_vision_tag(&model_id.0)
    }

    /// The model ids this backend serves (for the host's model catalog).
    #[must_use]
    pub fn model_ids(&self) -> Vec<ModelId> {
        self.models
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .cloned()
            .map(ModelId)
            .collect()
    }

    /// The declared context window for `model_id` (fetched from `/api/show` at
    /// discovery), or `0` when unknown. Display/discovery only (SN-8) — for parity
    /// with the llama backend's GGUF `n_ctx`, surfaced in `kx models list`.
    #[must_use]
    pub fn context_len(&self, model_id: &ModelId) -> u32 {
        self.context_len
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&model_id.0)
            .copied()
            .unwrap_or(0)
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
            Ok(names) => {
                let served = self.models.read().unwrap_or_else(PoisonError::into_inner);
                names
                    .into_iter()
                    .filter(|n| served.contains(n))
                    .map(ModelId)
                    .collect()
            }
            Err(_) => Vec::new(),
        }
    }

    /// Fail-closed served-set gate.
    fn ensure_served(&self, model_id: &ModelId) -> Result<(), InferenceError> {
        if self.supports_tag(&model_id.0) {
            Ok(())
        } else {
            Err(InferenceError::ModelNotFound {
                model_id: model_id.0.clone(),
            })
        }
    }

    /// Resolve a Multimodal dispatch's `content_refs` to base64-encoded image
    /// payloads, enforcing the SAME fail-closed gate ladder as the in-process llama
    /// backend's `resolve_image_refs` (PR-B2), in the SAME order, all BEFORE egress:
    /// 1. the served tag must declare vision (`/api/show`) — else a non-vision model
    ///    would silently answer without the image (a lie);
    /// 2. a content store must be bound;
    /// 3. each ref must resolve in the store;
    /// 4. each payload must be within the warrant's `mem_bytes` ceiling (the 16 MiB
    ///    vision-recipe ceiling) — checked BEFORE the base64 alloc;
    /// 5. each payload must sniff as a recognized image (audio/other reserved).
    fn resolve_images(
        &self,
        model_id: &ModelId,
        content_refs: &[ContentRef],
        warrant: &WarrantSpec,
    ) -> Result<Vec<String>, InferenceError> {
        if !self.is_vision_tag(&model_id.0) {
            return Err(InferenceError::Unsupported {
                reason: "model does not declare vision; cannot serve a multimodal request",
            });
        }
        let store = self
            .content_store
            .as_ref()
            .ok_or(InferenceError::Unsupported {
                reason: "no content store bound; cannot fetch multimodal content_refs",
            })?;
        let cap = warrant.resource_ceiling.mem_bytes;
        let mut images = Vec::with_capacity(content_refs.len());
        for r in content_refs {
            let bytes = store
                .fetch(r)
                .ok_or(InferenceError::ContentStoreMiss { content_ref: *r })?;
            let len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
            if len > cap {
                return Err(InferenceError::ScopeViolation {
                    field: "image_bytes",
                    requested: len,
                    ceiling: cap,
                });
            }
            if sniff_image_format(&bytes).is_none() {
                return Err(InferenceError::Unsupported {
                    reason: "content_ref is not a recognized image; audio and other \
                             modalities are reserved for later PRs",
                });
            }
            images.push(base64::engine::general_purpose::STANDARD.encode(&bytes));
        }
        Ok(images)
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
        // `images` are base64-encoded payloads passed out-of-band in the `/api/generate`
        // `images` array (PR-B2 vision); the text path leaves it empty (byte-identical
        // body). For a `Multimodal` dispatch the marker is stripped from the prompt —
        // Ollama splices the image(s) per the model's projector, NOT by marker position.
        let (prompt, images) = match input {
            InferenceInput::Text(prompt) => (prompt.clone(), Vec::new()),
            InferenceInput::Multimodal { text, content_refs } => {
                let images = self.resolve_images(model_id, content_refs, warrant)?;
                (text.replace(MEDIA_MARKER, ""), images)
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
                &images,
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
        self.supports_tag(&model_id.0)
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
