//! The Morphic workflow-as-data types ([`WorkflowDef`], [`StepDef`],
//! [`StepRole`], [`StepRef`], [`StepEdge`]) and the compile output
//! ([`CompiledMote`], [`CompiledWorkflow`]).
//!
//! A workflow is authored as a graph of *steps* (agents) joined by typed
//! *edges* — NOT as pre-derived Motes. [`crate::compile`] turns that graph into
//! a Mote DAG. Authoring-as-data is what makes compilation a meaningful, pure
//! function: identity is derived, never hand-assigned.

use std::collections::BTreeMap;

use kx_mote::{
    ConfigKey, ConfigVal, EdgeMeta, EffectPattern, InferenceParams, LogicRef, ModelId, Mote,
    MoteGraph, NdClass, PromptTemplateHash, ToolName, ToolVersion,
};
use kx_warrant::WarrantSpec;

/// An opaque handle to a step within one [`WorkflowDef`].
///
/// Minted only by [`WorkflowDef::add_step`]; the inner index is private so a
/// `StepRef` cannot be forged or dangle within the workflow that created it.
/// (Mixing a `StepRef` across two different `WorkflowDef`s is the one misuse the
/// type cannot prevent; [`crate::compile`] still range-checks every index.)
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StepRef(pub(crate) usize);

impl StepRef {
    /// The underlying step index (its insertion order in the [`WorkflowDef`]).
    #[inline]
    #[must_use]
    pub const fn index(self) -> usize {
        self.0
    }
}

/// The role a step plays in the workflow — the axis that drives `critic_for`
/// and `is_topology_shaper` on the compiled [`kx_mote::MoteDef`].
///
/// Modelled as an enum so the critic/shaper mutual exclusion (executor refusal
/// R-8: a Mote may not be both a critic and a topology shaper) is
/// *unrepresentable* — a step is exactly one of these.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StepRole {
    /// An ordinary producer step. `critic_for = None`, `is_topology_shaper = false`.
    Plain,
    /// A critic for `producer` (the `ValidateThenCommit` 3c pattern, D20). The
    /// compiled `MoteDef.critic_for` is set to the producer's derived `MoteId`.
    /// The producer MUST precede this step in the DAG (declare a dependency
    /// edge from producer to critic), or compilation fails with
    /// [`crate::CompileError::InvalidCritic`].
    Critic {
        /// The step this critic validates.
        producer: StepRef,
    },
    /// A topology shaper (`is_topology_shaper = true`). Its children are NOT
    /// static compile output — at runtime the shaper commits a
    /// [`kx_mote::TopologyDecision`] and the projection materializes children
    /// deterministically (D23/D37). Here the step is only marked as a shaper;
    /// the dynamic unroll is exercised against the runtime materializer.
    TopologyShaper,
}

/// One authored step (agent): everything needed to build its
/// [`kx_mote::MoteDef`] EXCEPT the per-instance position (`graph_position`) and
/// committed-input identity (`input_data_id`), which [`crate::compile`] derives
/// from the DAG. Carries the `warrant` + `capability` through to submission.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StepDef {
    /// Hash of the compiled artifact backing this step's logic.
    pub logic_ref: LogicRef,
    /// Pinned model identity (inclusive of version + quantization).
    pub model_id: ModelId,
    /// Hash of the system/prompt template the step uses.
    pub prompt_template_hash: PromptTemplateHash,
    /// The closed set of tools the step may call, each at its pinned version.
    pub tool_contract: BTreeMap<ToolName, ToolVersion>,
    /// The non-determinism class — recovery semantics + tiering derive from it.
    pub nd_class: NdClass,
    /// Behavior-affecting configuration allowlist (operational keys excluded).
    pub config_subset: BTreeMap<ConfigKey, ConfigVal>,
    /// Which effect/commit pattern this step uses.
    pub effect_pattern: EffectPattern,
    /// Decoding parameters (participate in identity, D50). Seed lives here.
    pub inference_params: InferenceParams,
    /// The step's role (plain / critic / topology shaper).
    pub role: StepRole,
    /// The warrant the step runs under, carried verbatim to submission.
    pub warrant: WarrantSpec,
    /// The capability a WORLD-MUTATING / READ-ONLY-NONDET dispatch routes
    /// through (PURE steps ignore it), carried verbatim to submission.
    pub capability: ToolName,
}

/// A declared directed dependency edge (`parent` → `child`) carrying
/// [`kx_mote::EdgeMeta`] (Data / Control, cascade semantics).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StepEdge {
    /// The upstream step.
    pub parent: StepRef,
    /// The downstream step that depends on `parent`.
    pub child: StepRef,
    /// The kind of dependency (Data / Control) + cascade metadata.
    pub edge: EdgeMeta,
}

/// A Morphic workflow authored as plain, deterministically-ordered data.
///
/// Step insertion order and the `seed` are identity-bearing (they feed
/// `graph_position` and entrypoint `input_data_id`). Build one with
/// [`WorkflowDef::new`], [`WorkflowDef::add_step`], and [`WorkflowDef::add_edge`],
/// then [`crate::compile`] it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkflowDef {
    pub(crate) steps: Vec<StepDef>,
    pub(crate) edges: Vec<StepEdge>,
    pub(crate) seed: u32,
}

impl WorkflowDef {
    /// Create an empty workflow with the given workflow-input `seed`. The seed
    /// is folded into every entrypoint step's `input_data_id`, so two workflows
    /// identical except for their seed produce different `MoteId`s — the basis
    /// of reproducible-by-reference synthesis (D50).
    #[inline]
    #[must_use]
    pub const fn new(seed: u32) -> Self {
        Self {
            steps: Vec::new(),
            edges: Vec::new(),
            seed,
        }
    }

    /// Append a step, returning its [`StepRef`] handle.
    pub fn add_step(&mut self, step: StepDef) -> StepRef {
        let idx = self.steps.len();
        self.steps.push(step);
        StepRef(idx)
    }

    /// Declare a `parent` → `child` dependency edge.
    ///
    /// # Errors
    ///
    /// Returns [`crate::CompileError::StepIndexOutOfRange`] if either ref is not
    /// a step in this workflow, or [`crate::CompileError::DuplicateEdge`] if the
    /// same `(parent, child)` pair was already declared.
    pub fn add_edge(
        &mut self,
        parent: StepRef,
        child: StepRef,
        edge: EdgeMeta,
    ) -> Result<(), crate::CompileError> {
        let n = self.steps.len();
        if parent.0 >= n {
            return Err(crate::CompileError::StepIndexOutOfRange(parent.0));
        }
        if child.0 >= n {
            return Err(crate::CompileError::StepIndexOutOfRange(child.0));
        }
        if self
            .edges
            .iter()
            .any(|e| e.parent == parent && e.child == child)
        {
            return Err(crate::CompileError::DuplicateEdge {
                parent: parent.0,
                child: child.0,
            });
        }
        self.edges.push(StepEdge {
            parent,
            child,
            edge,
        });
        Ok(())
    }

    /// The number of steps declared so far.
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.steps.len()
    }

    /// `true` if no steps have been declared.
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    /// The workflow-input seed.
    #[inline]
    #[must_use]
    pub const fn seed(&self) -> u32 {
        self.seed
    }
}

/// One compiled step ready to submit: a derived [`Mote`] plus the warrant and
/// capability carried from its [`StepDef`].
///
/// Structurally mirrors `kx_runtime::WorkflowMote`, so a [`CompiledWorkflow`]'s
/// motes drop directly into the scheduler's submission surface.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompiledMote {
    /// The derived Mote (identity already computed by [`Mote::new`]).
    pub mote: Mote,
    /// The warrant the Mote runs under.
    pub warrant: WarrantSpec,
    /// The capability a WM/ROND dispatch routes through.
    pub capability: ToolName,
}

/// The result of [`crate::compile`]: the compiled motes in topological
/// (submission) order, plus a [`MoteGraph`] view for DAG queries and tests.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompiledWorkflow {
    /// Compiled motes in topological order — also the order to submit them.
    pub motes: Vec<CompiledMote>,
    /// The same motes as a `kx-mote` graph container (nodes + parent edges).
    pub graph: MoteGraph,
}
