// `LlamaInferenceBackend` — the OSS v0.1 in-process llama.cpp backend.
//
// Per D28 (OSS stays lean) this is the ONLY backend that ships in the OSS
// repo. Out-of-process backends (Triton, vLLM, remote APIs) ride the same
// `InferenceBackend` trait but live in private `kx-cloud/*` crates.
//
// v0.1 design choices:
//   - Model registry is a frozen-at-construction `HashMap<ModelId, PathBuf>`.
//     Reloading a different model is a `kx-llamacpp` constructor reset; the
//     runtime doesn't reload models per-dispatch (they're heavyweight).
//   - LlamaBackend is RAII-ref-counted inside kx-llamacpp; we call ::new()
//     per dispatch since the type is `!Send + !Sync` and we cannot hold it
//     long-term in a Send + Sync backend struct. The internal mutex makes
//     repeated `LlamaBackend::new()` calls cheap (subsequent calls just
//     bump a ref-count; the actual `llama_backend_init` runs once).
//   - The Model load IS expensive (seconds for a 7B GGUF). Future PRs may
//     introduce a long-lived `LoadedModel` cache (via worker thread + mpsc
//     channel to dodge the `!Send` constraint). v0.1 ships the simpler
//     per-call path because the OSS critical-path goal is the seam,
//     not throughput.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use kx_mote::ModelId;
use kx_warrant::WarrantSpec;

use crate::backend::InferenceBackend;
use crate::types::{
    check_within, InferenceError, InferenceInput, InferenceOutput, InferenceParams,
};

/// Backend name reported in `InferenceOutput.backend_name`.
const BACKEND_NAME: &str = "kx-llamacpp";

/// Default context window. Per llama.cpp convention, 0 means "use the
/// model's own `n_ctx_train`". We pick a conservative cap so v0.1 doesn't
/// silently allocate huge KV caches.
const DEFAULT_N_CTX: u32 = 4096;

/// Convert a `kx_llamacpp::LlamaError` into our public error enum.
///
/// Localised to one place so the dispatcher's error surface stays
/// stable as `kx-llamacpp`'s error variants evolve. Takes the error
/// by value so it can be used directly as `.map_err(map_llama_err)`
/// in the dispatch path; the underlying error's `Display` is what
/// we ultimately surface so consuming the original is fine.
#[allow(clippy::needless_pass_by_value)]
fn map_llama_err(err: kx_llamacpp::LlamaError) -> InferenceError {
    InferenceError::BackendFailure {
        backend: BACKEND_NAME,
        message: format!("{err}"),
    }
}

/// OSS v0.1 in-process inference backend wrapping `kx-llamacpp`.
///
/// **What it implements**:
///   - `InferenceInput::Text(prompt)` — runs the prompt through the
///     loaded model.
///   - `InferenceParams.grammar = None` — vanilla sampling (greedy when
///     `temperature_bps == 0`, otherwise temp + top-k + top-p).
///
/// **What it returns `Err(Unsupported)` on** (deliberate seam):
///   - `InferenceInput::Multimodal { .. }` — reserved for future PRs.
///   - `InferenceParams.grammar = Some(_)` — reserved for future PRs.
///
/// See `roadmap-multimodal-synthesis-post-pr9` for the sequencing
/// commitment.
#[derive(Debug, Clone)]
pub struct LlamaInferenceBackend {
    model_paths: Arc<HashMap<ModelId, PathBuf>>,
    n_ctx: u32,
}

impl LlamaInferenceBackend {
    /// Construct a backend with no registered models. Useful for unit
    /// tests where every dispatch is expected to return `ModelNotFound`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            model_paths: Arc::new(HashMap::new()),
            n_ctx: DEFAULT_N_CTX,
        }
    }

    /// Construct a backend with a single model registered.
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
        let mut map = HashMap::new();
        map.insert(id, path);
        Self {
            model_paths: Arc::new(map),
            n_ctx: DEFAULT_N_CTX,
        }
    }

    /// Construct a backend with multiple model ids registered.
    #[must_use]
    pub fn with_models(models: HashMap<ModelId, PathBuf>) -> Self {
        Self {
            model_paths: Arc::new(models),
            n_ctx: DEFAULT_N_CTX,
        }
    }

    /// Override the default context window.
    #[must_use]
    pub fn with_n_ctx(mut self, n_ctx: u32) -> Self {
        self.n_ctx = n_ctx;
        self
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
        // ---- Forward-compat reservation gates (PR 8 plan) -----------------
        let prompt = match input {
            InferenceInput::Text(s) => s.as_str(),
            InferenceInput::Multimodal { .. } => {
                return Err(InferenceError::Unsupported {
                    reason: "multimodal input reserved for post-PR-9; see HANDOFF §3.8",
                });
            }
        };
        if params.grammar.is_some() {
            return Err(InferenceError::Unsupported {
                reason: "constrained generation (grammar) reserved for post-PR-9; see HANDOFF §3.8",
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

        // ---- Resolve registered path --------------------------------------
        let path = self
            .model_paths
            .get(model_id)
            .ok_or_else(|| InferenceError::ModelNotFound {
                model_id: model_id.0.clone(),
            })?;

        // ---- Timeout setup -------------------------------------------------
        let start = Instant::now();
        let timeout = Duration::from_millis(warrant.resource_ceiling.wall_clock_ms);
        let check_timeout = |elapsed: Duration| -> Result<(), InferenceError> {
            if elapsed >= timeout {
                Err(InferenceError::Timeout {
                    wall_clock_ms: warrant.resource_ceiling.wall_clock_ms,
                })
            } else {
                Ok(())
            }
        };

        // ---- llama.cpp setup (per-call; see module docs) ------------------
        let backend = kx_llamacpp::LlamaBackend::new().map_err(map_llama_err)?;
        let model = kx_llamacpp::Model::load(&backend, path).map_err(map_llama_err)?;
        check_timeout(start.elapsed())?;

        let ctx_params = kx_llamacpp::ContextParams::new().with_n_ctx(self.n_ctx);
        let mut ctx =
            kx_llamacpp::Context::new_with_params(&model, &ctx_params).map_err(map_llama_err)?;
        let vocab = model.vocab();

        let prompt_tokens = vocab.tokenize(prompt, true, false).map_err(map_llama_err)?;
        check_timeout(start.elapsed())?;

        // ---- Sampler chain -------------------------------------------------
        let mut sampler = if params.temperature_bps == 0 {
            kx_llamacpp::Sampler::greedy(&backend).map_err(map_llama_err)?
        } else {
            #[allow(clippy::cast_precision_loss)]
            let temp = (params.temperature_bps as f32) / 10_000.0;
            #[allow(clippy::cast_precision_loss)]
            let top_p = (params.top_p_bps as f32) / 10_000.0;
            #[allow(clippy::cast_possible_wrap)]
            let top_k = params.top_k as i32;
            kx_llamacpp::Sampler::typical(&backend, temp, top_k, top_p, params.seed)
                .map_err(map_llama_err)?
        };

        // ---- Generation loop ----------------------------------------------
        let generator = kx_llamacpp::Generator::new(&mut ctx, &mut sampler, &vocab, prompt_tokens)
            .map_err(map_llama_err)?;

        let mut output_bytes: Vec<u8> = Vec::with_capacity(
            usize::try_from(params.max_output_tokens.saturating_mul(4)).unwrap_or(2048),
        );
        let mut output_tokens: u32 = 0;

        for token_result in generator {
            check_timeout(start.elapsed())?;
            let token = token_result.map_err(map_llama_err)?;
            vocab
                .token_to_piece_into(token, 0, false, &mut output_bytes)
                .map_err(map_llama_err)?;
            output_tokens = output_tokens.saturating_add(1);
            if output_tokens >= params.max_output_tokens {
                break;
            }
            if vocab.is_eog(token) {
                break;
            }
        }

        Ok(InferenceOutput {
            bytes: output_bytes,
            output_tokens,
            backend_name: BACKEND_NAME,
            model_id: model_id.clone(),
            elapsed: start.elapsed(),
        })
    }

    fn supports(&self, model_id: &ModelId) -> bool {
        self.model_paths.contains_key(model_id)
    }

    fn name(&self) -> &'static str {
        BACKEND_NAME
    }
}
