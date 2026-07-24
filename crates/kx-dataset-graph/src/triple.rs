// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! [`Triple`] — one extracted `(subject, predicate, object)` fact tagged with the
//! source chunk it came from — and [`normalize_entity`], the deterministic node
//! identity a subject/object collapses to.

use kx_content::ContentRef;

/// A subject–predicate–object fact extracted from a corpus chunk.
///
/// The three strings are stored RAW (display/provenance); a subject or object is
/// resolved to a graph NODE via [`normalize_entity`] at index/query time, so two
/// spellings of the same entity (`"Acme Corp"` / `" acme  corp "`) collapse to one
/// node. `source` is the content-addressed identity of the chunk the fact was
/// extracted from — what a graph walk returns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Triple {
    /// The subject entity, raw as extracted.
    pub subject: String,
    /// The relation/predicate, raw as extracted.
    pub predicate: String,
    /// The object entity, raw as extracted.
    pub object: String,
    /// The source chunk this fact was extracted from.
    pub source: ContentRef,
}

impl Triple {
    /// Build a triple from its parts.
    pub fn new(
        subject: impl Into<String>,
        predicate: impl Into<String>,
        object: impl Into<String>,
        source: ContentRef,
    ) -> Self {
        Self {
            subject: subject.into(),
            predicate: predicate.into(),
            object: object.into(),
            source,
        }
    }
}

/// The node identity a raw entity string collapses to: whitespace-collapsed and
/// lowercased. Deterministic and locale-independent (`str::to_lowercase` is
/// Unicode-defined, not locale-sensitive), so the same entity produces the same
/// node on any machine — a prerequisite for a reproducible neighbour walk.
#[must_use]
pub fn normalize_entity(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}
