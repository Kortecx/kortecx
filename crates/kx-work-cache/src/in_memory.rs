//! `InMemoryWorkCache` — a process-local [`WorkCache`] for tests and for a serve that
//! wants cross-run dedup within one process without a durable sidecar. Vanishes on
//! drop; rebuilds implicitly as work re-commits.

use std::collections::BTreeMap;
use std::sync::Mutex;

use kx_content::ContentRef;
use kx_mote::{MoteId, NdClass};

use crate::{WorkCache, WorkFingerprint};

/// An in-memory, first-writer-wins [`WorkCache`] backed by a `Mutex<BTreeMap>`.
///
/// `BTreeMap` (not `HashMap`) keeps iteration order deterministic, matching the
/// codebase's reproducibility discipline for any future debug dump.
#[derive(Debug, Default)]
pub struct InMemoryWorkCache {
    inner: Mutex<BTreeMap<WorkFingerprint, ContentRef>>,
}

impl InMemoryWorkCache {
    /// Construct an empty cache.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The number of cached entries. Test/observability convenience.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }

    /// `true` iff the cache holds no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl WorkCache for InMemoryWorkCache {
    fn lookup(&self, fp: &WorkFingerprint) -> Option<ContentRef> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(fp)
            .copied()
    }

    fn insert(&self, fp: WorkFingerprint, result_ref: ContentRef, _nd: NdClass, _source: MoteId) {
        // First-writer-wins: entry() + or_insert keeps the earliest ref. For a PURE
        // fingerprint the ref is deterministic, so this only matters under a race.
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entry(fp)
            .or_insert(result_ref);
    }

    fn evict(&self, fp: &WorkFingerprint) {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(fp);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kx_mote::{InputDataId, MoteDefHash};

    fn fp(b: u8) -> WorkFingerprint {
        crate::work_fingerprint(
            NdClass::Pure,
            &MoteDefHash::from_bytes([b; 32]),
            &InputDataId::from_bytes([b; 32]),
        )
    }

    #[test]
    fn insert_then_lookup_round_trips() {
        let cache = InMemoryWorkCache::new();
        let r = ContentRef::of(b"result");
        assert!(cache.lookup(&fp(1)).is_none());
        cache.insert(fp(1), r, NdClass::Pure, MoteId::from_bytes([0; 32]));
        assert_eq!(cache.lookup(&fp(1)), Some(r));
    }

    #[test]
    fn first_writer_wins() {
        let cache = InMemoryWorkCache::new();
        let first = ContentRef::of(b"first");
        let second = ContentRef::of(b"second");
        cache.insert(fp(1), first, NdClass::Pure, MoteId::from_bytes([0; 32]));
        cache.insert(fp(1), second, NdClass::Pure, MoteId::from_bytes([1; 32]));
        assert_eq!(
            cache.lookup(&fp(1)),
            Some(first),
            "second insert must not clobber"
        );
    }

    #[test]
    fn evict_removes() {
        let cache = InMemoryWorkCache::new();
        let r = ContentRef::of(b"result");
        cache.insert(fp(1), r, NdClass::Pure, MoteId::from_bytes([0; 32]));
        cache.evict(&fp(1));
        assert!(cache.lookup(&fp(1)).is_none());
    }
}
