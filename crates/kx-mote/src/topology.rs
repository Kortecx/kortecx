//! D37 Seam A enforcement primitives: [`RoleId`] + [`crate::ChildDescriptor`] +
//! [`crate::TopologyDecision`] (the workflow-shape enforcement primitive at the
//! foundation layer).

use serde::{Deserialize, Serialize};

use crate::def::canonical_config;
use crate::effect::EffectPattern;
use crate::id::LogicRef;
use crate::ndclass::NdClass;
use crate::strings::ConfigVal;

// ---------------------------------------------------------------------------
// D37 Seam A enforcement: TopologyDecision + ChildDescriptor + RoleId
// ---------------------------------------------------------------------------

/// Identifier for a `Role` (the RBAC template per D30; `kx_warrant::Role`).
///
/// Lives in `kx-mote` as a simple newtype to keep the foundation crate
/// dependency-free from `kx-warrant` (which sits a layer above and
/// depends on `kx-mote`). A `RoleId` is an opaque string the workflow
/// author chose; the registry layer (downstream) maps `RoleId` →
/// `kx_warrant::Role` at materialization time.
///
/// # Examples
///
/// ```
/// use kx_mote::RoleId;
///
/// let r = RoleId("critic".into());
/// assert_eq!(&r.0, "critic");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct RoleId(pub String);

/// **D37 Seam A enforcement primitive — per-child descriptor.**
///
/// One entry in a [`crate::TopologyDecision`]'s `children` vector. Declarative
/// shape only — describes what the shaper *wants* spawned; the runtime
/// materializes the concrete child `MoteDef` + `Mote` at recovery /
/// dispatch time by combining this descriptor with the parent's commit
/// graph.
///
/// **Does NOT carry `shaper_mote_id`** per D37: the shaper is implicitly
/// identified by the `result_ref` that points to the enclosing
/// [`crate::TopologyDecision`]. Embedding `shaper_mote_id` here would create
/// divergence-on-replay risk — the descriptor's content-address would
/// depend on the shaper's MoteId, which depends on the shaper's
/// `mote_def_hash`, which depends on the shaper's `MoteDef` shape; a
/// chain that's circular and fragile under replay.
///
/// **Closed payload**: the five fields below are the workflow author's (or
/// the topology-shaper model's) declarative intent for the child. The
/// runtime derives:
///
/// - `parents` from the shaper's committed graph (the shaper's own
///   committed parents form the child's data lineage)
/// - `input_data_id` from those parents' `result_ref`s
/// - `mote_def_hash` from `MoteDef(logic_ref, nd_class, effect_pattern,
///   role-derived warrant axes, `config_subset`, …)`
/// - `graph_position` from the shaper's `graph_position` + the
///   descriptor's index in the `children` vector
/// - `mote_id` from [`crate::derive_mote_id`] over the above
///
/// # Examples
///
/// ```
/// use kx_mote::{ChildDescriptor, ConfigVal, EffectPattern, LogicRef, NdClass, RoleId};
///
/// let c = ChildDescriptor {
///     role_id: RoleId("critic".into()),
///     logic_ref: LogicRef([0u8; 32]),
///     nd_class: NdClass::Pure,
///     effect_pattern: EffectPattern::IdempotentByConstruction,
///     intent: ConfigVal(Vec::new()),
/// };
/// assert_eq!(&c.role_id.0, "critic");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ChildDescriptor {
    /// The Role the child takes — the registry maps `role_id` to a
    /// `kx_warrant::Role` at materialization. The child's warrant is
    /// `intersect(parent.warrant, role.warrant)` per D30.
    pub role_id: RoleId,
    /// `MoteDef.logic_ref` for the child — what the child executes.
    pub logic_ref: LogicRef,
    /// `MoteDef.nd_class` for the child. Drives recovery semantics +
    /// scheduling priority.
    pub nd_class: NdClass,
    /// `MoteDef.effect_pattern` for the child. Determines whether the
    /// child's `ready_set` gates downstream consumers on a critic verdict
    /// (3c only).
    pub effect_pattern: EffectPattern,
    /// **Per-child instruction** the topology shaper emits for THIS child —
    /// carried verbatim into the materialized child's `config_subset` under
    /// [`crate::PROMPT_KEY`] by the child resolver, so a corrective child in
    /// a re-plan round runs ITS OWN task instruction rather than re-running
    /// the shaper's planning prompt. **Untrusted model content** — already
    /// size-capped + strictly parsed at the planner decode boundary
    /// (`decode_loop_proposal` / `decode_replan_proposal`) before it ever
    /// becomes a descriptor; the resolver only ever writes it to the prompt
    /// key, never to an authority axis (SN-8 narrowing-only is unaffected).
    ///
    /// **Empty `intent` preserves the pre-intent behavior**: the resolver
    /// then inherits the shaper's `config_subset` (incl. its prompt) verbatim,
    /// so an empty-intent child's materialized `MoteDef` is byte-identical to
    /// what it was before this field existed.
    ///
    /// **Identity-bearing**: a non-empty `intent` lands in the child's
    /// `config_subset`, which folds into `MoteDef::hash` → the child `MoteId`,
    /// so two children differing only by `intent` are genuinely distinct work.
    /// Because `intent` is serialized into the committed `TopologyDecision`,
    /// the materializer re-derives identical child identities on cold-refold
    /// (R49) — it is NOT `#[serde(skip)]` (which would make replay diverge).
    pub intent: ConfigVal,
}

/// **D37 Seam A enforcement primitive — the closed topology payload.**
///
/// The payload a topology-shaper Mote commits as its `result_ref`. The
/// shaper does NOT spawn imperatively; it commits this declarative
/// payload and the runtime materializes children deterministically.
///
/// **Single-source-of-truth principle (D37)**: child identity derives
/// from the shaper's committed `result_ref` (i.e., from
/// `ContentRef::of(canonical_bincode(self))`). `TopologyDecision` does
/// NOT carry `shaper_mote_id` for this reason — the shaper is
/// implicitly the Mote whose `result_ref` points to THIS payload.
///
/// **Refusal predicate R-8b** (per `validate-then-commit.md` §7,
/// PR 4.5): the executor refuses any imperative-spawn attempt at
/// submission time. Shapers MUST commit a `TopologyDecision`.
///
/// **Materialization** (P1.11): the projection reads
/// `shaper.result_ref`, deserializes the `TopologyDecision`, and
/// materializes one child `Mote` per [`crate::ChildDescriptor`] in `children`.
/// Materialization is deterministic — replay produces bit-identical
/// child `MoteId`s without coordination.
///
/// # Examples
///
/// ```
/// use kx_mote::{
///     ChildDescriptor, ConfigVal, EffectPattern, LogicRef, NdClass, RoleId,
///     TopologyDecision,
/// };
///
/// let td = TopologyDecision {
///     children: vec![
///         ChildDescriptor {
///             role_id: RoleId("critic".into()),
///             logic_ref: LogicRef([0u8; 32]),
///             nd_class: NdClass::Pure,
///             effect_pattern: EffectPattern::IdempotentByConstruction,
///             intent: ConfigVal(Vec::new()),
///         },
///         ChildDescriptor {
///             role_id: RoleId("worker".into()),
///             logic_ref: LogicRef([1u8; 32]),
///             nd_class: NdClass::WorldMutating,
///             effect_pattern: EffectPattern::StageThenCommit,
///             intent: ConfigVal(b"summarize the inputs".to_vec()),
///         },
///     ],
/// };
///
/// // Content-addressable: identical TopologyDecision → identical hash.
/// let h1 = td.hash();
/// let h2 = td.hash();
/// assert_eq!(h1, h2);
/// assert_eq!(h1.len(), 32);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TopologyDecision {
    /// The child Motes this shaper declares, in workflow-author intent
    /// order. Replay materializes children in this exact order; the
    /// descriptor index becomes the suffix of the child's
    /// `graph_position`, so order is identity-bearing.
    pub children: Vec<ChildDescriptor>,
}

impl TopologyDecision {
    /// Content-address of this `TopologyDecision`.
    ///
    /// `blake3(canonical_bincode(self))` using the workspace-canonical
    /// configuration via [`crate::canonical_config`]. **Deterministic + pure** —
    /// two calls on the same value produce identical bytes; two callers
    /// constructing the same `TopologyDecision` on different machines
    /// compute identical hashes.
    ///
    /// The shaper's `Committed` entry has `result_ref = ContentRef::of(
    /// canonical_bincode(td))` — i.e., the `result_ref` field on the
    /// shaper's journal entry equals `td.hash()`. The projection's
    /// materializer reads this `result_ref`, fetches the payload bytes
    /// from the content store, deserializes back to `TopologyDecision`,
    /// and materializes children.
    ///
    /// # Examples
    ///
    /// ```
    /// use kx_mote::TopologyDecision;
    ///
    /// let td = TopologyDecision { children: vec![] };
    /// let h = td.hash();
    /// assert_eq!(h.len(), 32);
    /// // Empty TopologyDecision has a stable, deterministic hash.
    /// assert_eq!(td.hash(), td.hash());
    /// ```
    #[must_use]
    pub fn hash(&self) -> [u8; 32] {
        let bytes = bincode::serde::encode_to_vec(self, canonical_config())
            .expect("TopologyDecision canonical bincode encodes infallibly");
        *blake3::hash(&bytes).as_bytes()
    }
}
