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

/// The strict outer envelope for a model-proposed agentic-loop round:
/// `{ "loop_proposal": { … } }`. `deny_unknown_fields` rejects any sibling key,
/// so a payload that merely *contains* a proposal (or smuggles a sibling field)
/// is refused — the same fail-closed boundary as [`Envelope`].
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LoopEnvelope {
    pub loop_proposal: LoopProposalWire,
}

/// The wire form of a [`crate::LoopProposal`]: a versioned, strict list of the
/// next round's steps. Decoded into the public `LoopProposal` (which carries no
/// `version` — that is an envelope concern, validated then dropped) only after
/// the envelope invariants pass. Reuses [`PlanStep`] verbatim — the same minimal
/// trust surface (role name + intent, `deny_unknown_fields`, no score channel →
/// D77 holds for free).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LoopProposalWire {
    /// Envelope schema version. Only `1` is accepted; any other value is refused
    /// as [`crate::PlanError::UnknownVersion`] (forward-compatible fail-closed,
    /// mirroring [`Plan::version`]).
    pub version: u32,
    /// The next round's steps, in author/intent order. Each lowers to a
    /// [`kx_mote::ChildDescriptor`] via [`crate::lower_loop_to_topology_decision`].
    pub next_steps: Vec<PlanStep>,
}

/// The strict outer envelope for a model-proposed **re-plan round** (PR-3 / AL2):
/// `{ "replan": { … } }`. Distinct from [`LoopEnvelope`] so [`crate::decode_loop_proposal`]
/// (the PR-2 initial-round boundary) stays byte-frozen — this is the SEPARATE
/// trust boundary a re-plan-on-failure round decodes through. `deny_unknown_fields`
/// rejects any sibling key (the same fail-closed discipline).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReplanEnvelope {
    pub replan: ReplanWire,
}

/// The wire form of a re-plan round's 3-way decision. Exactly ONE of `next_steps`
/// (a non-empty corrective fan-out — corrected-context / permission-adapt) or
/// `flag_human` (escalate) is present; both-or-neither is refused. `next_steps`
/// reuses [`PlanStep`] verbatim, so a corrective round lowers through the SAME
/// vetted-recipe path as an initial round (SN-8). `flag_human` carries an opaque,
/// bounded human-readable reason (never parsed for enforcement; surfaced to the
/// operator). No score channel anywhere → D77 holds for free.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReplanWire {
    /// Envelope schema version. Only `1` is accepted; any other value is refused
    /// as [`crate::PlanError::UnknownVersion`] (forward-compatible fail-closed).
    pub version: u32,
    /// The corrective round's steps (empty ⇒ the model is escalating, not
    /// re-planning). `#[serde(default)]` + `deny_unknown_fields` keeps the
    /// either/or shape strict.
    #[serde(default)]
    pub next_steps: Vec<PlanStep>,
    /// An escalation reason — present iff the model chose flag-a-human (the third
    /// route). Bounded by `MAX_FLAG_HUMAN_BYTES` in the decoder.
    #[serde(default)]
    pub flag_human: Option<String>,
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
