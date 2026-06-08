//! DP1 — the `EmbeddingBackend` capability seam (model-free).
//!
//! Proves the seam WITHOUT a real GGUF, mirroring `model_cache.rs`:
//!   - a backend that does NOT implement embeddings gets the default
//!     `Err(Unsupported)` (so the seam degrades gracefully);
//!   - a backend that DOES implement it returns an `EmbeddingOutput` and
//!     enforces the warrant route (the SN-8 / D35 authorize-before-work rule);
//!   - the real `LlamaInferenceBackend` wires `dispatch_embedding` through the
//!     warrant gate → resolver → owner-thread cache, reaching the load-failure /
//!     not-found / denied paths without a real model.
//!
//! Real-model embedding numerics are proven by `kx-llamacpp`'s
//! `smoke_embed_with_pooling_matches_mean_and_is_total` (CI smoke-test-with-model)
//! and exercised end-to-end by the DP2 RAG harness.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use kx_content::ContentRef;
use kx_inference::{
    EmbeddingBackend, EmbeddingOutput, EmbeddingPooling, InferenceBackend, InferenceError,
    InferenceInput, InferenceOutput, InferenceParams,
};
use kx_mote::ModelId;
use kx_warrant::{
    ExecutorClass, FsScope, ModelRoute, MoteClass, NetScope, ResourceCeiling, WarrantSpec,
};

fn warrant(model_id: ModelId) -> WarrantSpec {
    WarrantSpec {
        mote_class: MoteClass::Pure,
        nd_class: MoteClass::Pure,
        fs_scope: FsScope {
            mounts: BTreeMap::new(),
        },
        net_scope: NetScope::None,
        syscall_profile_ref: ContentRef([0u8; 32]),
        tool_grants: BTreeSet::new(),
        model_route: ModelRoute {
            model_id,
            max_input_tokens: 2048,
            max_output_tokens: 2048,
            max_calls: 100,
        },
        resource_ceiling: ResourceCeiling {
            cpu_milli: 1000,
            mem_bytes: 1 << 30,
            wall_clock_ms: 60_000,
            fd_count: 64,
            disk_bytes: 1 << 28,
        },
        environment_ref: None,
        executor_class: ExecutorClass::Bwrap,
        ..Default::default()
    }
}

/// A completion-only backend that does NOT opt into embeddings: it implements
/// `InferenceBackend` and takes `EmbeddingBackend`'s DEFAULT methods.
struct NoEmbedBackend;

impl InferenceBackend for NoEmbedBackend {
    fn dispatch(
        &self,
        _model_id: &ModelId,
        _input: &InferenceInput,
        _params: &InferenceParams,
        _warrant: &WarrantSpec,
    ) -> Result<InferenceOutput, InferenceError> {
        Err(InferenceError::Unsupported {
            reason: "stub: no completion",
        })
    }
    fn supports(&self, _model_id: &ModelId) -> bool {
        true
    }
    fn name(&self) -> &'static str {
        "no-embed-backend"
    }
}
impl EmbeddingBackend for NoEmbedBackend {}

/// A backend that DOES embed: it enforces the warrant route exactly like the
/// real backend, then returns a deterministic canned vector derived from the
/// text length (so the test can assert the value plumbs through).
struct CapableEmbedBackend;

impl InferenceBackend for CapableEmbedBackend {
    fn dispatch(
        &self,
        _model_id: &ModelId,
        _input: &InferenceInput,
        _params: &InferenceParams,
        _warrant: &WarrantSpec,
    ) -> Result<InferenceOutput, InferenceError> {
        Err(InferenceError::Unsupported {
            reason: "stub: embeddings only",
        })
    }
    fn supports(&self, _model_id: &ModelId) -> bool {
        true
    }
    fn name(&self) -> &'static str {
        "capable-embed-backend"
    }
}
impl EmbeddingBackend for CapableEmbedBackend {
    fn dispatch_embedding(
        &self,
        model_id: &ModelId,
        text: &str,
        _pooling: EmbeddingPooling,
        warrant: &WarrantSpec,
    ) -> Result<EmbeddingOutput, InferenceError> {
        if model_id != &warrant.model_route.model_id {
            return Err(InferenceError::WarrantDeniesModel {
                model_id: model_id.0.clone(),
                route: warrant.model_route.model_id.0.clone(),
            });
        }
        let vector = vec![text.len() as f32, 0.5, -0.5];
        Ok(EmbeddingOutput {
            vector,
            dim: 3,
            backend_name: "capable-embed-backend",
            model_id: model_id.clone(),
            elapsed: Duration::from_millis(1),
        })
    }
}

#[test]
fn default_embedding_capability_is_unsupported() {
    let id = ModelId("m".into());
    let w = warrant(id.clone());
    let err = NoEmbedBackend
        .dispatch_embedding(&id, "hello", EmbeddingPooling::Mean, &w)
        .expect_err("a non-embedding backend must report Unsupported");
    assert!(matches!(err, InferenceError::Unsupported { .. }));
}

#[test]
fn default_embedding_batch_is_all_unsupported() {
    let id = ModelId("m".into());
    let w = warrant(id.clone());
    let out =
        NoEmbedBackend.dispatch_embedding_batch(&id, &["a", "b", "c"], EmbeddingPooling::Mean, &w);
    assert_eq!(out.len(), 3);
    assert!(out
        .iter()
        .all(|r| matches!(r, Err(InferenceError::Unsupported { .. }))));
}

#[test]
fn capable_backend_returns_embedding() {
    let id = ModelId("m".into());
    let w = warrant(id.clone());
    let out = CapableEmbedBackend
        .dispatch_embedding(&id, "hello", EmbeddingPooling::Mean, &w)
        .expect("capable backend embeds");
    assert_eq!(out.dim, 3);
    assert_eq!(out.vector.len(), 3);
    assert_eq!(out.vector[0], 5.0, "text len plumbs through");
    assert_eq!(out.model_id, id);
}

#[test]
fn capable_backend_enforces_warrant_route() {
    let asked = ModelId("asked".into());
    // Warrant authorises a DIFFERENT model than the one requested.
    let w = warrant(ModelId("authorised".into()));
    let err = CapableEmbedBackend
        .dispatch_embedding(&asked, "hello", EmbeddingPooling::Mean, &w)
        .expect_err("route mismatch must be denied");
    assert!(matches!(err, InferenceError::WarrantDeniesModel { .. }));
}

#[test]
fn capable_backend_batch_maps_per_item() {
    let id = ModelId("m".into());
    let w = warrant(id.clone());
    let out = CapableEmbedBackend.dispatch_embedding_batch(
        &id,
        &["x", "yy", "zzz"],
        EmbeddingPooling::Cls,
        &w,
    );
    let lens: Vec<f32> = out
        .into_iter()
        .map(|r| r.expect("each embeds").vector[0])
        .collect();
    assert_eq!(lens, vec![1.0, 2.0, 3.0], "per-item text lengths");
}

#[test]
fn embedding_pooling_default_is_mean() {
    assert_eq!(EmbeddingPooling::default(), EmbeddingPooling::Mean);
}

#[test]
fn text_for_embedding_reports_its_text_len() {
    let input = InferenceInput::TextForEmbedding {
        text: "hello".into(),
        pooling: EmbeddingPooling::Mean,
    };
    assert_eq!(input.text_len(), 5);
}

// ---- Real backend wiring (llama.cpp feature; no real GGUF needed) -----------

#[cfg(feature = "llamacpp")]
mod real_backend {
    use super::{warrant, EmbeddingPooling};
    use kx_inference::{EmbeddingBackend, InferenceError, LlamaInferenceBackend};
    use kx_mote::ModelId;
    use std::path::PathBuf;

    #[test]
    fn dispatch_embedding_denies_unauthorised_route() {
        let asked = ModelId("asked".into());
        let backend =
            LlamaInferenceBackend::with_model(asked.clone(), PathBuf::from("/tmp/whatever.gguf"));
        // Warrant routes to a different model: the gate fires BEFORE any FFI.
        let w = warrant(ModelId("authorised".into()));
        let err = backend
            .dispatch_embedding(&asked, "hi", EmbeddingPooling::Mean, &w)
            .expect_err("unauthorised route must be denied before touching the model");
        assert!(matches!(err, InferenceError::WarrantDeniesModel { .. }));
    }

    #[test]
    fn dispatch_embedding_reports_model_not_found() {
        let id = ModelId("missing".into());
        // A backend with NO registered models; the warrant authorises `id`.
        let backend = LlamaInferenceBackend::new();
        let w = warrant(id.clone());
        let err = backend
            .dispatch_embedding(&id, "hi", EmbeddingPooling::Mean, &w)
            .expect_err("unresolved model must be ModelNotFound");
        assert!(matches!(err, InferenceError::ModelNotFound { .. }));
    }

    #[test]
    fn dispatch_embedding_wires_through_to_a_typed_load_failure() {
        let id = ModelId("m".into());
        // Resolvable id, but the path does not exist: the request must travel the
        // full path (warrant gate → resolve → owner-thread cache → Model::load)
        // and surface a typed BackendFailure, never a panic or hang.
        let backend = LlamaInferenceBackend::with_model(
            id.clone(),
            PathBuf::from("/nonexistent/definitely/not/a/model.gguf"),
        );
        let w = warrant(id.clone());
        let err = backend
            .dispatch_embedding(&id, "hi", EmbeddingPooling::Mean, &w)
            .expect_err("a missing model file must be a typed BackendFailure");
        assert!(matches!(err, InferenceError::BackendFailure { .. }));
    }
}
