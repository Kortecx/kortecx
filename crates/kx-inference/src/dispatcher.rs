// The `Dispatcher` — D35's "router is a dispatcher with capability
// enforcement, NOT a model selector."
//
// Dispatch flow (per 02-crate-specs.md:467-478, locked):
//   1. kx-model-validator::check(provided, required) →
//      TypeOk | DegradedSubtype | TypeError(refuse).
//   2. kx-memoizer::lookup(mote, snapshot) →
//      Some(CacheHit) (return; broker handles WM re-dispatch) | None.
//   3. On miss: route to the registered backend that `supports(model_id)`,
//      construct InferenceParams from warrant, dispatch with
//      `wall_clock_ms` timeout.
//   4. Return DispatchOutcome::Fresh(output) to the caller (executor
//      stages the result + commits).
//
// NO model-selection logic anywhere. The dispatcher takes
// `warrant.model_route.model_id` as the canonical name; if no backend
// supports it the call fails with `ModelNotFound`. The dispatcher never
// substitutes a different model.

use std::sync::Arc;

use kx_context_assembler::AssembledContext;
use kx_memoizer::CacheHit;
use kx_model_validator::{check, ModelRegistry, RequiredCapabilities};
use kx_mote::Mote;
use kx_projection::Snapshot;
use kx_warrant::WarrantSpec;

use crate::backend::InferenceBackend;
use crate::types::{InferenceError, InferenceInput, InferenceOutput, InferenceParams};

/// What `Dispatcher::dispatch_mote` returns when it succeeds.
///
/// `CacheHit` means the memoizer found a prior committed result for an
/// identity-equivalent Mote (exact cryptographic equality per SN-8);
/// the executor either returns it as-is (`Pure` / `ReadOnlyNondet`) or
/// uses it as the canonical result while the broker re-dispatches the
/// world-mutating effect (`WorldMutating { redispatch_effect: true }`).
///
/// `Fresh` means the backend produced a new inference result; the
/// executor stages it through `kx-content::ContentStore` and the
/// resulting `ContentRef` becomes `Committed.result_ref`.
#[derive(Debug, Clone)]
pub enum DispatchOutcome {
    /// Cached identity-match.
    CacheHit(CacheHit),
    /// Fresh backend output.
    Fresh(InferenceOutput),
}

/// Configuration for a `Dispatcher`.
#[derive(Clone)]
pub struct DispatcherConfig {
    /// Registry the dispatcher queries to obtain `ProvidedCapabilities`
    /// at bind-time validation. Provided as an `Arc<dyn ...>` so the
    /// dispatcher stays dyn-compatible across thread boundaries.
    pub model_registry: Arc<dyn ModelRegistry>,
}

impl std::fmt::Debug for DispatcherConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DispatcherConfig")
            .field("model_registry", &"<dyn ModelRegistry>")
            .finish()
    }
}

/// The router that ties together model-validator + memoizer + warrant
/// scope + a backend set.
///
/// # Examples
///
/// ```
/// use kx_inference::{Dispatcher, DispatcherConfig, LlamaInferenceBackend};
/// use kx_model_validator::InMemoryModelRegistry;
/// use std::sync::Arc;
///
/// let mut dispatcher = Dispatcher::new(DispatcherConfig {
///     model_registry: Arc::new(InMemoryModelRegistry::new()),
/// });
/// dispatcher.register_backend(Arc::new(LlamaInferenceBackend::new()));
/// assert_eq!(dispatcher.backend_count(), 1);
/// ```
#[derive(Clone)]
pub struct Dispatcher {
    backends: Vec<Arc<dyn InferenceBackend>>,
    config: DispatcherConfig,
}

impl std::fmt::Debug for Dispatcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Dispatcher")
            .field("backend_count", &self.backends.len())
            .field("config", &self.config)
            .finish()
    }
}

impl Dispatcher {
    /// Construct an empty dispatcher. Register backends with
    /// `register_backend` before calling `dispatch_mote`.
    #[must_use]
    pub fn new(config: DispatcherConfig) -> Self {
        Self {
            backends: Vec::new(),
            config,
        }
    }

    /// Add a backend to the dispatcher's routing set.
    ///
    /// Backends are queried in insertion order; the first to return
    /// `true` from `supports(model_id)` wins.
    pub fn register_backend(&mut self, backend: Arc<dyn InferenceBackend>) {
        self.backends.push(backend);
    }

    /// Number of registered backends.
    #[must_use]
    pub fn backend_count(&self) -> usize {
        self.backends.len()
    }

    /// Run the full dispatch protocol for a Mote.
    ///
    /// See module docs for the four-step flow.
    ///
    /// # Errors
    ///
    /// Returns `InferenceError::WarrantDeniesModel` if `mote.def.model_id`
    /// disagrees with `warrant.model_route.model_id`,
    /// `InferenceError::ModelNotFound` when no backend supports the model
    /// (or the registry has no entry), `InferenceError::ModelValidation`
    /// on `ValidatorOutcome::TypeError`, and any backend error verbatim.
    pub fn dispatch_mote(
        &self,
        mote: &Mote,
        warrant: &WarrantSpec,
        context: &AssembledContext,
        required_capabilities: &RequiredCapabilities,
        snapshot: &Snapshot,
    ) -> Result<DispatchOutcome, InferenceError> {
        // ---- 0. Warrant model-route check (cheapest; fail fast) -----------
        if mote.def.model_id != warrant.model_route.model_id {
            return Err(InferenceError::WarrantDeniesModel {
                model_id: mote.def.model_id.0.clone(),
                route: warrant.model_route.model_id.0.clone(),
            });
        }

        // ---- 1. Bind-time validator (D29) ---------------------------------
        let provided = self
            .config
            .model_registry
            .lookup(&mote.def.model_id)
            .ok_or_else(|| InferenceError::ModelNotFound {
                model_id: mote.def.model_id.0.clone(),
            })?;

        let outcome = check(&provided, required_capabilities);
        if outcome.is_type_error() {
            return Err(InferenceError::ModelValidation {
                message: format!("{outcome:?}"),
            });
        }
        // `TypeOk` and `DegradedSubtype` both proceed; the latter carries
        // a soft-flag the caller may surface in logs.

        // ---- 2. Memoizer lookup (D33) -------------------------------------
        if let Some(hit) = kx_memoizer::lookup(mote, snapshot) {
            return Ok(DispatchOutcome::CacheHit(hit));
        }

        // ---- 3. Fresh dispatch --------------------------------------------
        let input = InferenceInput::Text(serialize_context(context));
        let params = InferenceParams::from_warrant(warrant);

        let backend = self
            .backends
            .iter()
            .find(|b| b.supports(&mote.def.model_id))
            .ok_or_else(|| InferenceError::ModelNotFound {
                model_id: mote.def.model_id.0.clone(),
            })?;

        let output = backend.dispatch(&mote.def.model_id, &input, &params, warrant)?;
        Ok(DispatchOutcome::Fresh(output))
    }
}

/// Serialise an `AssembledContext` into a single UTF-8 prompt string.
///
/// v0.1 convention: each item rendered as `LABEL:\n<utf8-lossy bytes>\n\n`,
/// concatenated in `AssembledContext.items` order (which is deterministic
/// per D33). Workflow authors who need chat-template structure embed it
/// in their `AssembledContext` at build time (e.g., a preceding `system:`
/// item carries `<|im_start|>system\n...`).
///
/// The dispatcher is the right place for this convention because the
/// `InferenceBackend` trait's `Text(String)` input is the canonical
/// stable seam — making each backend re-implement the same default
/// would invite drift.
fn serialize_context(ctx: &AssembledContext) -> String {
    let mut out = String::new();
    for item in &ctx.items {
        out.push_str(&item.label);
        out.push_str(":\n");
        out.push_str(&String::from_utf8_lossy(&item.bytes));
        out.push_str("\n\n");
    }
    out
}
