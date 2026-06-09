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
use std::sync::{Arc, Mutex};

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
use kx_mote::{
    ConfigKey, EffectPattern, InferenceParams, LogicRef, ModelId, Mote, MoteId, NdClass,
    PromptTemplateHash, RoleId, ToolName, PROMPT_KEY,
};
use kx_planner::{
    decode_loop_proposal, decode_replan_proposal, lower_loop_to_topology_decision, max_plan_bytes,
    InMemoryRoleRecipes, ReplanProposal, RoleRecipe, RoleRecipeResolver,
};
use kx_warrant::{
    ExecutorClass, FsScope, InMemoryRoleRegistry, ModelRoute, MoteClass, NetScope, ResourceCeiling,
    Role, RoleRegistry, WarrantSpec,
};

use std::collections::BTreeMap;

use kx_content::ContentRef;

/// Run-policy cap on a shaper's per-decision fan-out (mirrors the harness
/// `LoopBudget::max_children` default). Enforced AFTER the planner decode's structural
/// `MAX_LOOP_STEPS` cap — a decision proposing more children is refused fail-closed, so a
/// runaway model cannot materialize an unbounded DAG.
pub(crate) const SHAPER_MAX_CHILDREN: usize = 8;

/// The single demo worker role a shaper fans out to: a PURE (greedy, recomputable) model
/// step that runs the serve model with the child's per-child `intent` as its prompt.
pub(crate) const WORKER_ROLE: &str = "worker";

/// The provisioned pieces for the LIVE shaper loop (PR-2b): the inference backend (for the
/// executor's model + shaper arms), the role→recipe allowlist the shaper's proposal lowers
/// through, and the role→warrant registry the coordinator narrows children against. Built
/// once at startup from the resolved serve model, so the executor's lowering and the
/// coordinator's narrowing agree on the same `worker` role.
#[derive(Clone)]
pub(crate) struct ShaperRuntime {
    pub(crate) backend: Arc<LlamaInferenceBackend>,
    pub(crate) model_id: ModelId,
    pub(crate) recipes: Arc<dyn RoleRecipeResolver>,
    pub(crate) role_registry: Arc<dyn RoleRegistry>,
}

/// The shaper's (and, via the `worker` role, each child's) warrant: routes to `model_id`,
/// runs `ReadOnlyNondet` on `exec_class`, no tools/fs/net. Children inherit it. Exposed
/// `pub(crate)` so a deterministic test submits a shaper under the same warrant the
/// `worker` role narrows against. (A `kx/recipes/plan` catalog recipe over this warrant —
/// so `kx invoke` triggers the loop — is a flagged follow-on; today the loop is driven by
/// submitting a shaper Mote via `SubmitRun`.)
pub(crate) fn shaper_warrant(model_id: &ModelId, exec_class: ExecutorClass) -> WarrantSpec {
    WarrantSpec {
        mote_class: MoteClass::ReadOnlyNondet,
        nd_class: MoteClass::ReadOnlyNondet,
        fs_scope: FsScope::empty(),
        net_scope: NetScope::None,
        syscall_profile_ref: ContentRef::from_bytes([0; 32]),
        tool_grants: std::collections::BTreeSet::new(),
        model_route: ModelRoute {
            model_id: model_id.clone(),
            max_input_tokens: MAX_SERVE_N_CTX,
            max_output_tokens: MAX_SERVE_N_CTX,
            max_calls: 8,
        },
        resource_ceiling: ResourceCeiling {
            cpu_milli: 2000,
            mem_bytes: 1 << 21,
            wall_clock_ms: 120_000,
            fd_count: 64,
            disk_bytes: 1 << 20,
        },
        environment_ref: None,
        executor_class: exec_class,
        ..Default::default()
    }
}

/// Build the shaper runtime from a resolved serve model (`#[cfg(feature = "inference")]`).
/// The `worker` role maps (in the recipe allowlist) to a PURE model step and (in the role
/// registry) to the shaper's own warrant — so a lowered child inherits the model route and
/// runs its per-child intent through the same backend.
pub(crate) fn build_shaper_runtime(
    model_id: &ModelId,
    backend: Arc<LlamaInferenceBackend>,
    exec_class: ExecutorClass,
) -> ShaperRuntime {
    let warrant = shaper_warrant(model_id, exec_class);
    let role_registry = InMemoryRoleRegistry::new();
    role_registry.register(
        RoleId(WORKER_ROLE.into()),
        Role {
            name: WORKER_ROLE.into(),
            version: 1,
            spec: warrant,
            description: String::new(),
        },
    );
    ShaperRuntime {
        backend,
        model_id: model_id.clone(),
        recipes: worker_recipes(model_id),
        role_registry: Arc::new(role_registry),
    }
}

/// The role→recipe allowlist a shaper's proposal lowers through (the children's VETTED
/// identity axes — a PURE, no-tool model step routed to the serve model). The `worker`
/// role is the only one provisioned for the demo loop. Shared by `build_shaper_runtime`
/// and the deterministic tests so both lower against the same recipe.
pub(crate) fn worker_recipes(model_id: &ModelId) -> Arc<dyn RoleRecipeResolver> {
    let recipes = InMemoryRoleRecipes::new();
    recipes.register(
        RoleId(WORKER_ROLE.into()),
        RoleRecipe {
            logic_ref: LogicRef::from_bytes([0x77; 32]),
            model_id: model_id.clone(),
            prompt_template_hash: PromptTemplateHash::from_bytes([0u8; 32]),
            tool_contract: BTreeMap::new(),
            capability: ToolName("kx-model".into()),
            nd_class: NdClass::Pure,
            effect_pattern: EffectPattern::IdempotentByConstruction,
            inference_params: InferenceParams::default(),
            deterministic_check: None,
        },
    );
    Arc::new(recipes)
}

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

/// A [`MoteExecutor`] that runs leased **model Motes** through an in-process
/// [`InferenceBackend`] and delegates everything else to `inner` (the PR-9b/R1
/// [`crate::real_exec::RouterExecutor`]).
///
/// Two model arms:
/// - **shaper** (`is_topology_shaper`, PR-2b/T1.1): run the model, decode its proposal
///   FAIL-CLOSED, budget-cap the fan-out, lower it through VETTED recipes, and commit the
///   resulting [`TopologyDecision`] as the shaper's `result_ref` — so the coordinator's
///   materializer derives + dispatches the children. The model-driven agentic loop, live.
/// - **leaf model** (`is_model_mote`, AL1): a greedy completion (recomputable) committed
///   as content bytes — the shaper's children land here, running their per-child intent.
///
/// A leased Mote's resolved Data context: the committed `(parent MoteId, result_ref)`
/// pairs the worker delivers via [`kx_worker::ContextSink`] for F-7 assembly.
type ParentResults = Vec<(MoteId, ContentRef)>;

/// Generic over the backend `B` so a deterministic stub injects in tests; production uses
/// [`LlamaInferenceBackend`]. The whole module is `#[cfg(feature = "inference")]`.
pub(crate) struct ModelRouterExecutor<B: InferenceBackend> {
    inner: Arc<dyn MoteExecutor>,
    backend: Arc<B>,
    store: LocalFsContentStore,
    /// The role→recipe allowlist the shaper arm lowers a model proposal through (SN-8 —
    /// the model names a role, the recipe gives the child's vetted identity axes). `None`
    /// ⇒ no shaper support provisioned (a shaper Mote then fails closed, dead-lettered).
    recipes: Option<Arc<dyn RoleRecipeResolver>>,
    /// F-7 (assemble-into-serve): the per-dispatch context slot the worker fills via
    /// [`kx_worker::ContextSink`] BEFORE each `run` (the frozen `MoteExecutor::run`
    /// carries no snapshot). Keyed by `MoteId` so a stale slot can never leak into the
    /// wrong Mote; consumed (taken) inside `dispatch_model`. The worker runs a lease
    /// batch sequentially on one thread, so the slot is set-then-consumed with no race.
    parent_ctx: Mutex<Option<(MoteId, ParentResults)>>,
}

impl<B: InferenceBackend> ModelRouterExecutor<B> {
    /// Wrap `inner` with the model arms backed by `backend`, publishing completions /
    /// decisions into `store` (the shared store the coordinator verifies). `recipes`
    /// enables the shaper arm (the live agentic loop); `None` leaves AL1 leaf-model
    /// dispatch only.
    pub(crate) fn new(
        inner: Arc<dyn MoteExecutor>,
        backend: Arc<B>,
        store: LocalFsContentStore,
        recipes: Option<Arc<dyn RoleRecipeResolver>>,
    ) -> Self {
        Self {
            inner,
            backend,
            store,
            recipes,
            parent_ctx: Mutex::new(None),
        }
    }

    /// Take this Mote's F-7 context if the worker delivered any for it (and clear the
    /// slot). Returns the parents iff the slot matches `mote_id` — a non-matching or
    /// empty slot yields `None`, so a leaf with no Data context assembles nothing
    /// (byte-identical to pre-F-7). A poisoned lock degrades to "no context" rather
    /// than aborting the dispatch.
    fn take_parent_context(&self, mote_id: MoteId) -> Option<Vec<(MoteId, ContentRef)>> {
        let mut slot = self.parent_ctx.lock().ok()?;
        match slot.as_ref() {
            Some((id, _)) if *id == mote_id => slot.take().map(|(_, parents)| parents),
            _ => None,
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

    /// Run a topology-SHAPER model Mote (PR-2b): the model proposes a fan-out, which is
    /// decoded fail-closed → budget-capped → lowered through vetted recipes → committed as
    /// a [`TopologyDecision`] (the coordinator materializes + dispatches the children).
    ///
    /// The untrusted-model boundary is closed BEFORE anything commits: `decode_loop_proposal`
    /// (size-cap-before-parse, `deny_unknown_fields`, `<think>`-strip, versioned) + the
    /// `SHAPER_MAX_CHILDREN` run-policy cap + `lower_loop_to_topology_decision` (role NAMES
    /// only; logic_ref/nd_class/effect_pattern come from the recipe — SN-8/IMP-5/D70). Any
    /// failure (no recipes, malformed/oversized/over-budget proposal, unknown role) returns
    /// a terminal `MoteExecutorError`, so the worker dead-letters the shaper (F4) and the
    /// run completes past it — never a panic, never raw model bytes committed as a decision.
    fn run_shaper(
        &self,
        mote: &Mote,
        warrant: &WarrantSpec,
    ) -> Result<MoteExecutionResult, MoteExecutorError> {
        let recipes = self
            .recipes
            .as_ref()
            .ok_or_else(|| internal("shaper Mote leased but no planner recipes provisioned"))?;
        // (1-2) Run the model ONCE (greedy — the committed decision is content-addressed,
        // so an identical proposal yields an identical result_ref; replay serves the fact).
        let bytes = self.dispatch_model(mote, warrant)?;
        // (3) Decode FAIL-CLOSED via ENVELOPE DISCRIMINATION (PR-2c-2). The byte-frozen
        // PR-2b `decode_loop_proposal` (a round-0 `loop` envelope) is tried first; on a
        // miss we fall back to `decode_replan_proposal` (a re-plan round's `replan`
        // envelope — its corrected prompt asks for one). The two envelopes are disjoint
        // (distinct top-level keys + `deny_unknown_fields`), so the fallback never
        // reinterprets a well-formed round-0 proposal — round 0 stays byte-frozen + the
        // canonical demo (which always decodes as a loop) is untouched. A `flag_human`
        // round is a TERMINAL stop: the error dead-letters this shaper (F4) and, having
        // no children, the run quiesces — the failed step stays a durable dead-lettered
        // fact (PR-2c-2 passive-terminal scope, D3; the reason is logged).
        let max_bytes = max_plan_bytes(warrant);
        let proposal = match decode_loop_proposal(&bytes, max_bytes) {
            Ok(p) => p,
            Err(loop_err) => match decode_replan_proposal(&bytes, max_bytes) {
                Ok(ReplanProposal::Topology(p)) => p,
                Ok(ReplanProposal::FlagHuman(reason)) => {
                    return Err(internal(&format!(
                        "re-plan escalated to a human (flag_human): {reason}"
                    )));
                }
                Err(replan_err) => {
                    return Err(internal(&format!(
                        "decode proposal failed as both loop ({loop_err}) and replan ({replan_err})"
                    )));
                }
            },
        };
        // (4) Run-policy fan-out cap (after decode's structural cap).
        if proposal.next_steps.len() > SHAPER_MAX_CHILDREN {
            return Err(internal(&format!(
                "decision proposes {} children, exceeding max {SHAPER_MAX_CHILDREN}",
                proposal.next_steps.len()
            )));
        }
        // (5) Lower through vetted recipes (an unregistered role fails closed).
        let decision = lower_loop_to_topology_decision(&proposal, &**recipes)
            .map_err(|e| internal(&format!("lower loop proposal: {e}")))?;
        // (6) Commit the decision as the shaper's result_ref (canonical bincode — the exact
        // bytes the coordinator's materializer decodes; `ContentRef::of(encode) == hash`).
        let result_ref = self
            .store
            .put(&decision.encode())
            .map_err(|e| internal(&format!("content store put (decision): {e}")))?;
        Ok(MoteExecutionResult {
            result_ref,
            started_at_epoch_ms: 0,
            finished_at_epoch_ms: 0,
        })
    }

    /// Greedy-decode the ChatML-wrapped prompt of a model Mote and return the raw
    /// completion bytes (shared by the leaf-model and shaper arms).
    fn dispatch_model(
        &self,
        mote: &Mote,
        warrant: &WarrantSpec,
    ) -> Result<Vec<u8>, MoteExecutorError> {
        let instruction = prompt_from_config(mote)
            .ok_or_else(|| internal("model Mote lost its prompt config key"))?;
        // F-7 (assemble-into-serve): prepend this Mote's resolved upstream context (if
        // the worker delivered any for it) so a corrective/leaf model reasons over its
        // run's source/parent results, not blind. Empty context ⇒ the prompt is
        // byte-identical to pre-F-7. A missing/oversized upstream fails closed.
        let instruction = match self.take_parent_context(mote.id) {
            Some(parents) if !parents.is_empty() => {
                let context =
                    crate::assemble_serve::assemble_from_parent_results(&parents, &self.store)
                        .map_err(|e| internal(&format!("assemble F-7 context: {e}")))?;
                format!("{context}{instruction}")
            }
            _ => instruction,
        };
        let input = InferenceInput::text(chatml(&instruction));
        let params = inference_params_from_mote(mote, warrant)
            .map_err(|e| internal(&format!("inference params: {e}")))?;
        let out = self
            .backend
            .dispatch(&mote.def.model_id, &input, &params, warrant)
            .map_err(|e| internal(&format!("model dispatch: {e}")))?;
        Ok(out.bytes)
    }

    /// Run a leaf model Mote: greedy decode the ChatML-wrapped prompt, publish the
    /// completion bytes, return their content ref.
    fn run_model(
        &self,
        mote: &Mote,
        warrant: &WarrantSpec,
    ) -> Result<MoteExecutionResult, MoteExecutorError> {
        // The committed `result_ref` is the content hash of the completion — a
        // greedy decode ⇒ identical bytes ⇒ identical ref (exactly-once-per-input).
        let bytes = self.dispatch_model(mote, warrant)?;
        let result_ref = self
            .store
            .put(&bytes)
            .map_err(|e| internal(&format!("content store put: {e}")))?;
        Ok(MoteExecutionResult {
            result_ref,
            started_at_epoch_ms: 0,
            finished_at_epoch_ms: 0,
        })
    }
}

impl<B: InferenceBackend> MoteExecutor for ModelRouterExecutor<B> {
    fn run(
        &self,
        mote: &Mote,
        warrant: &WarrantSpec,
        env: Option<Rootfs>,
    ) -> Result<MoteExecutionResult, MoteExecutorError> {
        // Shaper FIRST (a shaper is also a model Mote — has a prompt): the model proposes
        // topology, lowered + committed as a TopologyDecision. Then leaf-model (greedy
        // completion). Everything else → the inner real-body / PURE-echo router.
        if self.is_model_mote(mote) {
            if mote.def.is_topology_shaper {
                self.run_shaper(mote, warrant)
            } else {
                self.run_model(mote, warrant)
            }
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

/// F-7 (assemble-into-serve): the worker hands this executor a leased Mote's resolved
/// Data context BEFORE dispatch (the frozen `MoteExecutor::run` carries no snapshot).
/// The gateway clones ONE `Arc<ModelRouterExecutor>` into both the `MoteExecutor` and
/// the `ContextSink` role, so the slot the worker fills is the slot `dispatch_model`
/// consumes. Setting an empty list clears any stale prior slot for safety.
impl<B: InferenceBackend> kx_worker::ContextSink for ModelRouterExecutor<B> {
    fn set_parent_results(&self, mote_id: MoteId, parents: Vec<(MoteId, ContentRef)>) {
        if let Ok(mut slot) = self.parent_ctx.lock() {
            *slot = Some((mote_id, parents));
        }
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

    // --- shaper arm (PR-2b): deterministic decode→budget→lower→commit + fail-closed ---

    use kx_inference::{InferenceError, InferenceOutput};
    use kx_mote::{
        ConfigVal, GraphPosition, InputDataId, MoteDef, TopologyDecision, MOTE_DEF_SCHEMA_VERSION,
    };
    use smallvec::SmallVec;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    /// A backend that returns a fixed completion (the model's proposed plan) and counts
    /// calls — so a test asserts the proposal was sampled exactly once.
    struct StubBackend {
        reply: Vec<u8>,
        calls: AtomicUsize,
    }
    impl InferenceBackend for StubBackend {
        fn dispatch(
            &self,
            model_id: &ModelId,
            _input: &InferenceInput,
            _params: &kx_mote::InferenceParams,
            _warrant: &WarrantSpec,
        ) -> Result<InferenceOutput, InferenceError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(InferenceOutput {
                bytes: self.reply.clone(),
                output_tokens: 1,
                backend_name: "stub",
                model_id: model_id.clone(),
                elapsed: Duration::from_millis(0),
            })
        }
        fn supports(&self, _model_id: &ModelId) -> bool {
            true
        }
        fn name(&self) -> &'static str {
            "stub"
        }
    }

    /// An inner executor that must never run for a shaper Mote (asserts routing).
    #[derive(Debug)]
    struct NeverInner;
    impl MoteExecutor for NeverInner {
        fn run(
            &self,
            _m: &Mote,
            _w: &WarrantSpec,
            _e: Option<Rootfs>,
        ) -> Result<MoteExecutionResult, MoteExecutorError> {
            Err(internal("inner executor must not run for a shaper Mote"))
        }
        fn supports(&self, _c: ExecutorClass) -> bool {
            true
        }
    }

    fn model_id() -> ModelId {
        ModelId("kx-serve:stub".into())
    }

    /// A shaper Mote: `is_topology_shaper`, a planning prompt, routed to the stub model.
    fn shaper_mote() -> Mote {
        let mut cfg = BTreeMap::new();
        cfg.insert(
            ConfigKey(PROMPT_KEY.to_string()),
            ConfigVal(b"plan the work".to_vec()),
        );
        let def = MoteDef {
            critic_check: None,
            logic_ref: LogicRef::from_bytes([1u8; 32]),
            model_id: model_id(),
            prompt_template_hash: PromptTemplateHash::from_bytes([0u8; 32]),
            tool_contract: BTreeMap::new(),
            nd_class: NdClass::ReadOnlyNondet,
            config_subset: cfg,
            effect_pattern: EffectPattern::IdempotentByConstruction,
            critic_for: None,
            is_topology_shaper: true,
            inference_params: InferenceParams::default(),
            schema_version: MOTE_DEF_SCHEMA_VERSION,
        };
        Mote::new(
            def,
            InputDataId::from_bytes([0u8; 32]),
            GraphPosition(vec![0u8]),
            SmallVec::new(),
        )
    }

    fn executor(
        store: &LocalFsContentStore,
        reply: &[u8],
        recipes: Option<Arc<dyn RoleRecipeResolver>>,
    ) -> (ModelRouterExecutor<StubBackend>, Arc<StubBackend>) {
        let backend = Arc::new(StubBackend {
            reply: reply.to_vec(),
            calls: AtomicUsize::new(0),
        });
        let exec = ModelRouterExecutor::new(
            Arc::new(NeverInner),
            backend.clone(),
            store.clone(),
            recipes,
        );
        (exec, backend)
    }

    const TWO_CHILD_PROPOSAL: &[u8] = br#"{"loop_proposal":{"version":1,"next_steps":[{"role":"worker","intent":"summarize"},{"role":"worker","intent":"translate"}]}}"#;

    #[test]
    fn shaper_arm_commits_a_decoded_lowered_topology_decision() {
        let dir = tempfile::tempdir().unwrap();
        let store = LocalFsContentStore::open(dir.path()).unwrap();
        let (exec, backend) = executor(
            &store,
            TWO_CHILD_PROPOSAL,
            Some(worker_recipes(&model_id())),
        );
        let warrant = shaper_warrant(&model_id(), ExecutorClass::MacOsSandbox);

        let out = exec
            .run(&shaper_mote(), &warrant, None)
            .expect("shaper runs");
        assert_eq!(
            backend.calls.load(Ordering::SeqCst),
            1,
            "the proposal is sampled once"
        );

        // The committed result_ref is a canonical-bincode TopologyDecision (what the
        // coordinator's materializer decodes), carrying the two lowered children + intents.
        let bytes = store.get(&out.result_ref).unwrap();
        let td = TopologyDecision::decode(bytes.as_ref()).expect("result is a TopologyDecision");
        assert_eq!(td.children.len(), 2);
        assert_eq!(
            td.children[0].intent,
            ConfigVal(b"summarize".to_vec()),
            "the model's per-child intent is carried into the descriptor"
        );
        assert_eq!(td.children[1].intent, ConfigVal(b"translate".to_vec()));
        // The result_ref IS the decision's content hash (so commit's D55 guard + the
        // materializer agree on the bytes).
        assert_eq!(out.result_ref, ContentRef::of(&td.encode()));
    }

    // PR-2c-2: a re-plan round's shaper emits a `replan` envelope (its corrected prompt
    // asks for one). The byte-frozen `decode_loop_proposal` rejects it → the gateway falls
    // back to `decode_replan_proposal`, which lowers `next_steps` identically to a loop
    // proposal (envelope discrimination — round 0 stays byte-frozen).
    const REPLAN_NEXT_STEPS: &[u8] =
        br#"{"replan":{"version":1,"next_steps":[{"role":"worker","intent":"retry-with-creds"}]}}"#;
    const REPLAN_FLAG_HUMAN: &[u8] =
        br#"{"replan":{"version":1,"flag_human":"needs a credential I cannot grant"}}"#;

    #[test]
    fn shaper_arm_routes_a_replan_envelope_to_a_topology_decision() {
        let dir = tempfile::tempdir().unwrap();
        let store = LocalFsContentStore::open(dir.path()).unwrap();
        let (exec, backend) =
            executor(&store, REPLAN_NEXT_STEPS, Some(worker_recipes(&model_id())));
        let out = exec
            .run(
                &shaper_mote(),
                &shaper_warrant(&model_id(), ExecutorClass::MacOsSandbox),
                None,
            )
            .expect("a replan-envelope shaper commits a corrective topology");
        assert_eq!(backend.calls.load(Ordering::SeqCst), 1);
        let bytes = store.get(&out.result_ref).unwrap();
        let td = TopologyDecision::decode(bytes.as_ref()).expect("result is a TopologyDecision");
        assert_eq!(td.children.len(), 1, "the corrective step is lowered");
        assert_eq!(
            td.children[0].intent,
            ConfigVal(b"retry-with-creds".to_vec())
        );
        assert_eq!(out.result_ref, ContentRef::of(&td.encode()));
    }

    #[test]
    fn shaper_arm_flag_human_dead_letters_the_shaper() {
        let dir = tempfile::tempdir().unwrap();
        let store = LocalFsContentStore::open(dir.path()).unwrap();
        let (exec, _) = executor(&store, REPLAN_FLAG_HUMAN, Some(worker_recipes(&model_id())));
        let err = exec
            .run(
                &shaper_mote(),
                &shaper_warrant(&model_id(), ExecutorClass::MacOsSandbox),
                None,
            )
            .expect_err("flag_human is a terminal stop (dead-letter)");
        // Terminal (dead-letterable) — the run quiesces, the failed step stays a durable
        // dead-lettered fact; the escalation reason is surfaced in the diagnostic.
        match err {
            MoteExecutorError::Internal { reason, .. } => {
                assert!(reason.contains("flag_human"), "reason surfaced: {reason}");
                assert!(reason.contains("credential"));
            }
            other => panic!("expected a terminal Internal error, got {other:?}"),
        }
    }

    #[test]
    fn shaper_arm_fails_closed_on_malformed_proposal() {
        let dir = tempfile::tempdir().unwrap();
        let store = LocalFsContentStore::open(dir.path()).unwrap();
        let (exec, _) = executor(
            &store,
            b"not json at all",
            Some(worker_recipes(&model_id())),
        );
        let err = exec
            .run(
                &shaper_mote(),
                &shaper_warrant(&model_id(), ExecutorClass::MacOsSandbox),
                None,
            )
            .expect_err("a malformed proposal is refused");
        assert!(
            matches!(err, MoteExecutorError::Internal { .. }),
            "terminal (dead-letterable)"
        );
    }

    #[test]
    fn shaper_arm_fails_closed_over_budget() {
        // SHAPER_MAX_CHILDREN+1 steps → refused after the structural decode, before lowering.
        let steps: Vec<String> = (0..=SHAPER_MAX_CHILDREN)
            .map(|i| format!(r#"{{"role":"worker","intent":"s{i}"}}"#))
            .collect();
        let proposal = format!(
            r#"{{"loop_proposal":{{"version":1,"next_steps":[{}]}}}}"#,
            steps.join(",")
        );
        let dir = tempfile::tempdir().unwrap();
        let store = LocalFsContentStore::open(dir.path()).unwrap();
        let (exec, _) = executor(
            &store,
            proposal.as_bytes(),
            Some(worker_recipes(&model_id())),
        );
        let err = exec
            .run(
                &shaper_mote(),
                &shaper_warrant(&model_id(), ExecutorClass::MacOsSandbox),
                None,
            )
            .expect_err("an over-budget fan-out is refused");
        assert!(matches!(err, MoteExecutorError::Internal { .. }));
    }

    #[test]
    fn shaper_arm_fails_closed_on_unknown_role() {
        let proposal =
            br#"{"loop_proposal":{"version":1,"next_steps":[{"role":"intruder","intent":"x"}]}}"#;
        let dir = tempfile::tempdir().unwrap();
        let store = LocalFsContentStore::open(dir.path()).unwrap();
        // The recipe allowlist only knows `worker`; an unproposed role fails at lowering.
        let (exec, _) = executor(&store, proposal, Some(worker_recipes(&model_id())));
        let err = exec
            .run(
                &shaper_mote(),
                &shaper_warrant(&model_id(), ExecutorClass::MacOsSandbox),
                None,
            )
            .expect_err("an unregistered role is refused");
        assert!(matches!(err, MoteExecutorError::Internal { .. }));
    }

    #[test]
    fn shaper_arm_fails_closed_without_recipes() {
        let dir = tempfile::tempdir().unwrap();
        let store = LocalFsContentStore::open(dir.path()).unwrap();
        let (exec, _) = executor(&store, TWO_CHILD_PROPOSAL, None);
        let err = exec
            .run(
                &shaper_mote(),
                &shaper_warrant(&model_id(), ExecutorClass::MacOsSandbox),
                None,
            )
            .expect_err("a shaper without provisioned recipes is refused");
        assert!(matches!(err, MoteExecutorError::Internal { .. }));
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
