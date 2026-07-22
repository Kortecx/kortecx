// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! [`InMemoryCatalog`] — the reference [`CatalogRegistry`] backend.
//!
//! A `BTreeMap<TaskSignatureHash, SignatureEntry>` under an [`RwLock`]: O(log n)
//! `lookup` / `get`, sub-linear amortized `register`, deterministic (hash-order)
//! enumeration. Process-local and rebuildable — not for production durability
//! (a persistent backend implements the same trait). Exists primarily to prove
//! [`CatalogRegistry`] carries no storage-substrate assumption (the role
//! `kx_content::InMemoryContentStore` plays for `ContentStore`).

use std::collections::btree_map::Entry;
use std::collections::BTreeMap;
use std::sync::RwLock;

use crate::entry::SignatureEntry;
use crate::registry::{CatalogError, CatalogRegistry, RegistrationOutcome};
use crate::signature::TaskSignatureHash;

/// An ephemeral, process-local [`CatalogRegistry`]. Multiple readers, one writer.
///
/// # Examples
///
/// ```
/// use kx_catalog::{CatalogRegistry, InMemoryCatalog, RecipeSnapshot, SignatureEntry, TaskSignature};
/// use kx_mote::MoteDefHash;
/// use kx_workflow::ManifestId;
///
/// let catalog = InMemoryCatalog::new();
/// assert!(catalog.is_empty());
///
/// let sig = TaskSignature::model_invariant(MoteDefHash::from_bytes([7u8; 32]));
/// let entry = SignatureEntry::new(sig, ManifestId([1u8; 32]), RecipeSnapshot::new([2u8; 32]));
/// let hash = entry.hash();
///
/// let outcome = catalog.register_signature(entry.clone()).unwrap();
/// assert!(outcome.is_inserted());
/// assert_eq!(catalog.len(), 1);
///
/// // Idempotent: re-registering the same entry is a no-op.
/// let again = catalog.register_signature(entry).unwrap();
/// assert!(!again.is_inserted());
/// assert_eq!(catalog.len(), 1);
///
/// assert_eq!(catalog.get_signature(&hash).unwrap().hash(), hash);
/// ```
#[derive(Debug, Default)]
pub struct InMemoryCatalog {
    by_hash: RwLock<BTreeMap<TaskSignatureHash, SignatureEntry>>,
}

impl InMemoryCatalog {
    /// Construct an empty catalog.
    #[inline]
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl CatalogRegistry for InMemoryCatalog {
    fn register_signature(
        &self,
        entry: SignatureEntry,
    ) -> Result<RegistrationOutcome, CatalogError> {
        let hash = entry.hash();
        // The Entry API avoids holding a borrow across the insert (no double-borrow).
        let mut guard = self.by_hash.write().expect("poisoned lock");
        match guard.entry(hash) {
            Entry::Occupied(occupied) => {
                if *occupied.get() == entry {
                    Ok(RegistrationOutcome::AlreadyPresent(hash))
                } else {
                    // Same hash, different bytes: refuse (immutability).
                    Err(CatalogError::ImmutabilityConflict(hash.to_hex()))
                }
            }
            Entry::Vacant(vacant) => {
                vacant.insert(entry);
                tracing::debug!(task_signature_hash = %hash, "registered catalog signature");
                Ok(RegistrationOutcome::Inserted(hash))
            }
        }
    }

    fn lookup(&self, hash: &TaskSignatureHash) -> Option<SignatureEntry> {
        self.by_hash
            .read()
            .expect("poisoned lock")
            .get(hash)
            .cloned()
    }

    fn list_signatures<'a>(&'a self) -> Box<dyn Iterator<Item = SignatureEntry> + 'a> {
        let guard = self.by_hash.read().expect("poisoned lock");
        // Snapshot under the read lock (hash order), then release before iterating.
        let entries: Vec<SignatureEntry> = guard.values().cloned().collect();
        Box::new(entries.into_iter())
    }

    fn len(&self) -> usize {
        self.by_hash.read().expect("poisoned lock").len()
    }
}
