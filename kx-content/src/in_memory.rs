//! An in-memory [`ContentStore`] backend.
//!
//! Exists for two reasons:
//!
//! 1. **Trait-seam proof.** A second backend with a different storage substrate proves the
//!    [`ContentStore`] trait carries no in-process-filesystem assumption in its signature.
//!    This is the lightweight version of test obligation #6 from `content-store.md` §10
//!    ("a fake/in-memory backend compiles against the trait with no in-process-specific
//!    signature dependencies").
//! 2. **Cheap, deterministic fixtures for downstream tests.** The journal (P1.4),
//!    projection (P1.5), and executor (P1.9) test suites can use this backend without
//!    touching the filesystem.
//!
//! **Not for production.** No durability, no persistence across process restarts, no
//! retention discipline. Use [`crate::LocalFsContentStore`] or a future cloud impl in any
//! deployment.

use std::collections::HashMap;
use std::sync::RwLock;

use bytes::Bytes;

use crate::{ContentRef, ContentStore, NotFound, StoreError};

/// An ephemeral, process-local [`ContentStore`]. Backed by a [`HashMap<ContentRef, Bytes>`]
/// under a [`RwLock`]; supports multiple readers and one writer.
#[derive(Debug, Default)]
pub struct InMemoryContentStore {
    objects: RwLock<HashMap<ContentRef, Bytes>>,
}

impl InMemoryContentStore {
    /// Construct an empty store.
    #[inline]
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of distinct refs currently stored. Useful for asserting dedup in tests.
    #[must_use]
    pub fn len(&self) -> usize {
        self.objects.read().expect("poisoned lock").len()
    }

    /// `true` when the store has no objects.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl ContentStore for InMemoryContentStore {
    type Payload = Bytes;

    fn put(&self, bytes: &[u8]) -> Result<ContentRef, StoreError> {
        let r = ContentRef::of(bytes);
        let mut guard = self.objects.write().expect("poisoned lock");
        // Idempotent: identical bytes share one entry.
        guard
            .entry(r)
            .or_insert_with(|| Bytes::copy_from_slice(bytes));
        Ok(r)
    }

    fn get(&self, r: &ContentRef) -> Result<Self::Payload, NotFound> {
        let guard = self.objects.read().expect("poisoned lock");
        guard.get(r).cloned().ok_or(NotFound)
    }

    fn delete(&self, r: &ContentRef) -> Result<(), StoreError> {
        let mut guard = self.objects.write().expect("poisoned lock");
        guard.remove(r);
        Ok(())
    }

    fn list_refs<'a>(&'a self) -> Box<dyn Iterator<Item = ContentRef> + 'a> {
        let guard = self.objects.read().expect("poisoned lock");
        let refs: Vec<ContentRef> = guard.keys().copied().collect();
        Box::new(refs.into_iter())
    }

    fn contains(&self, r: &ContentRef) -> bool {
        let guard = self.objects.read().expect("poisoned lock");
        guard.contains_key(r)
    }
}
