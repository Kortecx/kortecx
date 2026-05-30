// SPDX-License-Identifier: Apache-2.0
//! [`AnnotationStore`] — an advisory, mutable, rebuildable curation projection
//! keyed by [`ContentRef`].
//!
//! # The wall (SN-8, load-bearing)
//!
//! This projection is **off the truth path**. It is NEVER journaled, NEVER on the
//! identity / commit / memoization path, and it NEVER gates runtime execution. Its
//! sole job is to feed an advisory quality signal into a model's *catalog* context
//! ("agent A does X at Y% accuracy") — human/agent curation, not a fact.
//!
//! The boundary is enforced **by the dependency graph, not by this comment**:
//! `AnnotationStore` lives in `kx-dataset`, a crate the guarantee-path crates
//! (`kx-executor`, `kx-projection`, `kx-scheduler`) structurally do not depend on,
//! so the compiler rejects any attempt to import it onto the guarantee path on
//! every build. The `tests/annotation_boundary.rs` lint is a tripwire on top of
//! that wall — the wall is the dependency direction.
//!
//! **The temptation to move this layer "closer" to the executor for convenience is
//! itself the SN-8 violation.** A usefulness score is a similarity-flavoured signal;
//! the moment it gates selection, eviction, or promotion it has crossed onto the
//! trust path that SN-8 reserves for exact cryptographic equality.
//!
//! # No floats
//!
//! Evidence is **integer-scaled** ([`Annotation::usefulness_scaled`] is `i64`). No
//! float ever touches this projection, so even a future mistake that wired it toward
//! a decision path would carry no float to taint a canonical hash.
//!
//! # Immutability of facts (D-LOCK-4)
//!
//! Facts (committed blobs) are immutable, period — there is no edit/version path.
//! The only curation primitive is the yes/no rating ([`Annotation::yes_no`]); this
//! store annotates *about* a fact by its content ref, it never mutates the fact.

use std::collections::BTreeMap;

use kx_content::ContentRef;
use serde::{Deserialize, Serialize};

/// One advisory curation record for a committed payload. Mutable and rebuildable;
/// never a journaled fact.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Annotation {
    /// Usefulness, **integer-scaled** (e.g. basis points, `0..=10_000`). Never a float.
    pub usefulness_scaled: i64,
    /// The yes/no rating — the only curation primitive (D-LOCK-4: facts are
    /// immutable; rating is the curation surface).
    pub yes_no: bool,
    /// Opaque reviewer identity (free-form; advisory only).
    pub reviewed_by: String,
    /// Free-form reviewer notes (advisory only).
    pub notes: String,
}

/// A mutable, rebuildable projection of [`Annotation`]s keyed by [`ContentRef`].
///
/// Advisory only — see the module-level wall. Iteration is in `ContentRef` order
/// (`BTreeMap`) so a rebuild is deterministic.
#[derive(Clone, Debug, Default)]
pub struct AnnotationStore {
    by_ref: BTreeMap<ContentRef, Annotation>,
}

impl AnnotationStore {
    /// Create an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert or overwrite the annotation for `content_ref`. Mutable by design — a
    /// reviewer may freely override a prior rating; this touches no fact.
    pub fn set(&mut self, content_ref: ContentRef, annotation: Annotation) {
        self.by_ref.insert(content_ref, annotation);
    }

    /// Read the annotation for `content_ref`, if any.
    #[must_use]
    pub fn get(&self, content_ref: &ContentRef) -> Option<&Annotation> {
        self.by_ref.get(content_ref)
    }

    /// Remove and return the annotation for `content_ref`, if any.
    pub fn remove(&mut self, content_ref: &ContentRef) -> Option<Annotation> {
        self.by_ref.remove(content_ref)
    }

    /// Number of annotated refs.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_ref.len()
    }

    /// `true` if no refs are annotated.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_ref.is_empty()
    }

    /// Iterate `(content_ref, annotation)` in `ContentRef` order.
    pub fn iter(&self) -> impl Iterator<Item = (&ContentRef, &Annotation)> {
        self.by_ref.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::{Annotation, AnnotationStore, ContentRef};

    fn r(b: u8) -> ContentRef {
        ContentRef::from_bytes([b; 32])
    }

    fn ann(score: i64, yes: bool) -> Annotation {
        Annotation {
            usefulness_scaled: score,
            yes_no: yes,
            reviewed_by: "tester".into(),
            notes: String::new(),
        }
    }

    #[test]
    fn set_get_roundtrip() {
        let mut s = AnnotationStore::new();
        assert!(s.is_empty());
        s.set(r(1), ann(7_500, true));
        assert_eq!(s.len(), 1);
        let a = s.get(&r(1)).unwrap();
        assert_eq!(a.usefulness_scaled, 7_500);
        assert!(a.yes_no);
        assert!(s.get(&r(2)).is_none());
    }

    #[test]
    fn set_overwrites_mutably() {
        // Mutability is the point: a reviewer can override a prior rating freely; no
        // fact moves — only this advisory projection changes.
        let mut s = AnnotationStore::new();
        s.set(r(1), ann(1_000, false));
        s.set(r(1), ann(9_000, true));
        assert_eq!(s.len(), 1);
        let a = s.get(&r(1)).unwrap();
        assert_eq!(a.usefulness_scaled, 9_000);
        assert!(a.yes_no);
    }

    #[test]
    fn remove_returns_and_clears() {
        let mut s = AnnotationStore::new();
        s.set(r(3), ann(42, true));
        assert_eq!(s.remove(&r(3)).unwrap().usefulness_scaled, 42);
        assert!(s.remove(&r(3)).is_none());
        assert!(s.is_empty());
    }

    #[test]
    fn iter_is_content_ref_ordered() {
        let mut s = AnnotationStore::new();
        s.set(r(9), ann(1, true));
        s.set(r(2), ann(2, true));
        s.set(r(5), ann(3, true));
        let keys: Vec<u8> = s.iter().map(|(k, _)| k.as_bytes()[0]).collect();
        assert_eq!(
            keys,
            vec![2, 5, 9],
            "BTreeMap iteration → deterministic ContentRef order (rebuildable projection)"
        );
    }
}
