//! The Batch A host model catalog: a startup-built display list behind the
//! [`ModelCatalogView`] seam (`ListModels`), plus (POC-3) the live-residency
//! recompute.
//!
//! ALWAYS wired (the toolscout always-on precedent): an FFI-free serve answers
//! with an honest EMPTY list — "no models on this serve" is discovery data, not
//! an error. Display/discovery only (SN-8): model *selection* stays a recipe
//! ENUM free-param validated server-side at binding; nothing here authorizes
//! anything. The per-model `loaded` flag (POC-3) is recomputed per `list()` call
//! from the live engine residency — display only, never an authority bit.

use std::collections::BTreeSet;
use std::sync::Arc;

use kx_gateway_core::{GatewayError as CoreError, ModelCatalogView, ModelSummaryEntry};

/// The host's READ handle to live model residency (POC-3 `ListModels.loaded`).
/// Abstracts the concrete inference backend so the always-compiled catalog stays
/// feature-agnostic; implemented over the inference backend ONLY under the
/// `inference` feature (`BackendEngine` in `model_lifecycle`), so an FFI-free
/// build never has one ⇒ residency is always empty. The mutating warm/evict
/// controls live on the inference-only `ModelEngine` supertrait.
pub(crate) trait ModelResidency: Send + Sync {
    /// The model ids currently RESIDENT in RAM (the live LRU snapshot).
    fn resident_ids(&self) -> Vec<String>;
}

/// The startup-built catalog: the registered serve model set (or none). The base
/// display fields are immutable after construction; only the `loaded` residency
/// flag is recomputed live from the engine.
pub(crate) struct HostModelCatalog {
    entries: Vec<ModelSummaryEntry>,
    /// The live residency view, if this is an inference serve. `None` ⇒ FFI-free
    /// (`loaded` stays `false` for every entry — honest).
    engine: Option<Arc<dyn ModelResidency>>,
}

impl HostModelCatalog {
    /// A catalog over the provisioned entries (built by the inference wiring
    /// from the SAME descriptor facts the backend registered; an empty vec is
    /// the FFI-free / no-fit-model serve). No engine ⇒ `loaded` always `false`.
    pub(crate) fn new(entries: Vec<ModelSummaryEntry>) -> Self {
        Self {
            entries,
            engine: None,
        }
    }

    /// Bind the live model engine so `list()` reports live RAM residency (POC-3).
    /// Only the inference serve calls this; an FFI-free build never does.
    #[cfg(feature = "inference")]
    #[must_use]
    pub(crate) fn with_engine(mut self, engine: Arc<dyn ModelResidency>) -> Self {
        self.engine = Some(engine);
        self
    }
}

impl ModelCatalogView for HostModelCatalog {
    fn list(&self) -> Result<Vec<ModelSummaryEntry>, CoreError> {
        let mut entries = self.entries.clone();
        if let Some(engine) = &self.engine {
            // Recompute `loaded` from the live LRU snapshot (cheap owner-thread
            // round-trip; ListModels is a low-frequency display RPC).
            let resident: BTreeSet<String> = engine.resident_ids().into_iter().collect();
            for e in &mut entries {
                e.loaded = resident.contains(&e.model_id);
            }
        }
        Ok(entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_catalog_lists_nothing() {
        assert!(HostModelCatalog::new(Vec::new()).list().unwrap().is_empty());
    }

    #[test]
    fn entries_pass_through_unchanged() {
        let catalog = HostModelCatalog::new(vec![ModelSummaryEntry {
            model_id: "kx-serve:qwen3-4b".into(),
            modalities: vec!["text".into()],
            description: "Qwen3 4B".into(),
            serving: true,
            context_len: 8192,
            loaded: false,
            chat_handle: "kx/recipes/chat".into(),
        }]);
        let listed = catalog.list().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].model_id, "kx-serve:qwen3-4b");
        assert!(listed[0].serving);
        // No engine bound ⇒ residency is honestly false.
        assert!(!listed[0].loaded);
    }

    /// A fake residency view to drive the `loaded` recompute without the FFI.
    struct FakeEngine(Vec<String>);
    impl ModelResidency for FakeEngine {
        fn resident_ids(&self) -> Vec<String> {
            self.0.clone()
        }
    }

    #[test]
    fn loaded_reflects_live_engine_residency() {
        let entries = vec![
            ModelSummaryEntry {
                model_id: "a".into(),
                modalities: vec!["text".into()],
                description: "A".into(),
                serving: true,
                context_len: 4096,
                loaded: false,
                chat_handle: "kx/recipes/chat".into(),
            },
            ModelSummaryEntry {
                model_id: "b".into(),
                modalities: vec!["text".into()],
                description: "B".into(),
                serving: false,
                context_len: 4096,
                loaded: false,
                chat_handle: "kx/recipes/m-b".into(),
            },
        ];
        // Only "b" resident.
        let catalog = HostModelCatalog {
            entries,
            engine: Some(Arc::new(FakeEngine(vec!["b".into()]))),
        };
        let listed = catalog.list().unwrap();
        assert!(!listed[0].loaded, "a is not resident");
        assert!(listed[1].loaded, "b is resident");
    }
}
