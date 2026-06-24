//! POC-3 host-side model lifecycle: the live-engine adapter over the inference
//! backend + the `ModelLifecycleControl` host impl behind `LoadModel`/
//! `OffloadModel`.
//!
//! Only the inference serve has a live model engine, so this whole module is
//! inference-gated. The control is scoped to the FIXED registered set (an
//! unregistered id is `NotFound`, fail-closed BEFORE the engine is asked — never
//! a warm of an arbitrary path). Off-journal / off-digest: residency is ephemeral
//! RAM state that rebuilds EMPTY on restart.

use std::collections::BTreeSet;
use std::sync::Arc;

use kx_gateway_core::{GatewayError, ModelLifecycleControl, ModelLifecycleOutcome};
use kx_inference::LlamaInferenceBackend;
use kx_mote::ModelId;

use crate::models::ModelResidency;

/// The live model engine the lifecycle host drives: residency reads
/// ([`ModelResidency`] supertrait, shared with the catalog) PLUS the mutating
/// warm/evict controls. Implemented over the inference backend (`BackendEngine`);
/// abstracted as a trait so the host impl is unit-testable without the FFI.
pub(crate) trait ModelEngine: ModelResidency {
    /// Warm a registered model into RAM. `Err(msg)` on a backend/load failure.
    fn warm(&self, model_id: &str) -> Result<(), String>;
    /// Evict a registered model from RAM; `Ok(true)` iff it was resident.
    fn evict(&self, model_id: &str) -> Result<bool, String>;
}

/// The live-engine adapter over the concrete inference backend. A newtype (not an
/// inherent-method `impl` on the backend) so the trait `warm`/`evict` never
/// collide with the backend's inherent `warm`/`evict`.
pub(crate) struct BackendEngine(pub(crate) Arc<LlamaInferenceBackend>);

impl ModelResidency for BackendEngine {
    fn resident_ids(&self) -> Vec<String> {
        self.0.resident().into_iter().map(|m| m.0).collect()
    }
}

impl ModelEngine for BackendEngine {
    fn warm(&self, model_id: &str) -> Result<(), String> {
        self.0
            .warm(&ModelId(model_id.to_string()))
            .map_err(|e| e.to_string())
    }

    fn evict(&self, model_id: &str) -> Result<bool, String> {
        self.0
            .evict(&ModelId(model_id.to_string()))
            .map_err(|e| e.to_string())
    }
}

/// The host impl behind `LoadModel`/`OffloadModel`. Holds the live engine + the
/// FIXED registered set (the server's startup-provisioned model ids); an
/// unregistered id is refused with `NotFound` BEFORE the engine is touched.
pub(crate) struct HostModelLifecycle {
    engine: Arc<dyn ModelEngine>,
    registered: BTreeSet<String>,
}

impl HostModelLifecycle {
    pub(crate) fn new(engine: Arc<dyn ModelEngine>, registered: BTreeSet<String>) -> Self {
        Self { engine, registered }
    }

    /// Fail-closed gate: only a model in the server's fixed registered set can be
    /// warmed/evicted. A static message (no id echo) — the registered set is
    /// already enumerable via `ListModels`, so this is honest, not an oracle.
    fn ensure_registered(&self, model_id: &str) -> Result<(), GatewayError> {
        if self.registered.contains(model_id) {
            Ok(())
        } else {
            Err(GatewayError::NotFound("model not registered"))
        }
    }
}

impl ModelLifecycleControl for HostModelLifecycle {
    fn load(&self, model_id: &str) -> Result<ModelLifecycleOutcome, GatewayError> {
        self.ensure_registered(model_id)?;
        let was_resident = self.engine.resident_ids().iter().any(|id| id == model_id);
        self.engine.warm(model_id).map_err(GatewayError::Internal)?;
        Ok(ModelLifecycleOutcome {
            model_id: model_id.to_string(),
            loaded: true,
            was_resident,
        })
    }

    fn offload(&self, model_id: &str) -> Result<ModelLifecycleOutcome, GatewayError> {
        self.ensure_registered(model_id)?;
        let was_resident = self
            .engine
            .evict(model_id)
            .map_err(GatewayError::Internal)?;
        Ok(ModelLifecycleOutcome {
            model_id: model_id.to_string(),
            loaded: false,
            was_resident,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// An in-memory fake engine (no FFI) recording warm/evict + a residency set.
    struct FakeEngine {
        resident: Mutex<BTreeSet<String>>,
    }
    impl FakeEngine {
        fn new(resident: &[&str]) -> Self {
            Self {
                resident: Mutex::new(resident.iter().map(|s| (*s).to_string()).collect()),
            }
        }
    }
    impl ModelResidency for FakeEngine {
        fn resident_ids(&self) -> Vec<String> {
            self.resident.lock().unwrap().iter().cloned().collect()
        }
    }
    impl ModelEngine for FakeEngine {
        fn warm(&self, model_id: &str) -> Result<(), String> {
            self.resident.lock().unwrap().insert(model_id.to_string());
            Ok(())
        }
        fn evict(&self, model_id: &str) -> Result<bool, String> {
            Ok(self.resident.lock().unwrap().remove(model_id))
        }
    }

    fn lifecycle(resident: &[&str]) -> HostModelLifecycle {
        HostModelLifecycle::new(
            Arc::new(FakeEngine::new(resident)),
            ["gemma", "qwen"].iter().map(|s| (*s).to_string()).collect(),
        )
    }

    #[test]
    fn load_cold_then_idempotent() {
        let lc = lifecycle(&[]);
        let out = lc.load("qwen").unwrap();
        assert_eq!(out.model_id, "qwen");
        assert!(out.loaded);
        assert!(!out.was_resident, "cold load");
        // Loading again is idempotent and reports it was already resident.
        let out2 = lc.load("qwen").unwrap();
        assert!(out2.loaded);
        assert!(out2.was_resident);
    }

    #[test]
    fn offload_frees_then_idempotent() {
        let lc = lifecycle(&["gemma"]);
        let out = lc.offload("gemma").unwrap();
        assert!(!out.loaded);
        assert!(out.was_resident, "was resident before offload");
        // Offloading again is an idempotent no-op.
        let out2 = lc.offload("gemma").unwrap();
        assert!(!out2.loaded);
        assert!(!out2.was_resident);
    }

    #[test]
    fn unregistered_model_is_fail_closed_not_found() {
        let lc = lifecycle(&[]);
        let err = lc.load("not-registered").unwrap_err();
        assert!(matches!(err, GatewayError::NotFound(_)));
        let err = lc.offload("not-registered").unwrap_err();
        assert!(matches!(err, GatewayError::NotFound(_)));
    }
}
