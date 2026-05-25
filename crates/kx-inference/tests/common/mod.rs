//! Shared test fixture: a `FakeBackend` impl of `InferenceBackend`.
//!
//! Each test file `include!`s this so the fixture lives in exactly one place
//! and stays consistent across the test suite. The fake is deliberately
//! minimal — it does NOT load llama.cpp — so unit tests can run on every
//! host (including hosts without GGUF model files).

#![allow(
    dead_code,
    unreachable_pub,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::pedantic
)]

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use kx_inference::{
    InferenceBackend, InferenceError, InferenceInput, InferenceOutput, InferenceParams,
};
use kx_mote::ModelId;
use kx_warrant::WarrantSpec;

/// Test backend that returns a canned output for any registered model.
#[derive(Debug)]
pub struct FakeBackend {
    name: &'static str,
    supported: HashSet<ModelId>,
    /// Canned response bytes for any successful dispatch.
    response_bytes: Vec<u8>,
    /// Atomic invocation counter so tests can assert dispatch was called.
    pub dispatch_calls: Arc<AtomicU64>,
    /// Atomic batch_dispatch counter (separate from the per-item count).
    pub batch_calls: Arc<AtomicU64>,
}

impl FakeBackend {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            supported: HashSet::new(),
            response_bytes: b"FAKE OUTPUT".to_vec(),
            dispatch_calls: Arc::new(AtomicU64::new(0)),
            batch_calls: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn with_model(mut self, id: ModelId) -> Self {
        self.supported.insert(id);
        self
    }

    pub fn with_response(mut self, bytes: Vec<u8>) -> Self {
        self.response_bytes = bytes;
        self
    }

    pub fn dispatch_count(&self) -> u64 {
        self.dispatch_calls.load(Ordering::SeqCst)
    }

    pub fn batch_count(&self) -> u64 {
        self.batch_calls.load(Ordering::SeqCst)
    }
}

impl InferenceBackend for FakeBackend {
    fn dispatch(
        &self,
        model_id: &ModelId,
        input: &InferenceInput,
        params: &InferenceParams,
        _warrant: &WarrantSpec,
    ) -> Result<InferenceOutput, InferenceError> {
        self.dispatch_calls.fetch_add(1, Ordering::SeqCst);

        // Honour the same reservation gates as a real backend.
        if matches!(input, InferenceInput::Multimodal { .. }) {
            return Err(InferenceError::Unsupported {
                reason: "fake backend mirrors v0.1 multimodal reservation",
            });
        }
        if params.grammar.is_some() {
            return Err(InferenceError::Unsupported {
                reason: "fake backend mirrors v0.1 grammar reservation",
            });
        }
        if !self.supported.contains(model_id) {
            return Err(InferenceError::ModelNotFound {
                model_id: model_id.0.clone(),
            });
        }
        Ok(InferenceOutput {
            bytes: self.response_bytes.clone(),
            output_tokens: 1,
            backend_name: self.name,
            model_id: model_id.clone(),
            elapsed: Duration::from_millis(1),
        })
    }

    fn supports(&self, model_id: &ModelId) -> bool {
        self.supported.contains(model_id)
    }

    fn name(&self) -> &'static str {
        self.name
    }

    fn batch_dispatch(
        &self,
        items: &[kx_inference::BatchItem<'_>],
    ) -> Vec<Result<InferenceOutput, InferenceError>> {
        // Count the batch call (separately from individual dispatches —
        // the default impl forwards to dispatch, so per-item counters
        // still tick).
        self.batch_calls.fetch_add(1, Ordering::SeqCst);
        items
            .iter()
            .map(|item| self.dispatch(item.model_id, item.input, item.params, item.warrant))
            .collect()
    }
}
