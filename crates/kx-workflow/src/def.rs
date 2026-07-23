//! The Morphic workflow-as-data types ([`WorkflowDef`], [`StepDef`],
//! [`StepRole`], [`StepRef`], [`StepEdge`]) and the compile output
//! ([`CompiledMote`], [`CompiledWorkflow`]).
//!
//! A workflow is authored as a graph of *steps* (agents) joined by typed
//! *edges* — NOT as pre-derived Motes. [`crate::compile`] turns that graph into
//! a Mote DAG. Authoring-as-data is what makes compilation a meaningful, pure
//! function: identity is derived, never hand-assigned.

use std::collections::BTreeMap;

use kx_critic_types::CheckSpec;
use kx_mote::{
    ConfigKey, ConfigVal, EdgeMeta, EffectPattern, InferenceParams, LogicRef, ModelId, Mote,
    MoteGraph, NdClass, PromptTemplateHash, ToolName, ToolVersion,
};
use kx_warrant::WarrantSpec;
use serde::{Deserialize, Serialize};

/// An opaque handle to a step within one [`WorkflowDef`].
///
/// Minted only by [`WorkflowDef::add_step`]; the inner index is private so a
/// `StepRef` cannot be forged or dangle within the workflow that created it.
/// (Mixing a `StepRef` across two different `WorkflowDef`s is the one misuse the
/// type cannot prevent; [`crate::compile`] still range-checks every index.)
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
    /// A **deterministic critic** for `producer` (D60 / P4.2-2). Compiles to a
    /// PURE `MoteDef` with `critic_for = producer's MoteId` AND
    /// `critic_check = Some(check)` — the declared check is folded into the
    /// critic's identity. At runtime the executor evaluates `check` in-process
    /// against the producer's committed bytes (`run_native_critic_mote`) and
    /// commits a `CriticVerdict`; the projection's promotion gate reads it.
    /// The producer MUST precede this step in the DAG (declare a dependency
    /// edge), or compilation fails with [`crate::CompileError::InvalidCritic`].
    /// Unlike [`StepRole::Critic`] (a model-validated `ValidateThenCommit`
    /// critic), this carries no model and is decorrelated from the producer.
    DeterministicCritic {
        /// The step this critic validates.
        producer: StepRef,
        /// The declarative check evaluated in-process against the producer's
        /// committed output bytes.
        check: CheckSpec,
    },
}

/// One authored step (agent): everything needed to build its
/// [`kx_mote::MoteDef`] EXCEPT the per-instance position (`graph_position`) and
/// committed-input identity (`input_data_id`), which [`crate::compile`] derives
/// from the DAG. Carries the `warrant` + `capability` through to submission.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

    /// Bind a free-param slot to a concrete value across every step that
    /// declares it, returning the number of steps bound.
    ///
    /// A step *declares* slot `name` by carrying `ConfigKey(name)` in its
    /// `config_subset`; this overwrites that entry with `value`. A return of `0`
    /// means no step declares the slot — the caller (e.g. the D121 inbound
    /// execution path) should fail-closed rather than silently drop the value.
    ///
    /// This is the binding primitive for parametrized recipes: inject validated
    /// argument bytes here, *before* [`compile`](crate::compile). Because
    /// `config_subset` flows verbatim into each `MoteDef`, distinct bound values
    /// yield distinct `MoteId`s — exactly-once-per-distinct-input by construction
    /// (and identical inputs re-derive identical identity → idempotent re-invoke).
    pub fn bind_param(&mut self, name: &str, value: &ConfigVal) -> usize {
        let key = ConfigKey(name.to_string());
        let mut bound = 0usize;
        for step in &mut self.steps {
            if let Some(slot) = step.config_subset.get_mut(&key) {
                *slot = value.clone();
                bound += 1;
            }
        }
        bound
    }

    /// PR-7: INSERT a config entry into every ENTRY step — a step with no incoming
    /// edge (a DAG root). Unlike [`Self::bind_param`] (which overwrites a declared
    /// free-param slot), this ADDS `key` to each root step's `config_subset`, making
    /// the entry `MoteId` identity-bearing over the value (e.g. attached
    /// context-bundle items, `CONTEXT_ITEMS_KEY`). Returns the number of steps
    /// injected. Because `config_subset` flows verbatim into each `MoteDef`, a
    /// distinct value yields a distinct entry `MoteId` — and an EMPTY call (no
    /// injection) leaves identity byte-identical (the canonical-digest-preserving
    /// property: a run that attaches no context never reaches this method).
    pub fn inject_entry_config(&mut self, key: &str, value: &ConfigVal) -> usize {
        let has_parent: std::collections::HashSet<usize> =
            self.edges.iter().map(|e| e.child.0).collect();
        let k = ConfigKey(key.to_string());
        let mut injected = 0usize;
        for (idx, step) in self.steps.iter_mut().enumerate() {
            if !has_parent.contains(&idx) {
                step.config_subset.insert(k.clone(), value.clone());
                injected += 1;
            }
        }
        injected
    }

    /// INSERT a config entry into ONE step's identity-bearing `config_subset` — the
    /// single-step sibling of [`Self::inject_entry_config`].
    ///
    /// `inject_entry_config` targets every DAG ROOT, which is right for something attached
    /// to the RUN (a context bundle the whole workflow is grounded in). It is wrong for
    /// something attached to a NODE: an App that binds one skill's instructions to its
    /// second gatherer means that step and no other, and on a fan-out "every root" is
    /// several steps. Returns `false` when `step_index` is out of range (a caller-side
    /// indexing error, never a silent no-op on the wrong step).
    ///
    /// Like its sibling this ADDS to `config_subset`, so the step's `MoteId` becomes
    /// identity-bearing over the value — and a workflow that injects nothing never reaches
    /// this method, which is what keeps the canonical digest untouched.
    pub fn inject_step_config(&mut self, step_index: usize, key: &str, value: &ConfigVal) -> bool {
        match self.steps.get_mut(step_index) {
            Some(step) => {
                step.config_subset
                    .insert(ConfigKey(key.to_string()), value.clone());
                true
            }
            None => false,
        }
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
    /// The AUTHORED step this mote came from — its [`StepRef`] index in the
    /// [`WorkflowDef`], i.e. its position in the blueprint the caller wrote.
    ///
    /// [`crate::compile`] emits motes in TOPOLOGICAL order, which is deliberately not
    /// authoring order, so a caller holding a per-step decision (an App binding a
    /// connection's secret scope to one node) had no way to say WHICH mote it meant.
    /// Recomputing the topological order caller-side would be a second definition of an
    /// ordering rule that already lives here — the kind of duplicate that drifts. This
    /// carries the answer instead. Off-identity: it is not part of the `Mote`.
    pub step_index: usize,
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
