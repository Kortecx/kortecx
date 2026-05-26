//! [`crate::Mote`] — the runtime-instance type. Composes a [`crate::MoteDef`] with
//! per-instance position data + parent edges + derived identity.

use smallvec::SmallVec;

use crate::def::{derive_mote_id, MoteDef};
use crate::edge::ParentRef;
use crate::effect::EffectPattern;
use crate::id::{InputDataId, MoteId};
use crate::ndclass::NdClass;
use crate::strings::GraphPosition;

// ---------------------------------------------------------------------------
// Mote — the runtime-instance shape
// ---------------------------------------------------------------------------

/// A Mote instance: a `MoteDef` paired with the per-instance position data
/// (`input_data_id`, `graph_position`) and declared parents, plus the
/// computed [`crate::MoteId`].
///
/// Construct via [`Mote::new`] to ensure `id` is derived correctly from the
/// other fields. The `id` field is exposed for read access but not for
/// independent mutation; any mutation of `def`, `input_data_id`, or
/// `graph_position` invalidates `id` and must go through reconstruction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mote {
    /// The computed identity. Read-only after construction.
    pub id: MoteId,
    /// The behavior-determining inputs.
    pub def: MoteDef,
    /// The committed-input identity (executor-derived; opaque here).
    pub input_data_id: InputDataId,
    /// This Mote's stable position in the DAG.
    pub graph_position: GraphPosition,
    /// Declared parent edges. Up to 4 parents stack-inline; spillover heap-allocates.
    pub parents: SmallVec<[ParentRef; 4]>,
}

impl Mote {
    /// Build a `Mote`, computing the [`crate::MoteId`] from its components.
    ///
    /// The constructor is the single entry point that guarantees `id`
    /// matches the rest of the struct — direct struct literal construction
    /// is supported by public fields, but callers are responsible for
    /// maintaining the invariant if they go that route.
    #[must_use]
    pub fn new(
        def: MoteDef,
        input_data_id: InputDataId,
        graph_position: GraphPosition,
        parents: SmallVec<[ParentRef; 4]>,
    ) -> Self {
        let def_hash = def.hash();
        let id = derive_mote_id(&def_hash, &input_data_id, &graph_position);
        Self {
            id,
            def,
            input_data_id,
            graph_position,
            parents,
        }
    }

    /// The Mote's non-determinism tag (mirrors `def.nd_class` for fast access).
    #[inline]
    #[must_use]
    pub const fn nd_class(&self) -> NdClass {
        self.def.nd_class
    }

    /// The Mote's declared effect pattern.
    #[inline]
    #[must_use]
    pub const fn effect_pattern(&self) -> EffectPattern {
        self.def.effect_pattern
    }
}
