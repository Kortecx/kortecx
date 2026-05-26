//! [`ModelRegistry`] trait + OSS-default in-memory implementation.

use std::collections::BTreeMap;

use kx_mote::ModelId;

use crate::provided::ProvidedCapabilities;

/// A lookup over models the runtime knows about, keyed by [`ModelId`].
///
/// OSS ships [`InMemoryModelRegistry`]; cloud impls (hosted registry, S3-
/// backed manifests) plug in behind the same trait per D28 (cloud-first
/// scale principle).
pub trait ModelRegistry: Send + Sync {
    /// Look up a model's declared (or verified) provided capabilities.
    fn lookup(&self, model_id: &ModelId) -> Option<ProvidedCapabilities>;

    /// Iterate every model in the registry. Used by
    /// [`crate::Recommender::candidates`] to find substitutes when the
    /// requested model fails its check.
    fn entries(&self) -> Vec<(ModelId, ProvidedCapabilities)>;
}

/// In-memory `ModelRegistry`. The OSS default; cloud impls bring their own.
///
/// # Examples
///
/// ```
/// use kx_model_validator::{InMemoryModelRegistry, ModelRegistry, ProvidedCapabilities};
/// use kx_mote::ModelId;
///
/// let mut reg = InMemoryModelRegistry::new();
/// reg.insert(
///     ModelId("llama-3-8b-instruct".into()),
///     ProvidedCapabilities::declared().with_context_window_tokens(8_192),
/// );
/// assert!(reg.lookup(&ModelId("llama-3-8b-instruct".into())).is_some());
/// assert!(reg.lookup(&ModelId("unknown".into())).is_none());
/// ```
#[derive(Debug, Default, Clone)]
pub struct InMemoryModelRegistry {
    by_id: BTreeMap<ModelId, ProvidedCapabilities>,
}

impl InMemoryModelRegistry {
    /// Construct an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert (or replace) a model entry.
    pub fn insert(&mut self, id: ModelId, capabilities: ProvidedCapabilities) {
        self.by_id.insert(id, capabilities);
    }

    /// Remove an entry; returns whether it was present.
    pub fn remove(&mut self, id: &ModelId) -> bool {
        self.by_id.remove(id).is_some()
    }

    /// Number of entries in the registry.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    /// `true` when the registry has no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }
}

impl ModelRegistry for InMemoryModelRegistry {
    fn lookup(&self, model_id: &ModelId) -> Option<ProvidedCapabilities> {
        self.by_id.get(model_id).cloned()
    }

    fn entries(&self) -> Vec<(ModelId, ProvidedCapabilities)> {
        self.by_id
            .iter()
            .map(|(id, cap)| (id.clone(), cap.clone()))
            .collect()
    }
}
