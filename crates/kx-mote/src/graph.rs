//! [`crate::MoteGraph`] — minimal workflow-author-side container for composing a
//! Mote DAG.

use std::collections::BTreeMap;

use smallvec::SmallVec;

use crate::edge::ParentRef;
use crate::id::MoteId;
use crate::mote::Mote;

// ---------------------------------------------------------------------------
// MoteGraph — workflow-author-side container
// ---------------------------------------------------------------------------

/// A workflow-author-side container of Motes and their declared edges.
///
/// This is a *compile-time* shape — the structure a workflow author (or the
/// P4 SDK) builds before any Mote runs. The runtime never reads this; at
/// execution time the projection (P1.5) folds the journal log into the live
/// graph view. This type exists so workflow code has a typed handle for
/// composition and so unit tests have a convenient builder.
///
/// **Contains no traversal logic** — that lives in `kx-projection`. The
/// shape is plain data: keyed nodes and adjacency lists.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MoteGraph {
    /// All Motes in the graph, keyed by identity.
    pub nodes: BTreeMap<MoteId, Mote>,
    /// Adjacency: for each child, the list of declared parent edges.
    /// Mirrors what lands in each Mote's `parents` field at commit time.
    pub edges: BTreeMap<MoteId, SmallVec<[ParentRef; 4]>>,
}

impl MoteGraph {
    /// Create an empty graph.
    #[inline]
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a Mote and its declared parents. Overwrites any existing entry
    /// for the same `MoteId` (the caller is responsible for the uniqueness
    /// guarantee at compose time; the journal enforces it at runtime via
    /// dedupe-by-key).
    pub fn insert(&mut self, mote: Mote) {
        let id = mote.id;
        self.edges.insert(id, mote.parents.clone());
        self.nodes.insert(id, mote);
    }

    /// Number of Motes in the graph.
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// `true` if the graph contains no Motes.
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Borrow a Mote by identity.
    #[inline]
    #[must_use]
    pub fn get(&self, id: &MoteId) -> Option<&Mote> {
        self.nodes.get(id)
    }

    /// Borrow the declared parent edges of a Mote.
    #[inline]
    #[must_use]
    pub fn parents_of(&self, id: &MoteId) -> Option<&SmallVec<[ParentRef; 4]>> {
        self.edges.get(id)
    }
}
