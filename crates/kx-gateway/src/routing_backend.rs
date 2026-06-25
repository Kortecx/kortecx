//! The host-owned [`RoutingBackend`] — one [`InferenceBackend`] over N serve engines
//! (the in-process llama.cpp backend and/or an Ollama daemon), promoting the
//! `Dispatcher`'s "first backend whose `supports()` is true wins" rule
//! (`kx-inference/src/dispatcher.rs`) into the serve path WITHOUT touching that frozen
//! file. It also routes the lifecycle (warm/evict/resident) to the engine that owns
//! a model, so `kx models load/offload` + the catalog `loaded` flag work uniformly
//! across engines.
//!
//! The seam is [`ServeEngine`]: an [`InferenceBackend`] that also exposes the
//! lifecycle the trait lacks. Both concrete backends implement it directly (the
//! orphan rule permits a LOCAL trait impl for a foreign type), so no adapter newtype
//! is needed. The whole module is `#[cfg(feature = "serve-engine")]`; the
//! `LlamaInferenceBackend` impl is additionally `#[cfg(feature = "inference")]`.

use std::sync::Arc;

use kx_inference::{
    EmbeddingBackend, EmbeddingOutput, EmbeddingPooling, InferenceBackend, InferenceError,
    InferenceInput, InferenceOutput, InferenceParams, TokenSink,
};
use kx_mote::ModelId;
use kx_warrant::WarrantSpec;

use crate::model_lifecycle::ModelEngine;
use crate::models::ModelResidency;

/// The routing backend's audit identity (the per-member `backend_name` is preserved
/// in each [`InferenceOutput`]; this only names the router itself).
const ROUTING_NAME: &str = "kx-routing";

/// A serve engine: an [`InferenceBackend`] that also owns its model lifecycle.
pub(crate) trait ServeEngine: InferenceBackend {
    /// Warm `model_id` into the engine's memory.
    fn warm(&self, model_id: &str) -> Result<(), String>;
    /// Evict `model_id`; `Ok(true)` iff it was resident.
    fn evict(&self, model_id: &str) -> Result<bool, String>;
    /// The model ids currently resident in this engine.
    fn resident_ids(&self) -> Vec<String>;

    /// Embed `text` for `model_id` if this engine has the embedding capability.
    ///
    /// The default returns [`InferenceError::Unsupported`] (an engine without the
    /// [`EmbeddingBackend`] capability); a capable engine overrides this to delegate
    /// to its own [`EmbeddingBackend::dispatch_embedding`]. This keeps the embedding
    /// capability OFF the dyn-held [`InferenceBackend`] surface (the frozen
    /// `Dispatcher` is untouched) while letting [`RoutingBackend`] route an embed call
    /// to the owning engine — datasets server-embed at parity across engines (PR-B).
    fn embed(
        &self,
        model_id: &ModelId,
        text: &str,
        pooling: EmbeddingPooling,
        warrant: &WarrantSpec,
    ) -> Result<EmbeddingOutput, InferenceError> {
        let _ = (model_id, text, pooling, warrant);
        Err(InferenceError::Unsupported {
            reason: "embedding not supported by this serve engine",
        })
    }
}

#[cfg(feature = "inference")]
impl ServeEngine for kx_inference::LlamaInferenceBackend {
    fn warm(&self, model_id: &str) -> Result<(), String> {
        kx_inference::LlamaInferenceBackend::warm(self, &ModelId(model_id.to_string()))
            .map_err(|e| e.to_string())
    }
    fn evict(&self, model_id: &str) -> Result<bool, String> {
        kx_inference::LlamaInferenceBackend::evict(self, &ModelId(model_id.to_string()))
            .map_err(|e| e.to_string())
    }
    fn resident_ids(&self) -> Vec<String> {
        kx_inference::LlamaInferenceBackend::resident(self)
            .into_iter()
            .map(|m| m.0)
            .collect()
    }
    fn embed(
        &self,
        model_id: &ModelId,
        text: &str,
        pooling: EmbeddingPooling,
        warrant: &WarrantSpec,
    ) -> Result<EmbeddingOutput, InferenceError> {
        kx_inference::EmbeddingBackend::dispatch_embedding(self, model_id, text, pooling, warrant)
    }
}

impl ServeEngine for kx_ollama::OllamaBackend {
    fn warm(&self, model_id: &str) -> Result<(), String> {
        kx_ollama::OllamaBackend::warm(self, &ModelId(model_id.to_string()))
            .map_err(|e| e.to_string())
    }
    fn evict(&self, model_id: &str) -> Result<bool, String> {
        kx_ollama::OllamaBackend::evict(self, &ModelId(model_id.to_string()))
            .map_err(|e| e.to_string())
    }
    fn resident_ids(&self) -> Vec<String> {
        kx_ollama::OllamaBackend::resident(self)
            .into_iter()
            .map(|m| m.0)
            .collect()
    }
    fn embed(
        &self,
        model_id: &ModelId,
        text: &str,
        pooling: EmbeddingPooling,
        warrant: &WarrantSpec,
    ) -> Result<EmbeddingOutput, InferenceError> {
        kx_inference::EmbeddingBackend::dispatch_embedding(self, model_id, text, pooling, warrant)
    }
}

/// One [`InferenceBackend`] (+ lifecycle) over an ordered set of serve engines. A
/// dispatch / lifecycle call routes to the FIRST member whose `supports(model_id)`
/// is true; with no match it fails closed with [`InferenceError::ModelNotFound`]
/// (the `Dispatcher`'s rule). Member order is the engine registration order set by
/// the host (llama first when a GGUF is configured, Ollama second).
pub(crate) struct RoutingBackend {
    engines: Vec<Arc<dyn ServeEngine>>,
}

impl std::fmt::Debug for RoutingBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RoutingBackend")
            .field("engines", &self.engines.len())
            .finish()
    }
}

impl RoutingBackend {
    /// Build a routing backend over `engines` (member order = `supports()` tie-break).
    pub(crate) fn new(engines: Vec<Arc<dyn ServeEngine>>) -> Self {
        Self { engines }
    }

    /// The first member that serves `model_id`, if any.
    fn route(&self, model_id: &ModelId) -> Option<&Arc<dyn ServeEngine>> {
        self.engines.iter().find(|e| e.supports(model_id))
    }

    /// Warm `model_id` on its owning engine. Inherent twin of the [`ModelEngine`]
    /// impl (used by the host's warm-on-start path).
    ///
    /// # Errors
    /// [`InferenceError::ModelNotFound`] when no member serves `model_id`, else the
    /// owning engine's backend error.
    pub(crate) fn warm(&self, model_id: &ModelId) -> Result<(), InferenceError> {
        match self.route(model_id) {
            Some(engine) => {
                engine
                    .warm(&model_id.0)
                    .map_err(|message| InferenceError::BackendFailure {
                        backend: ROUTING_NAME,
                        message,
                    })
            }
            None => Err(InferenceError::ModelNotFound {
                model_id: model_id.0.clone(),
            }),
        }
    }
}

impl InferenceBackend for RoutingBackend {
    fn dispatch(
        &self,
        model_id: &ModelId,
        input: &InferenceInput,
        params: &InferenceParams,
        warrant: &WarrantSpec,
    ) -> Result<InferenceOutput, InferenceError> {
        match self.route(model_id) {
            Some(engine) => engine.dispatch(model_id, input, params, warrant),
            None => Err(InferenceError::ModelNotFound {
                model_id: model_id.0.clone(),
            }),
        }
    }

    fn dispatch_streaming(
        &self,
        model_id: &ModelId,
        input: &InferenceInput,
        params: &InferenceParams,
        warrant: &WarrantSpec,
        token_sink: Option<TokenSink>,
    ) -> Result<InferenceOutput, InferenceError> {
        match self.route(model_id) {
            Some(engine) => engine.dispatch_streaming(model_id, input, params, warrant, token_sink),
            None => Err(InferenceError::ModelNotFound {
                model_id: model_id.0.clone(),
            }),
        }
    }

    fn render_chat(&self, model_id: &ModelId, system: &str, user: &str) -> Option<String> {
        self.route(model_id)
            .and_then(|engine| engine.render_chat(model_id, system, user))
    }

    fn supports(&self, model_id: &ModelId) -> bool {
        self.engines.iter().any(|e| e.supports(model_id))
    }

    fn name(&self) -> &'static str {
        ROUTING_NAME
    }
}

impl EmbeddingBackend for RoutingBackend {
    /// Route an embed call to the FIRST member that serves `model_id` (the same
    /// `route()` rule as `dispatch`), forwarding to its [`ServeEngine::embed`]. A
    /// served-but-non-embedding engine returns [`InferenceError::Unsupported`]
    /// (bubbled, NOT collapsed into `ModelNotFound`); no member serving the model
    /// fails closed with [`InferenceError::ModelNotFound`].
    fn dispatch_embedding(
        &self,
        model_id: &ModelId,
        text: &str,
        pooling: EmbeddingPooling,
        warrant: &WarrantSpec,
    ) -> Result<EmbeddingOutput, InferenceError> {
        match self.route(model_id) {
            Some(engine) => engine.embed(model_id, text, pooling, warrant),
            None => Err(InferenceError::ModelNotFound {
                model_id: model_id.0.clone(),
            }),
        }
    }
}

impl ModelResidency for RoutingBackend {
    fn resident_ids(&self) -> Vec<String> {
        self.engines.iter().flat_map(|e| e.resident_ids()).collect()
    }
}

impl ModelEngine for RoutingBackend {
    fn warm(&self, model_id: &str) -> Result<(), String> {
        match self.route(&ModelId(model_id.to_string())) {
            Some(engine) => engine.warm(model_id),
            None => Err("model not registered".to_string()),
        }
    }

    fn evict(&self, model_id: &str) -> Result<bool, String> {
        match self.route(&ModelId(model_id.to_string())) {
            Some(engine) => engine.evict(model_id),
            None => Err("model not registered".to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::sync::Mutex;

    use kx_inference::{InferenceInput, InferenceParams};
    use kx_warrant::WarrantSpec;

    /// A no-FFI engine serving a fixed model set, recording warm/evict, returning a
    /// tagged completion so the routing can be observed end-to-end.
    struct FakeEngine {
        name: &'static str,
        models: BTreeSet<String>,
        resident: Mutex<BTreeSet<String>>,
        embeds: bool,
    }
    impl FakeEngine {
        fn new(name: &'static str, models: &[&str]) -> Self {
            Self {
                name,
                models: models.iter().map(|m| (*m).to_string()).collect(),
                resident: Mutex::new(BTreeSet::new()),
                embeds: false,
            }
        }
        /// Mark this fake as embedding-capable (mirrors a real `EmbeddingBackend`).
        fn embedding(mut self) -> Self {
            self.embeds = true;
            self
        }
    }
    impl InferenceBackend for FakeEngine {
        fn dispatch(
            &self,
            model_id: &ModelId,
            _input: &InferenceInput,
            _params: &InferenceParams,
            _warrant: &WarrantSpec,
        ) -> Result<InferenceOutput, InferenceError> {
            Ok(InferenceOutput {
                bytes: format!("{}:{}", self.name, model_id.0).into_bytes(),
                output_tokens: 1,
                backend_name: "fake",
                model_id: model_id.clone(),
                elapsed: std::time::Duration::ZERO,
            })
        }
        fn supports(&self, model_id: &ModelId) -> bool {
            self.models.contains(&model_id.0)
        }
        fn name(&self) -> &'static str {
            self.name
        }
    }
    impl ServeEngine for FakeEngine {
        fn warm(&self, model_id: &str) -> Result<(), String> {
            self.resident.lock().unwrap().insert(model_id.to_string());
            Ok(())
        }
        fn evict(&self, model_id: &str) -> Result<bool, String> {
            Ok(self.resident.lock().unwrap().remove(model_id))
        }
        fn resident_ids(&self) -> Vec<String> {
            self.resident.lock().unwrap().iter().cloned().collect()
        }
        fn embed(
            &self,
            model_id: &ModelId,
            _text: &str,
            _pooling: EmbeddingPooling,
            warrant: &WarrantSpec,
        ) -> Result<EmbeddingOutput, InferenceError> {
            if !self.embeds {
                // Served but non-embedding: bubble Unsupported (NOT ModelNotFound).
                return Err(InferenceError::Unsupported {
                    reason: "fake: no embedding capability",
                });
            }
            if model_id != &warrant.model_route.model_id {
                return Err(InferenceError::WarrantDeniesModel {
                    model_id: model_id.0.clone(),
                    route: warrant.model_route.model_id.0.clone(),
                });
            }
            if !self.supports(model_id) {
                return Err(InferenceError::ModelNotFound {
                    model_id: model_id.0.clone(),
                });
            }
            // Echo the model id so the caller can confirm WHICH engine/model embedded.
            Ok(EmbeddingOutput {
                vector: vec![1.0, 2.0, 3.0],
                dim: 3,
                backend_name: "fake",
                model_id: model_id.clone(),
                elapsed: std::time::Duration::ZERO,
            })
        }
    }

    fn routing() -> RoutingBackend {
        RoutingBackend::new(vec![
            Arc::new(FakeEngine::new("llama", &["gguf-a"])),
            Arc::new(FakeEngine::new("ollama", &["gemma3:12b"])),
        ])
    }

    fn dispatch(rb: &RoutingBackend, model: &str) -> Result<InferenceOutput, InferenceError> {
        rb.dispatch(
            &ModelId(model.to_string()),
            &InferenceInput::text("hi"),
            &InferenceParams::default(),
            &WarrantSpec::default(),
        )
    }

    #[test]
    fn dispatch_routes_to_the_owning_engine() {
        let rb = routing();
        assert_eq!(dispatch(&rb, "gguf-a").unwrap().bytes, b"llama:gguf-a");
        assert_eq!(
            dispatch(&rb, "gemma3:12b").unwrap().bytes,
            b"ollama:gemma3:12b"
        );
    }

    #[test]
    fn unknown_model_is_fail_closed() {
        let rb = routing();
        assert!(matches!(
            dispatch(&rb, "nope").unwrap_err(),
            InferenceError::ModelNotFound { .. }
        ));
    }

    #[test]
    fn supports_is_the_union() {
        let rb = routing();
        assert!(rb.supports(&ModelId("gguf-a".into())));
        assert!(rb.supports(&ModelId("gemma3:12b".into())));
        assert!(!rb.supports(&ModelId("nope".into())));
    }

    #[test]
    fn lifecycle_routes_to_the_owning_engine_and_unions_residency() {
        let rb = routing();
        ModelEngine::warm(&rb, "gguf-a").unwrap();
        ModelEngine::warm(&rb, "gemma3:12b").unwrap();
        let mut resident = ModelResidency::resident_ids(&rb);
        resident.sort();
        assert_eq!(
            resident,
            vec!["gemma3:12b".to_string(), "gguf-a".to_string()]
        );
        assert!(ModelEngine::evict(&rb, "gguf-a").unwrap());
        assert_eq!(
            ModelResidency::resident_ids(&rb),
            vec!["gemma3:12b".to_string()]
        );
        // An unregistered model is fail-closed.
        assert!(ModelEngine::warm(&rb, "nope").is_err());
    }

    // ---- PR-B: embedding routing (datasets server-embed at parity) ----

    fn warrant_for(model: &str) -> WarrantSpec {
        let mut w = WarrantSpec::default();
        w.model_route.model_id = ModelId(model.to_string());
        w
    }

    /// Routing where llama embeds `gguf-a` and ollama embeds `gemma3:12b`.
    fn routing_embed() -> RoutingBackend {
        RoutingBackend::new(vec![
            Arc::new(FakeEngine::new("llama", &["gguf-a"]).embedding()),
            Arc::new(FakeEngine::new("ollama", &["gemma3:12b"]).embedding()),
        ])
    }

    fn embed(rb: &RoutingBackend, model: &str) -> Result<EmbeddingOutput, InferenceError> {
        rb.dispatch_embedding(
            &ModelId(model.to_string()),
            "hello",
            EmbeddingPooling::Mean,
            &warrant_for(model),
        )
    }

    #[test]
    fn embedding_routes_to_the_capable_engine() {
        let rb = routing_embed();
        let out = embed(&rb, "gemma3:12b").unwrap();
        assert_eq!(out.model_id.0, "gemma3:12b");
        assert_eq!(out.dim, 3);
        // The llama-served model routes to the llama member.
        assert_eq!(embed(&rb, "gguf-a").unwrap().model_id.0, "gguf-a");
    }

    #[test]
    fn embedding_on_a_served_but_non_embedding_engine_is_unsupported() {
        // ollama serves gemma3:12b but is NOT embedding-capable.
        let rb = RoutingBackend::new(vec![Arc::new(FakeEngine::new("ollama", &["gemma3:12b"]))]);
        assert!(matches!(
            embed(&rb, "gemma3:12b").unwrap_err(),
            InferenceError::Unsupported { .. }
        ));
    }

    #[test]
    fn embedding_unknown_model_is_model_not_found() {
        let rb = routing_embed();
        assert!(matches!(
            embed(&rb, "nope").unwrap_err(),
            InferenceError::ModelNotFound { .. }
        ));
    }

    #[test]
    fn embedding_off_route_is_warrant_denied() {
        let rb = routing_embed();
        // The warrant authorizes a different model than the one embedded.
        let err = rb
            .dispatch_embedding(
                &ModelId("gemma3:12b".into()),
                "hi",
                EmbeddingPooling::Mean,
                &warrant_for("gguf-a"),
            )
            .unwrap_err();
        assert!(matches!(err, InferenceError::WarrantDeniesModel { .. }));
    }
}
