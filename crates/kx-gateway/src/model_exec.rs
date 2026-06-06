//! AL1 — live `kx serve` LLM dispatch (feature `inference`, OFF by default).
//!
//! R1 wired the embedded worker to a deterministic content-storing executor;
//! PR-9b added real sandboxed body-exec. This adds the **third** arm: a leased
//! **model Mote** (one carrying a `prompt` and routed to the registered serve
//! model) is run through the in-process [`LlamaInferenceBackend`], its completion
//! bytes published into the shared store (so the coordinator's D55 ref-existence
//! guard passes), and the commit proposed — all WITHOUT touching the frozen trio
//! (`kx-executor`/`kx-scheduler`/`kx-inference`): it composes the EXISTING public
//! `InferenceBackend` surface behind a [`MoteExecutor`] the gateway binary owns.
//!
//! [`ModelRouterExecutor`] wraps the inner [`crate::real_exec::RouterExecutor`]:
//! a model Mote → inference; everything else (real-body, PURE echo) → the inner
//! router, byte-for-byte unchanged. The whole module is `#[cfg(feature =
//! "inference")]`, so the default FFI-free build (and the dep-wall) is unaffected.
//!
//! Scope (v1): the **greedy / PURE** model path only — a greedy decode is
//! recomputable, so it is sound through the per-Mote executor seam. Stochastic
//! (ReadOnlyNondet) dispatch + D78 upstream-context assembly are follow-ons.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

use kx_content::{ContentStore, LocalFsContentStore};
use kx_executor::{MoteExecutionResult, MoteExecutor, MoteExecutorError, Rootfs};
use kx_inference::{
    inference_params_from_mote, InferenceBackend, InferenceInput, LlamaInferenceBackend,
};
use kx_model_store::{read_context_length, ModelDescriptor, ModelRegistry};
use kx_model_validator::{
    check, License, LicenseConstraint, Modality, ProvidedCapabilities, Quantization,
    RequiredCapabilities, ValidatorOutcome,
};
use kx_mote::{ConfigKey, ModelId, Mote, PROMPT_KEY};
use kx_warrant::{ExecutorClass, WarrantSpec};

/// Default context window when the GGUF declares none.
const DEFAULT_SERVE_N_CTX: u32 = 4096;
/// Ceiling on the served context window — bounds KV-cache memory regardless of
/// the model's (possibly very large) declared training context.
const MAX_SERVE_N_CTX: u32 = 8192;
/// Minimum context the agent's tool-use loop needs (mirrors the harness's
/// `kx_model_harness::registration::AGENT_MIN_CTX_TOKENS`).
const AGENT_MIN_CTX_TOKENS: u32 = 2048;

/// Qwen ChatML wrapping of a user prompt — the **training contract** the
/// companion model repo mirrors (kept byte-identical to
/// `kx_model_harness::prompt::chatml`; duplicated here so the production gateway
/// need not depend on the eval harness).
#[must_use]
fn chatml(prompt: &str) -> String {
    format!(
        "<|im_start|>system\nYou are a precise assistant. Follow the instruction exactly.<|im_end|>\n<|im_start|>user\n{prompt}<|im_end|>\n<|im_start|>assistant\n"
    )
}

/// Resolve the serve model GGUF: the `KX_SERVE_MODEL_GGUF` env path, iff it
/// exists. `None` ⇒ no model serving (the model recipe is not provisioned), so
/// `kx serve --features inference` still runs the durable spine + demo recipes.
pub(crate) fn resolve_serve_model() -> Option<std::path::PathBuf> {
    let p = std::path::PathBuf::from(std::env::var_os("KX_SERVE_MODEL_GGUF")?);
    p.is_file().then_some(p)
}

/// A stable [`ModelId`] for the served model, derived from the GGUF file stem so
/// a different model file yields a different id (hence distinct Mote identities).
/// Used identically by [`build_serve_backend`] (registration) and the model
/// recipe's warrant (`model_route.model_id`), so the backend's warrant check
/// (`model_id == warrant.model_route.model_id`) holds.
#[must_use]
pub(crate) fn serve_model_id(gguf: &Path) -> ModelId {
    let stem = gguf
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("kx-serve-model");
    ModelId(format!("kx-serve:{stem}"))
}

/// The kortecx agent's required model signature (mirrors the harness's
/// `kortecx_agent_requirements`): native tool-calling, Text, a chat template, a
/// commercial-OK license, a `q4_k_m`/`q8_0`/`f16` quantization.
fn agent_requirements() -> RequiredCapabilities {
    RequiredCapabilities {
        min_context_window_tokens: AGENT_MIN_CTX_TOKENS,
        requires_native_tool_calling: true,
        prefers_native_tool_calling: true,
        required_modalities: BTreeSet::from([Modality::Text]),
        allowed_quantizations: BTreeSet::from([
            Quantization::Q4KM,
            Quantization::Q8_0,
            Quantization::F16,
        ]),
        requires_chat_template: true,
        license_constraint: LicenseConstraint::RequireCommercialOk,
    }
}

/// The Apache-2.0 / Text / native-tool-calling / ChatML / `q4_k_m` capabilities
/// the campaign model declares at `context_window`.
fn agent_provided(context_window: u32) -> ProvidedCapabilities {
    ProvidedCapabilities::declared()
        .with_context_window_tokens(context_window)
        .with_native_tool_calling(true)
        .with_modalities(BTreeSet::from([Modality::Text]))
        .with_quantization(Quantization::Q4KM)
        .with_chat_template(Some("chatml".to_string()))
        .with_license(License::SpdxId("Apache-2.0".to_string()))
}

/// Resolve the served context window: the GGUF's declared `*.context_length`
/// (fail-soft), else [`DEFAULT_SERVE_N_CTX`], clamped to [`MAX_SERVE_N_CTX`].
fn resolve_n_ctx(gguf: &Path) -> u32 {
    read_context_length(gguf)
        .unwrap_or(DEFAULT_SERVE_N_CTX)
        .clamp(AGENT_MIN_CTX_TOKENS, MAX_SERVE_N_CTX)
}

/// Build the in-process inference backend for the served model, fail-closed: the
/// model's declared capabilities must type-check `TypeOk` against the agent's
/// required signature, or serving the model is refused (a clean error rather
/// than discovering unfitness mid-dispatch).
///
/// Returns the backend (ready for the model arm) on success.
///
/// # Errors
/// A string diagnostic if the model is not fit (not `TypeOk`) or the registry
/// rejects the descriptor.
pub(crate) fn build_serve_backend(
    gguf: &Path,
    model_id: &ModelId,
) -> Result<Arc<LlamaInferenceBackend>, String> {
    let n_ctx = resolve_n_ctx(gguf);
    let provided = agent_provided(n_ctx);
    match check(&provided, &agent_requirements()) {
        ValidatorOutcome::TypeOk => {}
        other => {
            return Err(format!(
                "served model is not TypeOk for the agent: {other:?}"
            ))
        }
    }
    let mut registry = ModelRegistry::new();
    registry
        .register(ModelDescriptor::text(model_id.clone(), gguf, n_ctx))
        .map_err(|e| format!("model registry rejected the descriptor: {e}"))?;
    let backend = LlamaInferenceBackend::with_resolver(Arc::new(registry)).with_n_ctx(n_ctx);
    Ok(Arc::new(backend))
}

/// A [`MoteExecutor`] that runs leased **model Motes** through the in-process
/// [`LlamaInferenceBackend`] and delegates everything else to `inner` (the
/// PR-9b/R1 [`crate::real_exec::RouterExecutor`]).
pub(crate) struct ModelRouterExecutor {
    inner: Arc<dyn MoteExecutor>,
    backend: Arc<LlamaInferenceBackend>,
    store: LocalFsContentStore,
}

impl ModelRouterExecutor {
    /// Wrap `inner` with the model arm backed by `backend`, publishing
    /// completions into `store` (the shared store the coordinator verifies).
    pub(crate) fn new(
        inner: Arc<dyn MoteExecutor>,
        backend: Arc<LlamaInferenceBackend>,
        store: LocalFsContentStore,
    ) -> Self {
        Self {
            inner,
            backend,
            store,
        }
    }

    /// A model Mote: carries a `prompt` AND routes to a model this backend serves.
    /// (The demo `echo`/`exec-demo` Motes carry no prompt, so they fall through.)
    fn is_model_mote(&self, mote: &Mote) -> bool {
        mote.def
            .config_subset
            .contains_key(&ConfigKey(PROMPT_KEY.to_string()))
            && self.backend.supports(&mote.def.model_id)
    }

    /// Run a model Mote: greedy decode the ChatML-wrapped prompt, publish the
    /// completion bytes, return their content ref.
    fn run_model(
        &self,
        mote: &Mote,
        warrant: &WarrantSpec,
    ) -> Result<MoteExecutionResult, MoteExecutorError> {
        let instruction = prompt_from_config(mote)
            .ok_or_else(|| internal("model Mote lost its prompt config key"))?;
        let input = InferenceInput::text(chatml(&instruction));
        // Params come verbatim from the identity-bearing `mote.def.inference_params`
        // (the SOLE permitted constructor, D50), clamped to the warrant.
        let params = inference_params_from_mote(mote, warrant)
            .map_err(|e| internal(&format!("inference params: {e}")))?;
        let out = self
            .backend
            .dispatch(&mote.def.model_id, &input, &params, warrant)
            .map_err(|e| internal(&format!("model dispatch: {e}")))?;
        // The committed `result_ref` is the content hash of the completion — a
        // greedy decode ⇒ identical bytes ⇒ identical ref (exactly-once-per-input).
        let result_ref = self
            .store
            .put(&out.bytes)
            .map_err(|e| internal(&format!("content store put: {e}")))?;
        Ok(MoteExecutionResult {
            result_ref,
            started_at_epoch_ms: 0,
            finished_at_epoch_ms: 0,
        })
    }
}

impl MoteExecutor for ModelRouterExecutor {
    fn run(
        &self,
        mote: &Mote,
        warrant: &WarrantSpec,
        env: Option<Rootfs>,
    ) -> Result<MoteExecutionResult, MoteExecutorError> {
        if self.is_model_mote(mote) {
            self.run_model(mote, warrant)
        } else {
            self.inner.run(mote, warrant, env)
        }
    }

    fn supports(&self, executor_class: ExecutorClass) -> bool {
        // The model arm leases on the same class as the inner router (the embedded
        // worker's single class); delegate the predicate so behavior is identical.
        self.inner.supports(executor_class)
    }
}

/// Extract the model Mote's prompt from `config_subset[PROMPT_KEY]`.
///
/// The recipe binder (kx-invoke free-param substitution) stores a `Str` arg
/// JSON-encoded (`"text"` — with quotes); a directly-built Mote may carry raw
/// bytes. Try a JSON-string decode first, fall back to lossy UTF-8 — so both the
/// recipe (`kx invoke kx/recipes/chat`) and direct-submission paths work.
fn prompt_from_config(mote: &Mote) -> Option<String> {
    let raw = &mote
        .def
        .config_subset
        .get(&ConfigKey(PROMPT_KEY.to_string()))?
        .0;
    Some(
        serde_json::from_slice::<String>(raw)
            .unwrap_or_else(|_| String::from_utf8_lossy(raw).into_owned()),
    )
}

/// A fail-closed [`MoteExecutorError::Internal`] from a `&str` diagnostic.
fn internal(reason: &str) -> MoteExecutorError {
    MoteExecutorError::Internal {
        reason: reason.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serve_model_id_is_stem_derived() {
        let id = serve_model_id(Path::new("/m/qwen3-0.6b-q4_k_m.gguf"));
        assert_eq!(id.0, "kx-serve:qwen3-0.6b-q4_k_m");
    }

    #[test]
    fn agent_signature_is_self_consistent() {
        // The capabilities we DECLARE for the served model satisfy the agent's
        // REQUIRED signature — so a real model (declaring the same) binds TypeOk.
        assert_eq!(
            check(&agent_provided(8192), &agent_requirements()),
            ValidatorOutcome::TypeOk
        );
    }

    #[test]
    fn n_ctx_is_clamped_to_ceiling() {
        // A missing GGUF → default, clamped within [min, max].
        let n = resolve_n_ctx(Path::new("/nonexistent.gguf"));
        assert!((AGENT_MIN_CTX_TOKENS..=MAX_SERVE_N_CTX).contains(&n));
    }

    #[test]
    fn chatml_is_the_training_contract() {
        let p = chatml("hi");
        assert!(p.starts_with("<|im_start|>system\n"));
        assert!(p.ends_with("<|im_start|>assistant\n"));
        assert!(p.contains("<|im_start|>user\nhi<|im_end|>"));
    }

    #[test]
    fn prompt_from_config_handles_json_and_raw() {
        use kx_mote::{
            ConfigVal, EffectPattern, GraphPosition, InferenceParams, InputDataId, LogicRef,
            MoteDef, NdClass, PromptTemplateHash, MOTE_DEF_SCHEMA_VERSION,
        };
        use smallvec::SmallVec;
        use std::collections::BTreeMap;

        let make = |val: &[u8]| {
            let mut cfg = BTreeMap::new();
            cfg.insert(ConfigKey(PROMPT_KEY.to_string()), ConfigVal(val.to_vec()));
            let def = MoteDef {
                critic_check: None,
                logic_ref: LogicRef::from_bytes([0u8; 32]),
                model_id: ModelId("m".into()),
                prompt_template_hash: PromptTemplateHash::from_bytes([0u8; 32]),
                tool_contract: BTreeMap::new(),
                nd_class: NdClass::Pure,
                config_subset: cfg,
                effect_pattern: EffectPattern::IdempotentByConstruction,
                critic_for: None,
                is_topology_shaper: false,
                inference_params: InferenceParams::default(),
                schema_version: MOTE_DEF_SCHEMA_VERSION,
            };
            Mote::new(
                def,
                InputDataId::from_bytes([0u8; 32]),
                GraphPosition(b"t".to_vec()),
                SmallVec::new(),
            )
        };
        // Recipe-bound (JSON-encoded Str) and directly-built (raw) both decode.
        assert_eq!(
            prompt_from_config(&make(br#""Capital of France?""#)).as_deref(),
            Some("Capital of France?")
        );
        assert_eq!(
            prompt_from_config(&make(b"raw prompt")).as_deref(),
            Some("raw prompt")
        );
    }
}
