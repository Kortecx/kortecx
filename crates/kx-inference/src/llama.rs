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

use kx_content::{sniff_image_format, ContentRef, ContentStore, NotFound};
use kx_model_store::{Modality, ModelDescriptor, ModelRegistry, ModelResolver};
use kx_mote::ModelId;
use kx_warrant::WarrantSpec;
use smallvec::SmallVec;

use crate::backend::{EmbeddingBackend, InferenceBackend};
use crate::cache::{ModelCache, DEFAULT_CACHE_CAPACITY};
use crate::types::{
    check_within, EmbeddingOutput, EmbeddingPooling, InferenceError, InferenceInput,
    InferenceOutput, InferenceParams,
};

/// Object-safe byte fetcher that erases [`ContentStore`]'s associated `Payload`
/// type so the backend can hold a single trait object regardless of the store
/// implementation. Blanket-implemented for every `Send + Sync` `ContentStore`,
/// so callers pass an `Arc<ConcreteStore>` and it coerces to
/// `Arc<dyn ContentFetcher>` directly.
///
/// Lives here (not in `kx-content`) because it exists solely to let the
/// multi-modal backend fetch a `content_ref`'s bytes; the store trait stays
/// generic for its hot-path callers (assembler, executor) that use `&S`.
pub trait ContentFetcher: Send + Sync {
    /// Fetch the bytes at `r`, or `None` if the store has no such object.
    fn fetch(&self, r: &ContentRef) -> Option<Vec<u8>>;
}

impl<S> ContentFetcher for S
where
    S: ContentStore + Send + Sync + ?Sized,
{
    fn fetch(&self, r: &ContentRef) -> Option<Vec<u8>> {
        match self.get(r) {
            Ok(payload) => Some(payload.to_vec()),
            Err(NotFound) => None,
        }
    }
}

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
    /// Optional content store the multi-modal path fetches `content_ref` image
    /// bytes from. `None` on the text-only path (the default); an image
    /// dispatch with no store bound fails closed with `Unsupported`.
    content_store: Option<Arc<dyn ContentFetcher>>,
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
            content_store: None,
            n_ctx: DEFAULT_N_CTX,
            cache_capacity: DEFAULT_CACHE_CAPACITY,
        }
    }

    /// Bind a content store the multi-modal path fetches image bytes from.
    ///
    /// Additive: text-only dispatch ignores it. Any `Send + Sync`
    /// [`ContentStore`] (e.g. an `Arc<LocalFsContentStore>`) coerces in via the
    /// [`ContentFetcher`] blanket impl. Without it, an image dispatch fails
    /// closed (`Unsupported`).
    #[must_use]
    pub fn with_content_store(mut self, store: Arc<dyn ContentFetcher>) -> Self {
        self.content_store = Some(store);
        self
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

    /// Construct a backend serving a single image (vision) model: the VLM
    /// weights `gguf` plus its vision projector `mmproj`. Pair with
    /// [`Self::with_content_store`] so the multi-modal path can fetch image
    /// `content_ref`s. Files need not exist at construction (lazy, like
    /// [`Self::with_model`]); the first image dispatch loads them.
    #[must_use]
    pub fn with_image_model(id: ModelId, gguf: PathBuf, mmproj: PathBuf) -> Self {
        let mut registry = ModelRegistry::new();
        if let Err(e) = registry.register(ModelDescriptor::image(id, gguf, mmproj, DEFAULT_N_CTX)) {
            tracing::error!(error = %e, "skipping image model in registry shim (unexpected)");
        }
        Self::with_resolver(Arc::new(registry))
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

    /// Number of cold multi-modal projector (`mmproj`) loads performed so far.
    /// With the PR-2.5 projector cache this rises once per distinct
    /// model+projector — the bundle loads the projector on the first image
    /// dispatch and reuses it thereafter (the base model stays cached too). A
    /// rise on *every* image dispatch would mean the projector cache regressed;
    /// the measured signal, never a claimed figure.
    #[must_use]
    pub fn mmproj_loads_performed(&self) -> u64 {
        self.cache.get().map_or(0, ModelCache::mmproj_loads)
    }

    /// Resolve multimodal `content_refs` to fail-closed, size-capped,
    /// image-sniffed bytes ready for the projector. Gates, in order:
    /// 1. the model must DECLARE the image modality (capability);
    /// 2. a content store must be bound (else nothing to fetch from);
    /// 3. every ref must resolve in the store;
    /// 4. every payload must be within the warrant's `mem_bytes` ceiling
    ///    (the pre-decode size cap — untrusted bytes are never handed to the
    ///    C decoder above the ceiling);
    /// 5. every payload must be a recognized image (audio / unknown are
    ///    reserved for later PRs — rejected, not silently decoded).
    fn resolve_image_refs(
        &self,
        descriptor: &ModelDescriptor,
        content_refs: &[ContentRef],
        warrant: &WarrantSpec,
    ) -> Result<SmallVec<[Vec<u8>; 2]>, InferenceError> {
        if !descriptor.supports(Modality::Image) {
            return Err(InferenceError::Unsupported {
                reason: "model does not declare the image modality; cannot serve a \
                         multimodal request",
            });
        }
        let store = self
            .content_store
            .as_ref()
            .ok_or(InferenceError::Unsupported {
                reason: "no content store bound; cannot fetch multimodal content_refs",
            })?;
        let cap = warrant.resource_ceiling.mem_bytes;
        let mut images: SmallVec<[Vec<u8>; 2]> = SmallVec::with_capacity(content_refs.len());
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
            images.push(bytes);
        }
        Ok(images)
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
            .field("content_store_bound", &self.content_store.is_some())
            .finish_non_exhaustive()
    }
}

impl Default for LlamaInferenceBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl LlamaInferenceBackend {
    /// The shared dispatch body for [`InferenceBackend::dispatch`] (`token_sink`
    /// `None`) and [`InferenceBackend::dispatch_streaming`] (`Some`). Threading
    /// the ADVISORY `token_sink` to the owner thread's generation loop is the
    /// ONLY difference between the two entry points — every gate (grammar /
    /// warrant model-route / ceilings) and the model-cache path are byte-
    /// identical, so the committed `InferenceOutput.bytes` is unchanged.
    fn dispatch_inner(
        &self,
        model_id: &ModelId,
        input: &InferenceInput,
        params: &InferenceParams,
        warrant: &WarrantSpec,
        token_sink: Option<crate::TokenSink>,
    ) -> Result<InferenceOutput, InferenceError> {
        // ---- Grammar reservation gate (fires before any model work) -------
        // Constrained generation is a distinct, still-reserved seam; it gates
        // every input variant.
        if params.grammar.is_some() {
            return Err(InferenceError::Unsupported {
                reason: "constrained generation (grammar) reserved; see HANDOFF",
            });
        }

        // ---- Warrant gates (D30 + D35) — authorize BEFORE touching content -
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

        let cache = self
            .cache
            .get_or_init(|| ModelCache::spawn(self.cache_capacity));

        // ---- Dispatch by input modality -----------------------------------
        match input {
            InferenceInput::Text(prompt) => cache.dispatch(
                descriptor.identity_digest,
                descriptor.gguf_path.clone(),
                model_id.clone(),
                prompt.clone(),
                SmallVec::new(),
                None,
                params.clone(),
                self.n_ctx,
                warrant.resource_ceiling.wall_clock_ms,
                token_sink,
            ),
            InferenceInput::Multimodal { text, content_refs } => {
                let images = self.resolve_image_refs(descriptor, content_refs, warrant)?;
                let mmproj = descriptor
                    .mmproj_path
                    .clone()
                    .ok_or(InferenceError::Unsupported {
                        reason: "image-capable model has no mmproj projector configured",
                    })?;
                cache.dispatch(
                    descriptor.identity_digest,
                    descriptor.gguf_path.clone(),
                    model_id.clone(),
                    text.clone(),
                    images,
                    Some(mmproj),
                    params.clone(),
                    self.n_ctx,
                    warrant.resource_ceiling.wall_clock_ms,
                    token_sink,
                )
            }
            // The completion path produces no embedding; embeddings ride the
            // separate `EmbeddingBackend::dispatch_embedding` seam.
            InferenceInput::TextForEmbedding { .. } => Err(InferenceError::Unsupported {
                reason: "embedding input on the completion path; use \
                         EmbeddingBackend::dispatch_embedding",
            }),
        }
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
        self.dispatch_inner(model_id, input, params, warrant, None)
    }

    /// PR-4.2 (T-STREAM1): the in-process streaming override. Identical to
    /// [`Self::dispatch`] but threads the ADVISORY `token_sink` to the owner
    /// thread's generation loop (each token's new bytes). The committed bytes are
    /// byte-identical to `dispatch`; a `None` sink is exactly `dispatch`.
    fn dispatch_streaming(
        &self,
        model_id: &ModelId,
        input: &InferenceInput,
        params: &InferenceParams,
        warrant: &WarrantSpec,
        token_sink: Option<crate::TokenSink>,
    ) -> Result<InferenceOutput, InferenceError> {
        self.dispatch_inner(model_id, input, params, warrant, token_sink)
    }

    /// Model-agnostic prompt formatting: render `system` + `user` through the
    /// model's OWN chat template. Resolves the descriptor (no warrant gate — this
    /// is pure formatting, not a model route; the route is enforced at `dispatch`)
    /// and renders on the shared owner thread (where the model is resident),
    /// reusing the loaded-model LRU. `None` iff the model id is unresolvable or the
    /// render fails — the caller then falls back to its own formatting.
    fn render_chat(&self, model_id: &ModelId, system: &str, user: &str) -> Option<String> {
        let descriptor = self.resolver.resolve(model_id)?;
        let cache = self
            .cache
            .get_or_init(|| ModelCache::spawn(self.cache_capacity));
        let messages = vec![
            ("system".to_string(), system.to_string()),
            ("user".to_string(), user.to_string()),
        ];
        cache
            .render_chat(
                descriptor.identity_digest,
                descriptor.gguf_path.clone(),
                messages,
            )
            .ok()
    }

    fn supports(&self, model_id: &ModelId) -> bool {
        self.resolver.resolve(model_id).is_some()
    }

    fn name(&self) -> &'static str {
        BACKEND_NAME
    }
}

impl EmbeddingBackend for LlamaInferenceBackend {
    /// Embed `text` for `model_id` under `pooling` (DP1). Authorizes the model
    /// route BEFORE touching the model (SN-8 / D35), resolves the descriptor,
    /// and runs the embed on the shared model-cache owner thread (reusing an
    /// already-loaded model). There is no `max_output_tokens` axis to narrow
    /// (an embedding emits no tokens), so the only quantitative ceiling is the
    /// warrant's wall-clock.
    fn dispatch_embedding(
        &self,
        model_id: &ModelId,
        text: &str,
        pooling: EmbeddingPooling,
        warrant: &WarrantSpec,
    ) -> Result<EmbeddingOutput, InferenceError> {
        // ---- Warrant gate (D35) — the route MUST authorize this model id ----
        if model_id != &warrant.model_route.model_id {
            return Err(InferenceError::WarrantDeniesModel {
                model_id: model_id.0.clone(),
                route: warrant.model_route.model_id.0.clone(),
            });
        }

        // ---- Resolve the model descriptor (paths) ---------------------------
        let descriptor =
            self.resolver
                .resolve(model_id)
                .ok_or_else(|| InferenceError::ModelNotFound {
                    model_id: model_id.0.clone(),
                })?;

        let cache = self
            .cache
            .get_or_init(|| ModelCache::spawn(self.cache_capacity));

        cache.dispatch_embedding(
            descriptor.identity_digest,
            descriptor.gguf_path.clone(),
            model_id.clone(),
            text.to_string(),
            pooling,
            warrant.resource_ceiling.wall_clock_ms,
        )
    }
}
