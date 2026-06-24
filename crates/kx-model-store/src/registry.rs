// SPDX-License-Identifier: Apache-2.0
//! [`ModelResolver`] — the seam a backend resolves a `ModelId` through — and the
//! in-memory [`ModelRegistry`] implementation.

use std::collections::BTreeMap;

use kx_content::ContentRef;
use kx_mote::ModelId;

use crate::descriptor::ModelDescriptor;
use crate::errors::ModelStoreError;

/// Capacity ceiling on a single registry — a resource-exhaustion guard so an
/// unbounded registration loop cannot grow memory without bound. Far above any
/// realistic single-node model count.
pub const MAX_MODELS: usize = 4096;

/// Resolve a [`ModelId`] to its [`ModelDescriptor`].
///
/// This is the seam an `InferenceBackend` holds (as `Arc<dyn ModelResolver>`)
/// instead of a frozen `HashMap` — a future durable / remote registry (a cloud
/// backend, a hot-swap-on-journaled-fact registry) implements the same trait
/// without the backend changing. `Send + Sync` so it can live in a shared,
/// thread-safe backend.
pub trait ModelResolver: Send + Sync {
    /// The descriptor for `id`, or `None` if no model is registered under it.
    fn resolve(&self, id: &ModelId) -> Option<&ModelDescriptor>;

    /// The `(model_id, identity_digest)` pair of every registered model, in a
    /// stable order. The default is empty (a resolver that cannot enumerate —
    /// e.g. a future remote registry — opts out without breaking dyn-safety); an
    /// in-memory registry overrides it. Used to map a live cache-residency
    /// snapshot (keyed by `ContentRef`) back to `ModelId`s for display, and to
    /// derive the fixed registered set the lifecycle controls are scoped to.
    ///
    /// NOTE: `identity_digest` is path+modalities-keyed (not `ModelId`-keyed), so
    /// two ids over the SAME gguf+modalities share one digest ⇒ one resident
    /// entry; a residency reverse-map should pick the first id for that digest.
    fn iter_identities(&self) -> Vec<(ModelId, ContentRef)> {
        Vec::new()
    }
}

/// An in-memory model registry: a deterministic `BTreeMap<ModelId, _>`.
///
/// `BTreeMap` (not `HashMap`) so iteration order is stable — useful for
/// diagnostics and any future canonical listing.
#[derive(Debug, Clone, Default)]
pub struct ModelRegistry {
    models: BTreeMap<ModelId, ModelDescriptor>,
}

impl ModelRegistry {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a descriptor under its `model_id`.
    ///
    /// Lazy: does NOT touch the model file (call
    /// [`ModelDescriptor::validate`](crate::ModelDescriptor::validate) for that).
    ///
    /// # Errors
    ///
    /// - [`ModelStoreError::DuplicateModel`] if the id is already registered
    ///   (re-registration is refused so identity cannot silently change under a
    ///   live cache).
    /// - [`ModelStoreError::TooManyModels`] if the registry is at [`MAX_MODELS`].
    pub fn register(&mut self, descriptor: ModelDescriptor) -> Result<(), ModelStoreError> {
        if self.models.contains_key(&descriptor.model_id) {
            return Err(ModelStoreError::DuplicateModel {
                model_id: descriptor.model_id.0.clone(),
            });
        }
        if self.models.len() >= MAX_MODELS {
            return Err(ModelStoreError::TooManyModels { cap: MAX_MODELS });
        }
        tracing::debug!(model_id = %descriptor.model_id.0, "registering model descriptor");
        self.models.insert(descriptor.model_id.clone(), descriptor);
        Ok(())
    }

    /// Build a registry from an iterator of descriptors, failing closed on the
    /// first duplicate or over-cap.
    ///
    /// # Errors
    ///
    /// Propagates [`register`](Self::register)'s errors.
    pub fn from_descriptors(
        descriptors: impl IntoIterator<Item = ModelDescriptor>,
    ) -> Result<Self, ModelStoreError> {
        let mut reg = Self::new();
        for d in descriptors {
            reg.register(d)?;
        }
        Ok(reg)
    }

    /// Number of registered models.
    #[must_use]
    pub fn len(&self) -> usize {
        self.models.len()
    }

    /// Whether the registry is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.models.is_empty()
    }

    /// Iterate the registered descriptors in `ModelId` order.
    pub fn iter(&self) -> impl Iterator<Item = &ModelDescriptor> {
        self.models.values()
    }
}

impl ModelResolver for ModelRegistry {
    fn resolve(&self, id: &ModelId) -> Option<&ModelDescriptor> {
        self.models.get(id)
    }

    fn iter_identities(&self) -> Vec<(ModelId, ContentRef)> {
        self.models
            .values()
            .map(|d| (d.model_id.clone(), d.identity_digest))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptor::Modality;

    fn mid(s: &str) -> ModelId {
        ModelId(s.to_string())
    }

    #[test]
    fn register_and_resolve() {
        let mut reg = ModelRegistry::new();
        reg.register(ModelDescriptor::text(mid("a"), "/m/a.gguf", 4096))
            .unwrap();
        let d = reg.resolve(&mid("a")).expect("registered");
        assert_eq!(d.gguf_path.to_str().unwrap(), "/m/a.gguf");
        assert!(d.supports(Modality::Text));
        assert!(reg.resolve(&mid("missing")).is_none());
    }

    #[test]
    fn duplicate_registration_is_refused() {
        let mut reg = ModelRegistry::new();
        reg.register(ModelDescriptor::text(mid("a"), "/m/a.gguf", 4096))
            .unwrap();
        let err = reg
            .register(ModelDescriptor::text(mid("a"), "/m/other.gguf", 4096))
            .unwrap_err();
        assert!(matches!(err, ModelStoreError::DuplicateModel { .. }));
        // Original is intact.
        assert_eq!(
            reg.resolve(&mid("a")).unwrap().gguf_path.to_str().unwrap(),
            "/m/a.gguf"
        );
    }

    #[test]
    fn from_descriptors_roundtrips() {
        let reg = ModelRegistry::from_descriptors([
            ModelDescriptor::text(mid("a"), "/m/a.gguf", 4096),
            ModelDescriptor::text(mid("b"), "/m/b.gguf", 2048),
        ])
        .unwrap();
        assert_eq!(reg.len(), 2);
        assert!(!reg.is_empty());
        // Deterministic ModelId order.
        let ids: Vec<_> = reg.iter().map(|d| d.model_id.0.clone()).collect();
        assert_eq!(ids, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn iter_identities_maps_ids_to_cache_digests_in_order() {
        let reg = ModelRegistry::from_descriptors([
            ModelDescriptor::text(mid("b"), "/m/b.gguf", 2048),
            ModelDescriptor::text(mid("a"), "/m/a.gguf", 4096),
        ])
        .unwrap();
        let pairs = reg.iter_identities();
        // BTreeMap order: a before b.
        let ids: Vec<_> = pairs.iter().map(|(id, _)| id.0.clone()).collect();
        assert_eq!(ids, vec!["a".to_string(), "b".to_string()]);
        // Each digest matches the descriptor's identity_digest.
        assert_eq!(pairs[0].1, reg.resolve(&mid("a")).unwrap().identity_digest);
        assert_eq!(pairs[1].1, reg.resolve(&mid("b")).unwrap().identity_digest);
    }

    #[test]
    fn default_iter_identities_is_empty() {
        // A resolver that does not override iter_identities opts out (dyn-safe).
        struct Opaque(ModelDescriptor);
        impl ModelResolver for Opaque {
            fn resolve(&self, id: &ModelId) -> Option<&ModelDescriptor> {
                (self.0.model_id == *id).then_some(&self.0)
            }
        }
        let r = Opaque(ModelDescriptor::text(mid("a"), "/m/a.gguf", 4096));
        assert!(r.resolve(&mid("a")).is_some());
        assert!(r.iter_identities().is_empty());
    }
}
