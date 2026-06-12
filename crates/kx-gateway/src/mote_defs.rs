//! The Batch B host def-resolver: [`MoteDefView`] over the SAME content store
//! the coordinator persists admitted defs into (`persist_def` — the canonical
//! encode's blake3 IS `mote_def_hash`, so the hash doubles as the address).
//!
//! Read-only + display-only (SN-8). A small FIFO-bounded decode cache keyed by
//! the def hash makes the per-drawer-open unary cheap: content-addressed defs
//! are immutable, so a cached entry can never go stale (no invalidation), and
//! hash-keying dedupes across motes/instances sharing one definition.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

use kx_content::{ContentRef, ContentStore, LocalFsContentStore};
use kx_gateway_core::{GatewayError as CoreError, MoteDefView};
use kx_mote::MoteDef;

/// Decoded-def cache cap. 256 defs ≈ 1 MiB worst case (defs are KB-scale);
/// FIFO eviction — recency tracking buys nothing at this size.
const DEF_CACHE_CAP: usize = 256;

/// The insertion-order bounded cache (hand-rolled: the `lru` crate is not in
/// the dependency tree and 30 lines do not justify a new edge).
struct BoundedDefCache {
    defs: BTreeMap<[u8; 32], Arc<MoteDef>>,
    order: VecDeque<[u8; 32]>,
}

impl BoundedDefCache {
    const fn new() -> Self {
        Self {
            defs: BTreeMap::new(),
            order: VecDeque::new(),
        }
    }

    fn get(&self, hash: &[u8; 32]) -> Option<Arc<MoteDef>> {
        self.defs.get(hash).cloned()
    }

    fn insert(&mut self, hash: [u8; 32], def: Arc<MoteDef>) {
        if self.defs.insert(hash, def).is_none() {
            self.order.push_back(hash);
            if self.order.len() > DEF_CACHE_CAP {
                if let Some(evicted) = self.order.pop_front() {
                    self.defs.remove(&evicted);
                }
            }
        }
    }
}

/// The host [`MoteDefView`]: content-store get → canonical decode → re-hash
/// integrity check → cache. Every miss flavor (absent blob, undecodable bytes,
/// a re-hash mismatch from on-disk corruption) degrades to `Ok(None)` — the
/// handler answers `def_found = false`; the blob is display substrate, never
/// load-bearing.
pub(crate) struct HostMoteDefView {
    store: Arc<LocalFsContentStore>,
    cache: Mutex<BoundedDefCache>,
}

impl HostMoteDefView {
    pub(crate) fn new(store: Arc<LocalFsContentStore>) -> Self {
        Self {
            store,
            cache: Mutex::new(BoundedDefCache::new()),
        }
    }
}

impl MoteDefView for HostMoteDefView {
    fn get_def(&self, mote_def_hash: &[u8; 32]) -> Result<Option<MoteDef>, CoreError> {
        if let Ok(cache) = self.cache.lock() {
            if let Some(def) = cache.get(mote_def_hash) {
                return Ok(Some((*def).clone()));
            }
        }
        let Ok(bytes) = self.store.get(&ContentRef::from_bytes(*mote_def_hash)) else {
            return Ok(None); // absent blob (pre-Batch-B journal / persist miss)
        };
        let def = match MoteDef::decode(&bytes) {
            Ok(def) => def,
            Err(error) => {
                tracing::warn!(%error, "def blob at the requested hash did not decode");
                return Ok(None);
            }
        };
        // Defense-in-depth against on-disk corruption: the decoded def must
        // re-hash to its own address (the content store already names by
        // blake3, so a mismatch means the bytes changed under us).
        if def.hash().as_bytes() != mote_def_hash {
            tracing::warn!("decoded def re-hash mismatch — treating as absent");
            return Ok(None);
        }
        let shared = Arc::new(def);
        if let Ok(mut cache) = self.cache.lock() {
            cache.insert(*mote_def_hash, Arc::clone(&shared));
        }
        Ok(Some((*shared).clone()))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use kx_mote::{
        ConfigKey, ConfigVal, EffectPattern, InferenceParams, LogicRef, ModelId, NdClass,
        PromptTemplateHash, MOTE_DEF_SCHEMA_VERSION,
    };
    use tempfile::TempDir;

    use super::*;

    fn def(tag: u8) -> MoteDef {
        let mut config_subset = BTreeMap::new();
        config_subset.insert(ConfigKey("tag".into()), ConfigVal(vec![tag]));
        MoteDef {
            critic_check: None,
            logic_ref: LogicRef::from_bytes([7u8; 32]),
            model_id: ModelId("m".into()),
            prompt_template_hash: PromptTemplateHash::from_bytes([9u8; 32]),
            tool_contract: BTreeMap::new(),
            nd_class: NdClass::Pure,
            config_subset,
            effect_pattern: EffectPattern::IdempotentByConstruction,
            critic_for: None,
            is_topology_shaper: false,
            inference_params: InferenceParams::default(),
            schema_version: MOTE_DEF_SCHEMA_VERSION,
        }
    }

    fn view(dir: &TempDir) -> (HostMoteDefView, Arc<LocalFsContentStore>) {
        let store = Arc::new(LocalFsContentStore::open(dir.path().join("content")).unwrap());
        (HostMoteDefView::new(store.clone()), store)
    }

    #[test]
    fn resolves_a_persisted_def_and_caches_it() {
        let dir = TempDir::new().unwrap();
        let (view, store) = view(&dir);
        let d = def(1);
        store.put(&d.encode()).unwrap();
        let resolved = view.get_def(d.hash().as_bytes()).unwrap().unwrap();
        assert_eq!(resolved, d);
        // Cached round (the store could now even drop the blob).
        store
            .delete(&ContentRef::from_bytes(*d.hash().as_bytes()))
            .unwrap();
        let cached = view.get_def(d.hash().as_bytes()).unwrap().unwrap();
        assert_eq!(cached, d);
    }

    #[test]
    fn absent_blob_is_an_honest_none() {
        let dir = TempDir::new().unwrap();
        let (view, _store) = view(&dir);
        assert!(view.get_def(&[0xEE; 32]).unwrap().is_none());
    }

    #[test]
    fn undecodable_bytes_are_an_honest_none() {
        let dir = TempDir::new().unwrap();
        let (view, store) = view(&dir);
        let garbage_ref = store.put(b"not a canonical MoteDef").unwrap();
        assert!(view.get_def(garbage_ref.as_bytes()).unwrap().is_none());
    }

    #[test]
    fn cache_is_fifo_bounded_at_the_cap() {
        let dir = TempDir::new().unwrap();
        let (view, store) = view(&dir);
        // Fill the cache one past the cap; the FIRST entry evicts.
        let first = def(0);
        store.put(&first.encode()).unwrap();
        view.get_def(first.hash().as_bytes()).unwrap().unwrap();
        for i in 0..DEF_CACHE_CAP {
            let mut d = def(1);
            d.config_subset
                .insert(ConfigKey("i".into()), ConfigVal(i.to_le_bytes().to_vec()));
            store.put(&d.encode()).unwrap();
            view.get_def(d.hash().as_bytes()).unwrap().unwrap();
        }
        // Evicted from the cache — but the STORE still holds it, so the read
        // path stays correct (a cache miss re-resolves).
        let resolved = view.get_def(first.hash().as_bytes()).unwrap().unwrap();
        assert_eq!(resolved, first);
        // Prove the eviction actually happened: drop the blob and re-fill the
        // cache past the cap again; the re-resolve must now miss BOTH.
        store
            .delete(&ContentRef::from_bytes(*first.hash().as_bytes()))
            .unwrap();
        for i in 0..=DEF_CACHE_CAP {
            let mut d = def(2);
            d.config_subset
                .insert(ConfigKey("j".into()), ConfigVal(i.to_le_bytes().to_vec()));
            store.put(&d.encode()).unwrap();
            view.get_def(d.hash().as_bytes()).unwrap().unwrap();
        }
        assert!(view.get_def(first.hash().as_bytes()).unwrap().is_none());
    }
}
