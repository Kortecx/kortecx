//! The typed plan structures the model proposes — deliberately the **minimal
//! trust surface** (IMP-5 / D70 / D75): a step names a *role* and an *intent*,
//! an edge names two step *indices*. Nothing here participates in Mote identity
//! or capability — the heavy `MoteDef` axes come from a vetted [`crate::RoleRecipe`]
//! (keyed by the role name), never from this model-authored data.
//!
//! Every type derives `serde::Deserialize` with `#[serde(deny_unknown_fields)]`
//! so a trailing or unexpected key fails the decode closed. The structs are flat
//! (strings + `usize` + a small unit enum): there is no unbounded recursive
//! descent, so [`crate::decode_plan`] is total over arbitrary bytes.

use serde::Deserialize;

/// The strict outer envelope: `{ "plan": { … } }`. `deny_unknown_fields` rejects
/// any sibling key, so a payload that merely *contains* a plan is refused.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Envelope {
    pub plan: Plan,
}

/// A model-proposed plan: an ordered list of steps plus the dependency edges
/// between them. Compiled (after role resolution) into a [`kx_workflow::WorkflowDef`].
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    /// Envelope schema version. Only `1` is accepted in M6; any other value is
    /// refused as [`crate::PlanError::UnknownVersion`] (forward-compatible — a
    /// newer planner bumps this and old binaries fail closed rather than
    /// mis-interpret).
    pub version: u32,
    /// The steps, in author/intent order. Step `i` is referenced by index `i`
    /// from [`PlanStep::producer`] and [`PlanEdge`].
    pub steps: Vec<PlanStep>,
    /// The directed `parent → child` dependency edges (Data edges in M6.1).
    #[serde(default)]
    pub edges: Vec<PlanEdge>,
}

/// One proposed step. The model picks a `role` (resolved against the vetted
/// registries — never a raw permission or logic hash) and writes a free-form
/// `intent` (carried as the step's identity-bearing instruction; never parsed
/// for enforcement).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanStep {
    /// The role name. Resolved to a `kx_warrant::Role` (for the warrant, via
    /// `intersect`) AND a [`crate::RoleRecipe`] (for the `MoteDef` axes). An
    /// unregistered name is refused — there is no fuzzy fallback (D70).
    pub role: String,
    /// Free-form human intent. Carried verbatim into the step's `config_subset`
    /// under [`crate::PLAN_PROMPT_KEY`] (identity-bearing); NEVER parsed for
    /// enforcement.
    pub intent: String,
    /// The structural role this step plays. Defaults to [`PlanStepKind::Plain`].
    #[serde(default)]
    pub kind: PlanStepKind,
    /// The producer step index this step validates — REQUIRED iff `kind` is
    /// [`PlanStepKind::Critic`] or [`PlanStepKind::DeterministicCritic`], and
    /// MUST be `< this step's index` (the producer precedes the critic). Absent
    /// otherwise.
    #[serde(default)]
    pub producer: Option<usize>,
}

/// The structural role a step plays — flat (a unit enum + a separate `producer`
/// index field on the step), so the JSON a model writes is plain (`"kind":
/// "critic"`), not nested.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStepKind {
    /// An ordinary producer step.
    #[default]
    Plain,
    /// A model-validated critic for the step named by [`PlanStep::producer`].
    Critic,
    /// A deterministic critic for [`PlanStep::producer`]; its `CheckSpec` comes
    /// from the vetted recipe (the model cannot author a check).
    DeterministicCritic,
    /// An agentic-loop / runtime-fan-out shaper step (lowered to a
    /// [`kx_mote::TopologyDecision`], never a DAG back-edge).
    TopologyShaper,
}

/// A directed dependency edge by step index. `parent` must commit before
/// `child`; a Data edge feeds `parent`'s committed bytes into `child`'s input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanEdge {
    /// The upstream step index.
    pub parent: usize,
    /// The downstream step index that depends on `parent`.
    pub child: usize,
}
