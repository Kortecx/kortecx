//! [`DataStore`] — the pluggable, journal-authoritative data-management seam —
//! and [`Dataset`] — a content-addressed corpus of typed rows with lineage.
//!
//! **Journal-authoritative.** A `DataStore` is a reconstructible, queryable
//! projection/cache of *committed* content — never a second source of truth.
//! Lose the store and it rebuilds by re-folding committed content; correctness
//! lives in the journal (D40) + content-addressing (D17), not here. Accordingly
//! a [`Dataset`]'s identity ([`Dataset::id`]) is a **pure function of its content
//! refs + lineage** — independent of any store instance — so a corpus
//! regenerated on another machine has the same `DatasetId`.
//!
//! [`InMemoryDataStore`] is the OSS-default backend; a Lance backend
//! (vectors + tensors + blobs + Delta versioning) is a later gated step behind
//! this same trait.

use std::collections::BTreeMap;
use std::sync::Mutex;

use kx_content::ContentRef;
use kx_mote::MoteId;
use serde::{Deserialize, Serialize};

use crate::error::DataError;
use crate::schema::{ContentSchema, TypedRef};

/// A pluggable typed store over content-addressed payloads. Methods take `&self`
/// (interior mutability) so a store can be shared (`Arc<dyn DataStore>`) by, e.g.,
/// a retrieval Mote — without granting it write authority over the journal.
pub trait DataStore {
    /// Store `bytes` tagged with `schema`, returning the content-addressed
    /// [`TypedRef`]. Idempotent on the bytes (same bytes → same ref).
    ///
    /// # Errors
    /// [`DataError::Poisoned`] if the store's lock was poisoned.
    fn put_typed(&self, bytes: &[u8], schema: ContentSchema) -> Result<TypedRef, DataError>;

    /// Read the payload + schema at `r`.
    ///
    /// # Errors
    /// [`DataError::NotFound`] if absent; [`DataError::Poisoned`] on a poisoned lock.
    fn get(&self, r: &ContentRef) -> Result<(Vec<u8>, ContentSchema), DataError>;

    /// The schema declared for `r`, if present.
    fn schema_of(&self, r: &ContentRef) -> Option<ContentSchema>;

    /// `true` if a payload is stored at `r`.
    fn contains(&self, r: &ContentRef) -> bool;
}

/// A 32-byte content-addressed identity of a [`Dataset`].
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct DatasetId(pub [u8; 32]);

impl DatasetId {
    /// Lowercase 64-char hex.
    #[must_use]
    pub fn to_hex(self) -> String {
        blake3::Hash::from_bytes(self.0).to_hex().to_string()
    }
}

impl std::fmt::Debug for DatasetId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "DatasetId({})",
            blake3::Hash::from_bytes(self.0).to_hex()
        )
    }
}

/// A content-addressed corpus: an ordered set of typed rows plus the Motes that
/// produced it (lineage/provenance). Its [`Dataset::id`] is pure over the rows +
/// lineage, so the corpus is reproducible-by-reference (the recipe-as-product /
/// Delta-sharing basis, P4.1e).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dataset {
    /// The typed rows, in their canonical (identity-bearing) order.
    pub rows: Vec<TypedRef>,
    /// The Motes whose committed output produced this corpus (provenance).
    pub lineage: Vec<MoteId>,
}

impl Dataset {
    /// Build a dataset from its rows + lineage.
    #[must_use]
    pub fn new(rows: Vec<TypedRef>, lineage: Vec<MoteId>) -> Self {
        Self { rows, lineage }
    }

    /// The content-addressed identity — a **pure** function of rows + lineage.
    /// Two byte-identical corpora (anywhere, any machine) share a `DatasetId`.
    #[must_use]
    pub fn id(&self) -> DatasetId {
        let mut h = blake3::Hasher::new();
        h.update(b"kx-dataset/dataset-id/v1");
        h.update(&(self.rows.len() as u64).to_le_bytes());
        for row in &self.rows {
            h.update(row.content_ref.as_bytes());
            row.schema.hash_into(&mut h);
        }
        h.update(&(self.lineage.len() as u64).to_le_bytes());
        for mote in &self.lineage {
            h.update(mote.as_bytes());
        }
        DatasetId(*h.finalize().as_bytes())
    }

    /// Number of rows.
    #[must_use]
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// `true` if the corpus has no rows.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

/// The OSS-default in-memory [`DataStore`]. A `BTreeMap` keyed by content ref;
/// suitable for tests and single-process runs. Drop it and rebuild from
/// committed content — nothing authoritative lives here.
#[derive(Default)]
pub struct InMemoryDataStore {
    inner: Mutex<BTreeMap<ContentRef, (Vec<u8>, ContentSchema)>>,
}

impl InMemoryDataStore {
    /// Create an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl DataStore for InMemoryDataStore {
    fn put_typed(&self, bytes: &[u8], schema: ContentSchema) -> Result<TypedRef, DataError> {
        let content_ref = ContentRef::of(bytes);
        let mut guard = self.inner.lock().map_err(|_| DataError::Poisoned)?;
        guard.insert(content_ref, (bytes.to_vec(), schema.clone()));
        Ok(TypedRef {
            content_ref,
            schema,
        })
    }

    fn get(&self, r: &ContentRef) -> Result<(Vec<u8>, ContentSchema), DataError> {
        let guard = self.inner.lock().map_err(|_| DataError::Poisoned)?;
        guard.get(r).cloned().ok_or(DataError::NotFound)
    }

    fn schema_of(&self, r: &ContentRef) -> Option<ContentSchema> {
        let guard = self.inner.lock().ok()?;
        guard.get(r).map(|(_, schema)| schema.clone())
    }

    fn contains(&self, r: &ContentRef) -> bool {
        self.inner
            .lock()
            .map(|g| g.contains_key(r))
            .unwrap_or(false)
    }
}
