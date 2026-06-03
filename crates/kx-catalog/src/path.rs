// SPDX-License-Identifier: Apache-2.0
//! Catalog namespacing (M7.2, D86) — an asset lives at `namespace/collection/name`.
//!
//! [`AssetPath`] is the Unity-Catalog analog (`catalog → schema → object`). Its
//! three segments are validated to one canonical lowercase byte-representation,
//! so two logically-equal paths share a hash and an authorization key. An
//! [`AssetRef`] addresses a governed asset either by its human path or by a
//! content-addressed [`TaskSignatureHash`] (a recipe), so a grant can name a
//! recipe directly without a path indirection.
//!
//! Construction is the SOLE validation gate ([`AssetPath::new`]): an illegal
//! path is **unrepresentable** downstream — there is no other way to build one.

use serde::{Deserialize, Serialize};

use crate::signature::TaskSignatureHash;

/// Maximum length, in bytes, of any single path segment. A bound keeps a path
/// (and its content-addressed key) small and rejects pathological input.
pub const MAX_SEGMENT_LEN: usize = 128;

/// A namespaced catalog path: `namespace/collection/name`.
///
/// Each segment is non-empty, at most [`MAX_SEGMENT_LEN`] bytes, drawn from the
/// canonical class `[a-z0-9._-]` (lowercase only — one byte-representation per
/// logical path), and may not begin or end with `.` or `-` (so a segment can
/// never be read as a relative-path token). Fields are private; the only
/// constructor is [`AssetPath::new`].
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct AssetPath {
    namespace: String,
    collection: String,
    name: String,
}

/// Why an [`AssetPath`] segment was rejected. Loud, typed refusal — never a
/// silently-coerced path.
#[derive(Clone, PartialEq, Eq, Debug, thiserror::Error)]
pub enum AssetPathError {
    /// A segment was empty.
    #[error("catalog path segment `{which}` must not be empty")]
    EmptySegment {
        /// Which segment (`namespace` / `collection` / `name`).
        which: &'static str,
    },
    /// A segment carried a character outside the canonical `[a-z0-9._-]` class.
    #[error(
        "catalog path segment `{which}` contains illegal character {ch:?} (allowed: [a-z0-9._-])"
    )]
    IllegalChar {
        /// Which segment.
        which: &'static str,
        /// The offending character.
        ch: char,
    },
    /// A segment exceeded [`MAX_SEGMENT_LEN`] bytes.
    #[error("catalog path segment `{which}` is {len} bytes (max {MAX_SEGMENT_LEN})")]
    SegmentTooLong {
        /// Which segment.
        which: &'static str,
        /// The over-long length in bytes.
        len: usize,
    },
    /// A segment began or ended with `.` or `-`.
    #[error("catalog path segment `{which}` must not begin or end with `.` or `-`")]
    LeadingOrTrailingPunct {
        /// Which segment.
        which: &'static str,
    },
}

/// Validate one path segment against the canonical class + bounds.
fn validate_segment(which: &'static str, seg: &str) -> Result<(), AssetPathError> {
    if seg.is_empty() {
        return Err(AssetPathError::EmptySegment { which });
    }
    if seg.len() > MAX_SEGMENT_LEN {
        return Err(AssetPathError::SegmentTooLong {
            which,
            len: seg.len(),
        });
    }
    for ch in seg.chars() {
        if !(ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '.' | '_' | '-')) {
            return Err(AssetPathError::IllegalChar { which, ch });
        }
    }
    // `unwrap` here cannot panic: the segment is non-empty (checked above).
    let first = seg.chars().next().unwrap_or(' ');
    let last = seg.chars().next_back().unwrap_or(' ');
    if matches!(first, '.' | '-') || matches!(last, '.' | '-') {
        return Err(AssetPathError::LeadingOrTrailingPunct { which });
    }
    Ok(())
}

impl AssetPath {
    /// Construct + validate. The ONLY way to build an `AssetPath`, so any
    /// `AssetPath` value is guaranteed canonical.
    ///
    /// # Errors
    ///
    /// Returns the first [`AssetPathError`] encountered (segments validated
    /// `namespace`, then `collection`, then `name`).
    pub fn new(
        namespace: impl Into<String>,
        collection: impl Into<String>,
        name: impl Into<String>,
    ) -> Result<Self, AssetPathError> {
        let namespace = namespace.into();
        let collection = collection.into();
        let name = name.into();
        validate_segment("namespace", &namespace)?;
        validate_segment("collection", &collection)?;
        validate_segment("name", &name)?;
        Ok(Self {
            namespace,
            collection,
            name,
        })
    }

    /// The namespace segment.
    #[inline]
    #[must_use]
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// The collection segment.
    #[inline]
    #[must_use]
    pub fn collection(&self) -> &str {
        &self.collection
    }

    /// The name segment.
    #[inline]
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
}

impl std::fmt::Display for AssetPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}/{}", self.namespace, self.collection, self.name)
    }
}

/// A reference to a governed asset: a human [`AssetPath`] or a content-addressed
/// recipe [`TaskSignatureHash`]. A closed enum — a grant can be issued on either
/// without a third, untyped addressing mode.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub enum AssetRef {
    /// Addressed by its namespaced path.
    Path(AssetPath),
    /// Addressed by the content hash of a registered recipe (M7.1).
    Signature(TaskSignatureHash),
}

impl std::fmt::Display for AssetRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Path(p) => write!(f, "{p}"),
            Self::Signature(h) => write!(f, "sig:{h}"),
        }
    }
}
