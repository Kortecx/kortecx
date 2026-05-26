//! Typed dependency edges between Motes: [`crate::EdgeKind`] (Data / Control),
//! [`crate::EdgeMeta`] (per-edge metadata), [`crate::ParentRef`] (one parent + edge).

use serde::{Deserialize, Serialize};

use crate::id::MoteId;

// ---------------------------------------------------------------------------
// Edges — typed dependencies between Motes
// ---------------------------------------------------------------------------

/// The type of a directed dependency edge between two Motes (`mote.md` §5, D6).
///
/// Stable u8 representations are used in journal `ParentEntry` encoding
/// (Data=0, Control=1) — these MUST NOT change without a journal
/// `schema_version` bump.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum EdgeKind {
    /// A's `ContentRef` is in B's `input_data_id`. Repudiating A always cascades
    /// to B (no opt-out — B read an invalidated input).
    Data = 0,

    /// A must be Committed before B runs, but A's output is not consumed by B.
    /// Cascades by default (the asymmetry rule, D7); per-edge author-declared
    /// causation-only opt-out via [`EdgeMeta::non_cascade`].
    Control = 1,
}

impl EdgeKind {
    /// Convert to the canonical u8 representation used by the journal.
    #[inline]
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        self as u8
    }
}

/// Per-edge metadata attached to a dependency edge (`projection.md` §7;
/// matches `journal-entry.md` §5 `ParentEntry`).
///
/// `non_cascade` is the per-edge author-declared causation-only opt-out, valid
/// **only** when `kind == EdgeKind::Control`. The encoder for journal entries
/// asserts `non_cascade == false` for Data edges (anti-pattern #6 in
/// `journal-entry.md` §11) — this crate provides constructors that uphold the
/// rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct EdgeMeta {
    /// The kind of dependency this edge expresses.
    pub kind: EdgeKind,
    /// Whether this edge is exempt from the repudiation cascade. Only valid
    /// when `kind == EdgeKind::Control`. Always `false` for Data edges.
    pub non_cascade: bool,
}

impl EdgeMeta {
    /// Construct a Data edge (always cascades).
    #[inline]
    #[must_use]
    pub const fn data() -> Self {
        Self {
            kind: EdgeKind::Data,
            non_cascade: false,
        }
    }

    /// Construct a Control edge that cascades on repudiation (the default).
    #[inline]
    #[must_use]
    pub const fn control() -> Self {
        Self {
            kind: EdgeKind::Control,
            non_cascade: false,
        }
    }

    /// Construct a Control edge with the per-edge non-cascade opt-out.
    ///
    /// Use this only when the workflow author has explicitly decided that
    /// repudiating the parent should NOT invalidate the child — a real but
    /// exceptional category (`mote.md` §5, D7). This is a reviewed act.
    #[inline]
    #[must_use]
    pub const fn control_non_cascading() -> Self {
        Self {
            kind: EdgeKind::Control,
            non_cascade: true,
        }
    }
}

/// A reference to a parent Mote within a [`crate::Mote`]'s declared dependencies.
///
/// Mirrors the on-disk `ParentEntry` shape (`journal-entry.md` §5, D19) at
/// the type level: a parent's `MoteId` plus its edge metadata. The journal
/// entry's `parents` field is a `SmallVec<[ParentRef; 4]>` in code; the
/// stack-inline storage covers the typical 0–4-parent case.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ParentRef {
    /// The parent Mote's identity.
    pub parent_id: MoteId,
    /// The edge connecting this parent to the child.
    pub edge: EdgeMeta,
}
