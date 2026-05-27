//! [`MoteDefRegistry`] — `MoteDefHash` → [`kx_mote::MoteDef`] lookup, the
//! materializer's seam to recover the shaper's full `MoteDef` from a
//! committed entry (which only carries the hash per `kx-journal` v2 / D36).
//!
//! Production callers wire an [`InMemoryMoteDefRegistry`] populated at
//! workflow-submission time. The trait shape admits cloud-side impls
//! (distributed registry, content-addressed MoteDef store) without
//! touching the materializer.

use std::collections::BTreeMap;
use std::sync::RwLock;

use kx_mote::{MoteDef, MoteDefHash};

/// `MoteDefHash` → `MoteDef` lookup seam.
///
/// MUST be pure / total / deterministic for any hash that was previously
/// registered: same hash in → byte-identical `MoteDef` out (or `None` if
/// never registered). No I/O on the read path beyond what the impl
/// requires.
pub trait MoteDefRegistry: Send + Sync {
    /// Return the registered `MoteDef` for this hash, or `None` if
    /// nothing was registered.
    ///
    /// Callers that need the def MUST handle `None` — an unknown hash
    /// at materialization time means the workflow author never
    /// registered the shaper's `MoteDef`, which is a workflow-author
    /// error.
    fn get(&self, hash: &MoteDefHash) -> Option<MoteDef>;
}

/// In-memory implementation of [`MoteDefRegistry`]. Workflow authors
/// register every `MoteDef` they will reference (shapers + materialized
/// children's `MoteDef`s — though the OSS-default
/// [`crate::InheritFromShaperResolver`] does not produce new defs that
/// would need separate registration; only the workflow-author-declared
/// shapers are looked up here).
///
/// # Examples
///
/// ```
/// use kx_mote::{EffectPattern, LogicRef, MoteDef, NdClass};
/// use kx_projection::{InMemoryMoteDefRegistry, MoteDefRegistry};
/// use std::collections::BTreeMap;
///
/// let def = MoteDef {
///     logic_ref: LogicRef([1u8; 32]),
///     model_id: kx_mote::ModelId("m".into()),
///     prompt_template_hash: kx_mote::PromptTemplateHash([2u8; 32]),
///     tool_contract: BTreeMap::new(),
///     nd_class: NdClass::Pure,
///     config_subset: BTreeMap::new(),
///     effect_pattern: EffectPattern::IdempotentByConstruction,
///     critic_for: None,
///     is_topology_shaper: false,
///     schema_version: kx_mote::MOTE_DEF_SCHEMA_VERSION,
/// };
/// let hash = def.hash();
/// let registry = InMemoryMoteDefRegistry::new();
/// registry.register(def.clone());
/// let recovered = registry.get(&hash).expect("registered");
/// assert_eq!(recovered, def);
/// ```
#[derive(Default)]
pub struct InMemoryMoteDefRegistry {
    inner: RwLock<BTreeMap<MoteDefHash, MoteDef>>,
}

impl InMemoryMoteDefRegistry {
    /// Construct an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a `MoteDef`. The key is its content-addressed hash via
    /// [`MoteDef::hash`].
    ///
    /// Idempotent: registering the same `MoteDef` twice is a no-op (the
    /// hash collision is by design — same bytes → same hash).
    pub fn register(&self, def: MoteDef) {
        let h = def.hash();
        // Recover from poison: a writer panicked, but the map's state is
        // still consistent for our purposes (no torn writes — `insert` is
        // atomic from the outside).
        let mut w = self
            .inner
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        w.insert(h, def);
    }

    /// Number of registered defs.
    pub fn len(&self) -> usize {
        self.inner
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }

    /// `true` if no defs are registered.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl MoteDefRegistry for InMemoryMoteDefRegistry {
    fn get(&self, hash: &MoteDefHash) -> Option<MoteDef> {
        self.inner
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(hash)
            .cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kx_mote::{EffectPattern, LogicRef, NdClass, PromptTemplateHash, MOTE_DEF_SCHEMA_VERSION};
    use std::collections::BTreeMap;

    fn def(seed: u8) -> MoteDef {
        MoteDef {
            logic_ref: LogicRef([seed; 32]),
            model_id: kx_mote::ModelId(format!("m-{seed}")),
            prompt_template_hash: PromptTemplateHash([seed.wrapping_add(1); 32]),
            tool_contract: BTreeMap::new(),
            nd_class: NdClass::Pure,
            config_subset: BTreeMap::new(),
            effect_pattern: EffectPattern::IdempotentByConstruction,
            critic_for: None,
            is_topology_shaper: false,
            schema_version: MOTE_DEF_SCHEMA_VERSION,
        }
    }

    #[test]
    fn empty_registry_returns_none() {
        let r = InMemoryMoteDefRegistry::new();
        let unknown_hash = MoteDefHash::from_bytes([0u8; 32]);
        assert!(r.get(&unknown_hash).is_none());
        assert!(r.is_empty());
    }

    #[test]
    fn round_trip_register_and_get() {
        let r = InMemoryMoteDefRegistry::new();
        let d = def(7);
        let h = d.hash();
        r.register(d.clone());
        let got = r.get(&h).expect("registered");
        assert_eq!(got, d);
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn double_register_same_def_idempotent() {
        let r = InMemoryMoteDefRegistry::new();
        let d = def(7);
        r.register(d.clone());
        r.register(d.clone());
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn different_defs_distinct_hashes() {
        let r = InMemoryMoteDefRegistry::new();
        let a = def(1);
        let b = def(2);
        assert_ne!(a.hash(), b.hash());
        r.register(a);
        r.register(b);
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<InMemoryMoteDefRegistry>();
        assert_send_sync::<std::sync::Arc<dyn MoteDefRegistry>>();
    }
}
