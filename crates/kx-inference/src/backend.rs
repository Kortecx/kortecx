// The `InferenceBackend` trait — the seam every model-serving
// implementation rides.
//
// CRITICAL (D35 + D28): the trait MUST carry NO in-process or
// in-binary assumptions in its signature. Triton (out-of-process),
// vLLM (out-of-process), remote APIs (cross-network), and the OSS
// in-process llama.cpp backend must all fit behind this trait WITHOUT
// trait change. If you find yourself wanting to add a method that
// only makes sense for an in-process backend, that method belongs on
// a more specific type, not on this trait.

use kx_mote::ModelId;
use kx_warrant::WarrantSpec;

use crate::types::{
    EmbeddingOutput, EmbeddingPooling, InferenceError, InferenceInput, InferenceOutput,
    InferenceParams,
};

/// A single dispatch request, packaged as a reference-tuple so
/// `batch_dispatch` doesn't force clones.
#[derive(Debug)]
pub struct BatchItem<'a> {
    /// Model identity for this item.
    pub model_id: &'a ModelId,
    /// Input prompt / multimodal package.
    pub input: &'a InferenceInput,
    /// Sampling + token-limit parameters.
    pub params: &'a InferenceParams,
    /// Warrant whose scope the backend enforces.
    pub warrant: &'a WarrantSpec,
}

/// The model-serving seam.
///
/// Every backend impl (OSS local `LlamaInferenceBackend`, future
/// `kx-cloud-inference-*` crates for vLLM / Triton / remote APIs)
/// implements this trait. The dispatcher (`Dispatcher`) holds a set
/// of backends and routes each call to the one that `supports` the
/// requested `ModelId`.
///
/// **Dyn-compatible**: methods have no generics, no `Self` in the
/// return type, and no associated types. The dispatcher holds
/// `Box<dyn InferenceBackend>` or `Arc<dyn InferenceBackend>` so a
/// runtime-configurable backend set is supported.
pub trait InferenceBackend: Send + Sync {
    /// Run a single inference request.
    ///
    /// The backend MUST honour `warrant.resource_ceiling.wall_clock_ms`
    /// as the timeout — exceeding it returns
    /// `Err(InferenceError::Timeout)`.
    ///
    /// The backend MUST return `Err(InferenceError::Unsupported)` on
    /// any input variant or param it does not implement. Future
    /// variants (`InferenceInput::Multimodal`,
    /// `InferenceParams.grammar = Some(_)`) are deliberate
    /// not-implemented-yet seams in OSS v0.1.
    ///
    /// # Errors
    ///
    /// Returns `InferenceError::Unsupported` for the reserved v0.1
    /// variants, `InferenceError::WarrantDeniesModel` when the model
    /// id does not match the warrant's route, `ScopeViolation` when
    /// params overshoot the warrant's ceilings, `ModelNotFound` when
    /// the backend cannot serve the requested model, `Timeout` on
    /// wall-clock expiry, and `BackendFailure` on any backend-internal
    /// failure.
    fn dispatch(
        &self,
        model_id: &ModelId,
        input: &InferenceInput,
        params: &InferenceParams,
        warrant: &WarrantSpec,
    ) -> Result<InferenceOutput, InferenceError>;

    /// Whether this backend can serve the named model.
    ///
    /// The dispatcher uses this to choose which backend to route a
    /// given `dispatch_mote` call to. Backends MUST return `false` for
    /// model ids they have not been configured with (so the dispatcher
    /// can probe a backend set deterministically).
    fn supports(&self, model_id: &ModelId) -> bool;

    /// Backend identity string for diagnostics + audit-trail logging.
    /// Echoed back in `InferenceOutput.backend_name`.
    fn name(&self) -> &'static str;

    /// Batched dispatch.
    ///
    /// Default impl calls `dispatch` per item. Future cloud backends
    /// (vLLM, Triton) override this to exploit true server-side
    /// batching. The default exists so every backend can be used in a
    /// batched context without re-implementation; the seam is what's
    /// load-bearing (D35).
    fn batch_dispatch(
        &self,
        items: &[BatchItem<'_>],
    ) -> Vec<Result<InferenceOutput, InferenceError>> {
        items
            .iter()
            .map(|item| self.dispatch(item.model_id, item.input, item.params, item.warrant))
            .collect()
    }
}

/// The **embedding capability seam** (DP1 / T2.1, the D108.2-sanctioned
/// `kx-inference` capability addition).
///
/// A *separate* trait — NOT a method on [`InferenceBackend`] — so that trait's
/// source stays byte-stable and every existing backend remains valid without
/// edit. A backend that can produce embeddings opts in by also implementing
/// this; the default methods return `Err(Unsupported)`, so the seam is exercised
/// (and degrades gracefully) even for backends that don't.
///
/// The `: InferenceBackend` supertrait bound keeps the dispatcher's existing
/// `dyn InferenceBackend` set unaffected: a caller that wants embeddings holds an
/// `&dyn EmbeddingBackend` (or the concrete backend) and calls
/// [`Self::dispatch_embedding`] directly — embeddings are an ingest-time act, not
/// a scheduler/executor `dispatch_mote` path, so the frozen control flow is
/// untouched.
pub trait EmbeddingBackend: InferenceBackend {
    /// Embed `text` for `model_id` under `pooling`, enforcing the warrant's
    /// model route (the backend MUST refuse a model the warrant did not
    /// authorise, exactly like [`InferenceBackend::dispatch`]).
    ///
    /// # Errors
    /// Returns [`InferenceError::Unsupported`] by default (a backend without the
    /// embedding capability). A capable backend returns
    /// [`InferenceError::WarrantDeniesModel`] when the model id does not match
    /// the warrant route, [`InferenceError::ModelNotFound`] when it cannot serve
    /// the model, [`InferenceError::Timeout`] on wall-clock expiry, and
    /// [`InferenceError::BackendFailure`] on any backend-internal failure (incl.
    /// an empty / untokenizable input).
    fn dispatch_embedding(
        &self,
        model_id: &ModelId,
        text: &str,
        pooling: EmbeddingPooling,
        warrant: &WarrantSpec,
    ) -> Result<EmbeddingOutput, InferenceError> {
        let _ = (model_id, text, pooling, warrant);
        Err(InferenceError::Unsupported {
            reason: "embedding not supported by this backend",
        })
    }

    /// Embed a batch of texts under one `pooling` + `model_id`. The default maps
    /// [`Self::dispatch_embedding`] per item; a future backend overrides this to
    /// exploit a single multi-sequence decode (the seam, mirroring
    /// [`InferenceBackend::batch_dispatch`], is what's load-bearing).
    fn dispatch_embedding_batch(
        &self,
        model_id: &ModelId,
        texts: &[&str],
        pooling: EmbeddingPooling,
        warrant: &WarrantSpec,
    ) -> Vec<Result<EmbeddingOutput, InferenceError>> {
        texts
            .iter()
            .map(|text| self.dispatch_embedding(model_id, text, pooling, warrant))
            .collect()
    }
}
