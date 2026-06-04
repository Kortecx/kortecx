// `LlamaInferenceBackend` — the OSS v0.1 in-process llama.cpp backend.
//
// Per D28 (OSS stays lean) this is the ONLY backend that ships in the OSS
// repo. Out-of-process backends (Triton, vLLM, remote APIs) ride the same
// `InferenceBackend` trait but live in private `kx-cloud/*` crates.
//
// Design (post-M4, D108.2):
//   - Model identity + paths + declared modalities come from a
//     `kx_model_store::ModelResolver` (a `BTreeMap`-backed `ModelRegistry` by
//     default), NOT a frozen `HashMap<ModelId, PathBuf>`. A future durable /
//     remote registry implements the same trait without this backend changing.
//   - Loaded `llama_model` handles are CACHED across dispatches by a dedicated
//     owner thread (see `cache.rs`). The pre-M4 path reloaded the model from
//     disk on EVERY dispatch (seconds for a 7B model; ruinous for a multi-GB
//     multimodal model); the cache loads each model once, keyed by its
//     `identity_digest`. `LlamaBackend`/`Model` are `!Send`/`!Sync`, so the
//     owner thread (which holds only a `Send + Sync` channel handle) is what
//     keeps this backend `Send + Sync`.
//   - `with_model` / `with_models` are retained as thin shims that build a
//     one-/multi-entry `ModelRegistry`, so existing callers are unchanged.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use kx_model_store::{ModelDescriptor, ModelRegistry, ModelResolver};
use kx_mote::ModelId;
use kx_warrant::WarrantSpec;

use crate::backend::InferenceBackend;
use crate::cache::{ModelCache, DEFAULT_CACHE_CAPACITY};
use crate::types::{
    check_within, InferenceError, InferenceInput, InferenceOutput, InferenceParams,
};

/// Backend name reported in `InferenceOutput.backend_name`.
pub(crate) const BACKEND_NAME: &str = "kx-llamacpp";

/// Default context window. Per llama.cpp convention, 0 means "use the
/// model's own `n_ctx_train`". We pick a conservative cap so v0.1 doesn't
/// silently allocate huge KV caches.
pub(crate) const DEFAULT_N_CTX: u32 = 4096;

/// OSS v0.1 in-process inference backend wrapping `kx-llamacpp`.
///
/// **What it implements**:
///   - `InferenceInput::Text(prompt)` — runs the prompt through the
///     loaded (and cached) model.
///   - `InferenceParams.grammar = None` — vanilla sampling (greedy when
///     `temperature_bps == 0`, otherwise temp + top-k + top-p).
///
/// **What it returns `Err(Unsupported)` on** (deliberate seam):
///   - `InferenceInput::Multimodal { .. }` — reserved for the multi-modal PRs.
///   - `InferenceParams.grammar = Some(_)` — reserved for constrained generation.
#[derive(Clone)]
pub struct LlamaInferenceBackend {
    /// The model registry / resolver this backend serves from.
    resolver: Arc<dyn ModelResolver>,
    /// Lazily-spawned loaded-model cache (owner thread). `OnceLock` so a backend
    /// that never dispatches (e.g. a unit test expecting `ModelNotFound`) never
    /// spawns a thread or touches the FFI. Shared across `Clone`s via `Arc`.
    cache: Arc<OnceLock<ModelCache>>,
    /// Context window passed to each dispatch.
    n_ctx: u32,
    /// Number of distinct models the cache keeps loaded at once.
    cache_capacity: usize,
}

impl LlamaInferenceBackend {
    /// Construct a backend with no registered models. Useful for unit
    /// tests where every dispatch is expected to return `ModelNotFound`.
    #[must_use]
    pub fn new() -> Self {
        Self::with_resolver(Arc::new(ModelRegistry::new()))
    }

    /// Construct a backend that serves from an arbitrary [`ModelResolver`]
    /// (e.g. a richly-populated `ModelRegistry`, or a future durable registry).
    #[must_use]
    pub fn with_resolver(resolver: Arc<dyn ModelResolver>) -> Self {
        Self {
            resolver,
            cache: Arc::new(OnceLock::new()),
            n_ctx: DEFAULT_N_CTX,
            cache_capacity: DEFAULT_CACHE_CAPACITY,
        }
    }

    /// Construct a backend with a single text model registered.
    ///
    /// # Examples
    ///
    /// ```
    /// use kx_inference::LlamaInferenceBackend;
    /// use kx_mote::ModelId;
    /// use std::path::PathBuf;
    ///
    /// let backend = LlamaInferenceBackend::with_model(
    ///     ModelId("llama-3-8b-instruct".into()),
    ///     PathBuf::from("/tmp/llama-3-8b-instruct.gguf"),
    /// );
    /// // The model file does NOT need to exist at construction time;
    /// // it's only opened on the first dispatch that requests this
    /// // model id. Construction is lazy on purpose.
    /// let _ = backend;
    /// ```
    #[must_use]
    pub fn with_model(id: ModelId, path: PathBuf) -> Self {
        Self::with_resolver(Arc::new(registry_from_paths(std::iter::once((id, path)))))
    }

    /// Construct a backend with multiple text model ids registered.
    #[must_use]
    pub fn with_models(models: HashMap<ModelId, PathBuf>) -> Self {
        Self::with_resolver(Arc::new(registry_from_paths(models)))
    }

    /// Override the default context window.
    #[must_use]
    pub fn with_n_ctx(mut self, n_ctx: u32) -> Self {
        self.n_ctx = n_ctx;
        self
    }

    /// Override the loaded-model cache capacity (distinct models kept resident).
    #[must_use]
    pub fn with_cache_capacity(mut self, capacity: usize) -> Self {
        self.cache_capacity = capacity.max(1);
        self
    }

    /// Number of cold `Model::load`s performed so far (0 if no dispatch has
    /// spawned the cache yet). The observable proof that cache hits do not
    /// reload, and the ops metric for model-load pressure.
    #[must_use]
    pub fn loads_performed(&self) -> u64 {
        self.cache.get().map_or(0, ModelCache::loads)
    }
}

/// Build a [`ModelRegistry`] of text-only descriptors from `(ModelId, path)`
/// pairs. Registration of a fresh, unique set is infallible; a duplicate or an
/// over-capacity entry is logged and skipped rather than panicking in an
/// infallible constructor (a `HashMap` source cannot contain duplicates).
fn registry_from_paths(paths: impl IntoIterator<Item = (ModelId, PathBuf)>) -> ModelRegistry {
    let mut registry = ModelRegistry::new();
    for (id, path) in paths {
        if let Err(e) = registry.register(ModelDescriptor::text(id, path, DEFAULT_N_CTX)) {
            tracing::error!(error = %e, "skipping model in registry shim (unexpected)");
        }
    }
    registry
}

impl std::fmt::Debug for LlamaInferenceBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LlamaInferenceBackend")
            .field("n_ctx", &self.n_ctx)
            .field("cache_capacity", &self.cache_capacity)
            .field("cache_spawned", &self.cache.get().is_some())
            .finish_non_exhaustive()
    }
}

impl Default for LlamaInferenceBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl InferenceBackend for LlamaInferenceBackend {
    fn dispatch(
        &self,
        model_id: &ModelId,
        input: &InferenceInput,
        params: &InferenceParams,
        warrant: &WarrantSpec,
    ) -> Result<InferenceOutput, InferenceError> {
        // ---- Forward-compat reservation gates (fire BEFORE any model work) ----
        let prompt = match input {
            InferenceInput::Text(s) => s.as_str(),
            InferenceInput::Multimodal { .. } => {
                return Err(InferenceError::Unsupported {
                    reason: "multimodal input reserved for the multi-modal PRs; see HANDOFF",
                });
            }
        };
        if params.grammar.is_some() {
            return Err(InferenceError::Unsupported {
                reason: "constrained generation (grammar) reserved; see HANDOFF",
            });
        }

        // ---- Warrant gates (D30 + D35) ------------------------------------
        if model_id != &warrant.model_route.model_id {
            return Err(InferenceError::WarrantDeniesModel {
                model_id: model_id.0.clone(),
                route: warrant.model_route.model_id.0.clone(),
            });
        }
        check_within(params, warrant)?;

        // ---- Resolve the model descriptor (paths + capabilities) ----------
        let descriptor =
            self.resolver
                .resolve(model_id)
                .ok_or_else(|| InferenceError::ModelNotFound {
                    model_id: model_id.0.clone(),
                })?;

        // ---- Dispatch through the loaded-model cache (owner thread) -------
        let cache = self
            .cache
            .get_or_init(|| ModelCache::spawn(self.cache_capacity));
        cache.dispatch(
            descriptor.identity_digest,
            descriptor.gguf_path.clone(),
            model_id.clone(),
            prompt.to_string(),
            params.clone(),
            self.n_ctx,
            warrant.resource_ceiling.wall_clock_ms,
        )
    }

    fn supports(&self, model_id: &ModelId) -> bool {
        self.resolver.resolve(model_id).is_some()
    }

    fn name(&self) -> &'static str {
        BACKEND_NAME
    }
}
