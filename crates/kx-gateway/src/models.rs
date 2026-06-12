//! The Batch A host model catalog: a startup-built, immutable display list
//! behind the [`ModelCatalogView`] seam (`ListModels`).
//!
//! ALWAYS wired (the toolscout always-on precedent): an FFI-free serve answers
//! with an honest EMPTY list — "no models on this serve" is discovery data, not
//! an error. Display/discovery only (SN-8): model *selection* stays a recipe
//! ENUM free-param validated server-side at binding; nothing here authorizes
//! anything.

use kx_gateway_core::{GatewayError as CoreError, ModelCatalogView, ModelSummaryEntry};

/// The startup-built catalog: today the single resolved serve model (or none).
/// Immutable after construction — the serve model cannot change without a
/// restart, so there is nothing to refresh.
pub(crate) struct HostModelCatalog {
    entries: Vec<ModelSummaryEntry>,
}

impl HostModelCatalog {
    /// A catalog over the provisioned entries (built by the inference wiring
    /// from the SAME descriptor facts the backend registered; an empty vec is
    /// the FFI-free / no-fit-model serve).
    pub(crate) fn new(entries: Vec<ModelSummaryEntry>) -> Self {
        Self { entries }
    }
}

impl ModelCatalogView for HostModelCatalog {
    fn list(&self) -> Result<Vec<ModelSummaryEntry>, CoreError> {
        Ok(self.entries.clone())
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
        }]);
        let listed = catalog.list().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].model_id, "kx-serve:qwen3-4b");
        assert!(listed[0].serving);
    }
}
