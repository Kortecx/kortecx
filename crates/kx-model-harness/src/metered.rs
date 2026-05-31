//! [`MeteredBackend`] — an [`InferenceBackend`] decorator that counts dispatches.
//!
//! The instrument for two campaign invariants:
//! - **serve-not-re-sample under crash (row C):** the committed-fact count of
//!   model dispatches in the recovery process must be `0` for an already-committed
//!   Mote (the model was sampled once, never re-sampled).
//! - **reuse the recipe, never the result (row E):** a memoizer hit re-serves the
//!   committed `result_ref` and the dispatch count does NOT increase.
//!
//! It rides the [`InferenceBackend`] trait verbatim — the trait carries no
//! in-process assumptions, so a counting wrapper is a pure decorator (D28 seam).

use std::sync::atomic::{AtomicU64, Ordering};

use kx_inference::{
    InferenceBackend, InferenceError, InferenceInput, InferenceOutput, InferenceParams,
};
use kx_mote::ModelId;
use kx_warrant::WarrantSpec;

/// Wraps an [`InferenceBackend`], counting every `dispatch` call. Construct once
/// and share via `Arc` between the [`crate::ModelExecutor`] (PURE/greedy path)
/// and the [`crate::ModelBroker`] (ROND/WM path) so the count aggregates every
/// model invocation in one process.
#[derive(Debug)]
pub struct MeteredBackend<B: InferenceBackend> {
    inner: B,
    dispatch_calls: AtomicU64,
}

impl<B: InferenceBackend> MeteredBackend<B> {
    /// Wrap `inner` with a zeroed dispatch counter.
    #[must_use]
    pub fn new(inner: B) -> Self {
        Self {
            inner,
            dispatch_calls: AtomicU64::new(0),
        }
    }

    /// Total number of `dispatch` calls observed so far (this process).
    #[must_use]
    pub fn calls(&self) -> u64 {
        self.dispatch_calls.load(Ordering::SeqCst)
    }
}

impl<B: InferenceBackend> InferenceBackend for MeteredBackend<B> {
    fn dispatch(
        &self,
        model_id: &ModelId,
        input: &InferenceInput,
        params: &InferenceParams,
        warrant: &WarrantSpec,
    ) -> Result<InferenceOutput, InferenceError> {
        self.dispatch_calls.fetch_add(1, Ordering::SeqCst);
        self.inner.dispatch(model_id, input, params, warrant)
    }

    fn supports(&self, model_id: &ModelId) -> bool {
        self.inner.supports(model_id)
    }

    fn name(&self) -> &'static str {
        self.inner.name()
    }
}
