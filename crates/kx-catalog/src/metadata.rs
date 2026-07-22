// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! Advisory catalog metadata (M7.3, D84) — discovery [`Tag`]s + an integer-scaled
//! confidence, modelled on `kx_dataset::AnnotationStore`.
//!
//! # The wall (SN-8, load-bearing)
//!
//! This projection is **off the trust path**. It is NEVER journaled, NEVER on the
//! identity / commit / memoization path, and it NEVER gates selection, eviction,
//! or promotion. Its sole job is to feed an advisory signal into a model's
//! *catalog* context (tags to discover by, a confidence to rank a *proposal* by) —
//! curation, not a fact. The boundary is enforced by the dependency graph (the
//! guarantee-path crates do not depend on `kx-catalog`), exactly as
//! `kx_dataset::AnnotationStore` documents; this module re-keys that same shape
//! from `ContentRef` to the catalog's [`AssetRef`] and adds a tag index.
//!
//! # No floats
//!
//! Confidence is **integer-scaled** ([`AdvisoryMetadata::confidence_scaled`] is
//! `i64`, e.g. basis points). No float ever touches this projection, so even a
//! future mistake that wired it toward a decision path would carry none onto a
//! canonical hash.
//!
//! # A rebuildable projection
//!
//! Iteration is in [`AssetRef`] order (`BTreeMap`), so a rebuild is deterministic.
//! The inverted tag index is kept in lock-step with the forward map by every
//! mutating method, so [`AdvisoryMetadataStore::assets_with_tag`] is
//! `O(log n + result)` — never an O(n) scan.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::path::AssetRef;

/// Maximum length, in bytes, of one discovery [`Tag`]. A bound keeps the inverted
/// index key small and rejects pathological input (mirrors [`crate::MAX_SEGMENT_LEN`]).
pub const MAX_TAG_LEN: usize = 64;

/// Maximum number of distinct tags one asset may carry — a hard fan-out bound on
/// the inverted index. Exceeding it is a loud, fail-closed refusal (never a silent
/// truncation).
pub const MAX_TAGS_PER_ASSET: usize = 256;

/// A canonical, validated discovery tag. Construction ([`Tag::new`]) is the SOLE
/// validation gate, so an illegal tag is **unrepresentable** downstream — the same
/// discipline as [`crate::AssetPath`]'s segments.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct Tag(String);

/// Why a tag (or a tag set) was rejected. Loud, typed refusal — never a silently
/// coerced or truncated tag.
#[derive(Clone, PartialEq, Eq, Debug, thiserror::Error)]
pub enum TagError {
    /// The tag was empty.
    #[error("tag must not be empty")]
    Empty,
    /// The tag exceeded [`MAX_TAG_LEN`] bytes.
    #[error("tag is {0} bytes (max {MAX_TAG_LEN})")]
    TooLong(usize),
    /// The tag carried a character outside the canonical `[a-z0-9._-]` class.
    #[error("tag contains illegal character {0:?} (allowed: [a-z0-9._-])")]
    IllegalChar(char),
    /// An asset's tag set exceeded [`MAX_TAGS_PER_ASSET`].
    #[error("asset carries {0} tags (max {MAX_TAGS_PER_ASSET})")]
    TooManyTags(usize),
}

impl Tag {
    /// Construct + validate. The ONLY way to build a `Tag`, so any `Tag` value is
    /// canonical: non-empty, at most [`MAX_TAG_LEN`] bytes, drawn from the
    /// lowercase class `[a-z0-9._-]`.
    ///
    /// # Errors
    ///
    /// Returns the first [`TagError`] encountered.
    pub fn new(s: impl Into<String>) -> Result<Self, TagError> {
        let s = s.into();
        if s.is_empty() {
            return Err(TagError::Empty);
        }
        if s.len() > MAX_TAG_LEN {
            return Err(TagError::TooLong(s.len()));
        }
        for ch in s.chars() {
            if !(ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '.' | '_' | '-')) {
                return Err(TagError::IllegalChar(ch));
            }
        }
        Ok(Self(s))
    }

    /// Borrow the tag's canonical string.
    #[inline]
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Tag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// One advisory curation record for a catalog asset. Integer-scaled, mutable,
/// rebuildable; NEVER journaled, NEVER an identity input, NEVER gates anything.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct AdvisoryMetadata {
    /// Confidence, **integer-scaled** (e.g. basis points, `0..=10_000`). Never a
    /// float — advisory ranking input for a *proposal*, never a gate.
    pub confidence_scaled: i64,
    /// The canonical discovery tag set (`BTreeSet` → deterministic iteration).
    pub tags: BTreeSet<Tag>,
    /// Opaque curator identity (free-form; advisory only).
    pub curated_by: String,
    /// Free-form curator notes (advisory only).
    pub notes: String,
}

/// A mutable, rebuildable advisory projection keyed by [`AssetRef`], with an
/// inverted `tag → assets` index for tag-based discovery.
///
/// Advisory only — see the module-level wall. Iteration is in [`AssetRef`] order
/// (`BTreeMap`), so a rebuild is deterministic. The inverted index is kept in sync
/// with the forward map by [`AdvisoryMetadataStore::set`] /
/// [`AdvisoryMetadataStore::remove`].
#[derive(Clone, Debug, Default)]
pub struct AdvisoryMetadataStore {
    by_ref: BTreeMap<AssetRef, AdvisoryMetadata>,
    by_tag: BTreeMap<Tag, BTreeSet<AssetRef>>,
}

impl AdvisoryMetadataStore {
    /// Create an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert or overwrite the advisory metadata for `asset`, keeping the inverted
    /// tag index in sync.
    ///
    /// # Errors
    ///
    /// Returns [`TagError::TooManyTags`] (fail-closed, no partial write) if the
    /// record carries more than [`MAX_TAGS_PER_ASSET`] tags.
    pub fn set(&mut self, asset: AssetRef, meta: AdvisoryMetadata) -> Result<(), TagError> {
        if meta.tags.len() > MAX_TAGS_PER_ASSET {
            return Err(TagError::TooManyTags(meta.tags.len()));
        }
        // Purge the prior record's inverted-index entries (clone the prior tag set
        // first so the forward map is not borrowed while the inverted map mutates).
        if let Some(old_tags) = self.by_ref.get(&asset).map(|m| m.tags.clone()) {
            for t in &old_tags {
                self.unindex_tag(t, &asset);
            }
        }
        for t in &meta.tags {
            self.by_tag
                .entry(t.clone())
                .or_default()
                .insert(asset.clone());
        }
        self.by_ref.insert(asset, meta);
        Ok(())
    }

    /// Read the advisory metadata for `asset`, if any.
    #[must_use]
    pub fn get(&self, asset: &AssetRef) -> Option<&AdvisoryMetadata> {
        self.by_ref.get(asset)
    }

    /// Remove and return the advisory metadata for `asset`, purging its inverted
    /// tag index entries.
    pub fn remove(&mut self, asset: &AssetRef) -> Option<AdvisoryMetadata> {
        let removed = self.by_ref.remove(asset)?;
        for t in &removed.tags {
            self.unindex_tag(t, asset);
        }
        Some(removed)
    }

    /// Drop `asset` from `tag`'s inverted-index bucket, removing the bucket when it
    /// empties (so iteration never sees a dangling tag).
    fn unindex_tag(&mut self, tag: &Tag, asset: &AssetRef) {
        if let Some(set) = self.by_tag.get_mut(tag) {
            set.remove(asset);
            if set.is_empty() {
                self.by_tag.remove(tag);
            }
        }
    }

    /// Every asset carrying `tag`, in [`AssetRef`] order. `O(log n + result)` via
    /// the inverted index — never an O(n) scan.
    pub fn assets_with_tag<'a>(&'a self, tag: &Tag) -> impl Iterator<Item = &'a AssetRef> + 'a {
        self.by_tag.get(tag).into_iter().flat_map(BTreeSet::iter)
    }

    /// Number of assets with advisory metadata.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_ref.len()
    }

    /// `true` if no asset has advisory metadata.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_ref.is_empty()
    }

    /// Iterate `(asset, metadata)` in [`AssetRef`] order (deterministic rebuild).
    pub fn iter(&self) -> impl Iterator<Item = (&AssetRef, &AdvisoryMetadata)> {
        self.by_ref.iter()
    }
}
