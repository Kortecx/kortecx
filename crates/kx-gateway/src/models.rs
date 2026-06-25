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
use std::sync::{Arc, PoisonError, RwLock};

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
/// display fields are immutable PER ENTRY, but the entry list itself is shared +
/// interior-mutable so a runtime `kx models pull` (Model Control v2) can append a
/// freshly-registered model and have it appear in `ListModels` WITHOUT a restart.
/// Only the `loaded` residency flag is recomputed live from the engine.
pub(crate) struct HostModelCatalog {
    /// Shared, append-only display entries. The same `Arc` is handed to the model
    /// puller so a pull's registration is visible here immediately (off-journal /
    /// off-digest — the catalog is pure RAM display state, SN-8).
    entries: Arc<RwLock<Vec<ModelSummaryEntry>>>,
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
            entries: Arc::new(RwLock::new(entries)),
            engine: None,
        }
    }

    /// Bind the live model engine so `list()` reports live RAM residency (POC-3).
    /// Only a serve-engine serve calls this; a model-less build never does.
    #[cfg(feature = "serve-engine")]
    #[must_use]
    pub(crate) fn with_engine(mut self, engine: Arc<dyn ModelResidency>) -> Self {
        self.engine = Some(engine);
        self
    }

    /// The shared entries handle — the model puller holds a clone so a runtime
    /// registration ([`register_entry`](Self::register_entry)) is visible through
    /// this same catalog. Only the serve-engine wiring (which builds the puller)
    /// uses it.
    #[cfg(feature = "serve-engine")]
    pub(crate) fn entries_handle(&self) -> Arc<RwLock<Vec<ModelSummaryEntry>>> {
        self.entries.clone()
    }

    /// Append a runtime-registered model's display entry (Model Control v2). Deduped
    /// by `model_id` — a re-register of an existing id is a benign no-op (the first
    /// entry, which carries the original `serving`/route facts, wins). A pulled model
    /// is always a SECONDARY (`serving = false`), so the primary route is untouched
    /// and the canonical projection digest stays invariant.
    #[cfg(feature = "serve-engine")]
    pub(crate) fn register_entry(
        entries: &RwLock<Vec<ModelSummaryEntry>>,
        entry: ModelSummaryEntry,
    ) {
        let mut guard = entries.write().unwrap_or_else(PoisonError::into_inner);
        if guard.iter().any(|e| e.model_id == entry.model_id) {
            return;
        }
        guard.push(entry);
    }
}

impl ModelCatalogView for HostModelCatalog {
    fn list(&self) -> Result<Vec<ModelSummaryEntry>, CoreError> {
        // Clone the snapshot OUT from under the read lock so the (cheap owner-thread)
        // residency round-trip never holds the catalog lock.
        let mut entries = self
            .entries
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
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
            engine: "kx-llamacpp".into(),
            can_embed: true,
            source: "local".into(),
            active: false,
            chat_rag_handle: String::new(),
        }]);
        let listed = catalog.list().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].model_id, "kx-serve:qwen3-4b");
        assert!(listed[0].serving);
        // No engine bound ⇒ residency is honestly false.
        assert!(!listed[0].loaded);
        // PR-B: the embedder flag passes through unchanged.
        assert!(listed[0].can_embed);
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
                engine: "kx-llamacpp".into(),
                can_embed: false,
                source: "local".into(),
                active: false,
                chat_rag_handle: String::new(),
            },
            ModelSummaryEntry {
                model_id: "b".into(),
                modalities: vec!["text".into()],
                description: "B".into(),
                serving: false,
                context_len: 4096,
                loaded: false,
                chat_handle: "kx/recipes/m-b".into(),
                engine: "kx-ollama".into(),
                can_embed: false,
                source: "ollama".into(),
                active: false,
                chat_rag_handle: String::new(),
            },
        ];
        // Only "b" resident.
        let catalog = HostModelCatalog {
            entries: Arc::new(RwLock::new(entries)),
            engine: Some(Arc::new(FakeEngine(vec!["b".into()]))),
        };
        let listed = catalog.list().unwrap();
        assert!(!listed[0].loaded, "a is not resident");
        assert!(listed[1].loaded, "b is resident");
    }

    #[cfg(feature = "serve-engine")]
    fn entry(id: &str, serving: bool) -> ModelSummaryEntry {
        ModelSummaryEntry {
            model_id: id.into(),
            modalities: vec!["text".into()],
            description: id.into(),
            serving,
            context_len: 4096,
            loaded: false,
            chat_handle: format!("kx/recipes/m-{id}"),
            engine: "kx-ollama".into(),
            can_embed: false,
            source: "pulled-ollama".into(),
            active: false,
            chat_rag_handle: String::new(),
        }
    }

    #[cfg(feature = "serve-engine")]
    #[test]
    fn register_entry_appends_and_is_visible_in_list() {
        // A1: a runtime registration is visible through the SAME catalog without a
        // restart (the shared `Arc<RwLock<Vec>>`).
        let catalog = HostModelCatalog::new(vec![entry("a", true)]);
        let handle = catalog.entries_handle();
        assert_eq!(catalog.list().unwrap().len(), 1);
        HostModelCatalog::register_entry(&handle, entry("b", false));
        let listed = catalog.list().unwrap();
        assert_eq!(listed.len(), 2);
        assert!(listed.iter().any(|e| e.model_id == "b"));
    }

    #[cfg(feature = "serve-engine")]
    #[test]
    fn register_entry_dedups_by_model_id() {
        let catalog = HostModelCatalog::new(vec![entry("a", true)]);
        let handle = catalog.entries_handle();
        // A re-register of an existing id is a benign no-op (the original wins).
        HostModelCatalog::register_entry(&handle, entry("a", false));
        let listed = catalog.list().unwrap();
        assert_eq!(listed.len(), 1);
        assert!(
            listed[0].serving,
            "the original entry's facts are preserved"
        );
    }
}
