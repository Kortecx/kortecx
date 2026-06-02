//! Lowering — turn a decoded [`Plan`] (or a loop [`LoopProposal`]) into the
//! runtime's registered-DAG shapes. This is where D74/D75/D76 become code:
//!
//! - [`lower_plan`] / [`compile_plan`]: a static plan → [`WorkflowDef`] →
//!   [`kx_workflow::compile`] (the structural gate). Each step's warrant is
//!   `kx_warrant::intersect(parent, role)` — the **only** warrant path in the
//!   crate, narrowing-only, so the planner can never escalate privilege (D75).
//!   The heavy `MoteDef` axes come from the vetted [`RoleRecipe`], never the
//!   model (IMP-5 / D70). The planner never hand-derives identity or hand-builds
//!   edges — `compile` derives `MoteId`s and enforces acyclicity /
//!   critic-precedence / deterministic order.
//! - [`lower_loop_to_topology_decision`]: an agentic loop is NOT a DAG back-edge;
//!   it lowers to a [`TopologyDecision`] a ROND shaper commits (D76). The
//!   projection materializes children deterministically (the shipped
//!   `DefaultTopologyMaterializer` narrows each child's warrant via `intersect`).

use std::collections::{BTreeMap, BTreeSet};

use kx_mote::{ChildDescriptor, ConfigKey, ConfigVal, EdgeMeta, RoleId, TopologyDecision};
use kx_warrant::{intersect, RoleRegistry, WarrantSpec};
use kx_workflow::{
    compile, CompileError, CompiledWorkflow, StepDef, StepRef, StepRole, WorkflowDef,
};

use crate::error::PlanError;
use crate::plan::{Plan, PlanStep, PlanStepKind};
use crate::role::{RoleRecipe, RoleRecipeResolver};

/// The `config_subset` key under which a step's `intent` is carried — the same
/// stable convention `kx_model_harness::prompt::PROMPT_KEY` reads, so a model
/// executor uses the intent as the step's instruction. Identity-bearing (two
/// plans with different intents compile to different `MoteId`s). A drift guard
/// in `kx-model-harness` asserts the two constants stay equal.
pub const PLAN_PROMPT_KEY: &str = "prompt";

/// Derive a replay-stable `WorkflowDef` seed from the committed plan bytes.
///
/// `blake3(plan_bytes)[..4]` as a little-endian `u32`. Deterministic — the same
/// committed plan re-compiles to the same DAG on replay (never a clock).
#[must_use]
pub fn seed_from_plan_bytes(plan_bytes: &[u8]) -> u32 {
    let h = blake3::hash(plan_bytes);
    let b = h.as_bytes();
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

/// Resolve a role's warrant template + recipe, returning the intersected child
/// warrant and the recipe. The single warrant path: `intersect(parent, role)`.
fn resolve_step(
    role_id: &RoleId,
    parent_warrant: &WarrantSpec,
    role_registry: &dyn RoleRegistry,
    recipe_resolver: &dyn RoleRecipeResolver,
) -> Result<(WarrantSpec, RoleRecipe), PlanError> {
    let role = role_registry
        .resolve(role_id)
        .ok_or_else(|| PlanError::UnknownRole(role_id.clone()))?;
    let warrant = intersect(parent_warrant, &role).map_err(|source| PlanError::Ungrantable {
        role: role_id.clone(),
        source,
    })?;
    let recipe = recipe_resolver
        .recipe(role_id)
        .ok_or_else(|| PlanError::UnknownRecipe(role_id.clone()))?;

    // IMP-5: refuse a recipe that names a tool the role's (intersected) warrant
    // does not grant — a step that could never legally call it. Exact
    // (name, version) membership (SN-8), never fuzzy.
    for (name, version) in &recipe.tool_contract {
        let granted = warrant
            .tool_grants
            .iter()
            .any(|g| &g.tool_id == name && &g.tool_version == version);
        if !granted {
            return Err(PlanError::UngrantableTool {
                role: role_id.clone(),
                tool: format!("{}@{}", name.0, version.0),
            });
        }
    }
    Ok((warrant, recipe))
}

/// Build a step's `config_subset` carrying the intent under [`PLAN_PROMPT_KEY`].
fn config_with_intent(intent: &str) -> BTreeMap<ConfigKey, ConfigVal> {
    let mut config = BTreeMap::new();
    config.insert(
        ConfigKey(PLAN_PROMPT_KEY.to_string()),
        ConfigVal(intent.as_bytes().to_vec()),
    );
    config
}

/// Map a [`PlanStep`]'s structural kind to a [`StepRole`], validating producer
/// references early (clearer error than `compile`'s `InvalidCritic`, and it
/// guarantees `refs[producer]` exists). `refs` holds the `StepRef`s of the
/// already-pushed steps (so a valid producer satisfies `producer < this index`).
fn step_role_for(
    step_index: usize,
    step: &PlanStep,
    recipe: &RoleRecipe,
    role_id: &RoleId,
    refs: &[StepRef],
) -> Result<StepRole, PlanError> {
    match step.kind {
        PlanStepKind::Plain => Ok(StepRole::Plain),
        PlanStepKind::TopologyShaper => Ok(StepRole::TopologyShaper),
        PlanStepKind::Critic => {
            let producer = valid_producer(step_index, step.producer)?;
            Ok(StepRole::Critic {
                producer: refs[producer],
            })
        }
        PlanStepKind::DeterministicCritic => {
            let producer = valid_producer(step_index, step.producer)?;
            let check = recipe
                .deterministic_check
                .clone()
                .ok_or_else(|| PlanError::MissingCheck(role_id.clone()))?;
            Ok(StepRole::DeterministicCritic {
                producer: refs[producer],
                check,
            })
        }
    }
}

/// A producer index is valid iff it is present and strictly precedes the critic.
fn valid_producer(step_index: usize, producer: Option<usize>) -> Result<usize, PlanError> {
    match producer {
        Some(p) if p < step_index => Ok(p),
        other => Err(PlanError::InvalidProducer {
            step: step_index,
            producer: other,
        }),
    }
}

/// Lower a decoded [`Plan`] into a [`WorkflowDef`], ready to [`compile`].
///
/// `seed` is the workflow-input seed (identity-bearing); derive it
/// deterministically from the committed plan bytes via [`seed_from_plan_bytes`]
/// — never a clock. `compile` is the structural gate; this function leaves
/// acyclicity / critic-precedence to it (surfacing a clearer [`PlanError::InvalidProducer`]
/// for an out-of-order producer up front). A `Critic` / `DeterministicCritic`
/// step's required producer→critic dependency edge is added automatically (if
/// not already declared), so `compile`'s `InvalidCritic` precedence check passes.
pub fn lower_plan(
    plan: &Plan,
    seed: u32,
    parent_warrant: &WarrantSpec,
    role_registry: &dyn RoleRegistry,
    recipe_resolver: &dyn RoleRecipeResolver,
) -> Result<WorkflowDef, PlanError> {
    let mut wf = WorkflowDef::new(seed);
    let mut refs: Vec<StepRef> = Vec::with_capacity(plan.steps.len());
    // Critic dependency edges to auto-add after the author-declared edges
    // (producer_index, critic_index).
    let mut critic_edges: Vec<(usize, usize)> = Vec::new();

    for (i, s) in plan.steps.iter().enumerate() {
        let role_id = RoleId(s.role.clone());
        let (warrant, recipe) =
            resolve_step(&role_id, parent_warrant, role_registry, recipe_resolver)?;
        let role = step_role_for(i, s, &recipe, &role_id, &refs)?;
        if let StepRole::Critic { .. } | StepRole::DeterministicCritic { .. } = role {
            if let Some(p) = s.producer {
                critic_edges.push((p, i));
            }
        }
        let step = StepDef {
            logic_ref: recipe.logic_ref,
            model_id: recipe.model_id.clone(),
            prompt_template_hash: recipe.prompt_template_hash,
            tool_contract: recipe.tool_contract.clone(),
            nd_class: recipe.nd_class,
            config_subset: config_with_intent(&s.intent),
            effect_pattern: recipe.effect_pattern,
            inference_params: recipe.inference_params.clone(),
            role,
            warrant,
            capability: recipe.capability.clone(),
        };
        refs.push(wf.add_step(step));
    }

    // Track declared edges so an auto critic edge never duplicates one.
    let mut declared: BTreeSet<(usize, usize)> = BTreeSet::new();
    for e in &plan.edges {
        let parent = *refs
            .get(e.parent)
            .ok_or(CompileError::StepIndexOutOfRange(e.parent))?;
        let child = *refs
            .get(e.child)
            .ok_or(CompileError::StepIndexOutOfRange(e.child))?;
        wf.add_edge(parent, child, EdgeMeta::data())?;
        declared.insert((e.parent, e.child));
    }
    // Auto-wire each critic's producer→critic Data edge (the critic reads the
    // producer's committed bytes to validate them) unless the author declared it.
    for (producer, critic) in critic_edges {
        if declared.contains(&(producer, critic)) {
            continue;
        }
        wf.add_edge(refs[producer], refs[critic], EdgeMeta::data())?;
    }

    Ok(wf)
}

/// [`lower_plan`] then [`compile`] — the one-call path from a decoded plan to a
/// registered Mote DAG. `compile` is the structural gate (acyclic / critic
/// precedence / deterministic order); its `CompileError` surfaces as
/// [`PlanError::Compile`].
pub fn compile_plan(
    plan: &Plan,
    seed: u32,
    parent_warrant: &WarrantSpec,
    role_registry: &dyn RoleRegistry,
    recipe_resolver: &dyn RoleRecipeResolver,
) -> Result<CompiledWorkflow, PlanError> {
    let wf = lower_plan(plan, seed, parent_warrant, role_registry, recipe_resolver)?;
    Ok(compile(&wf)?)
}

/// The model's proposal for ONE replanning round: the child steps to spawn next.
/// Same minimal trust surface as a [`Plan`] step — role name + intent only; it
/// carries **no score**, so confidence can never reach the promotion gate (D77).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopProposal {
    /// The next round's steps. Each lowers to a [`ChildDescriptor`].
    pub next_steps: Vec<PlanStep>,
}

/// Lower an agentic-loop proposal into a [`TopologyDecision`] a ROND shaper
/// commits (D76). Each next-step becomes a [`ChildDescriptor`] whose
/// `role_id` / `logic_ref` / `nd_class` / `effect_pattern` come from the vetted
/// [`RoleRecipe`] (never model output). The child's warrant is computed
/// downstream by the projection materializer via `intersect(shaper.warrant,
/// role)` — narrowing-only.
///
/// Pure + total + unit-testable without the full e2e (no store / journal /
/// projection). A re-plan APPENDS a fresh `TopologyDecision` (a new committed
/// fact); it never mutates a prior one (D76 / D-LOCK-4).
pub fn lower_loop_to_topology_decision(
    proposal: &LoopProposal,
    recipe_resolver: &dyn RoleRecipeResolver,
) -> Result<TopologyDecision, PlanError> {
    let mut children = Vec::with_capacity(proposal.next_steps.len());
    for s in &proposal.next_steps {
        let role_id = RoleId(s.role.clone());
        let recipe = recipe_resolver
            .recipe(&role_id)
            .ok_or_else(|| PlanError::UnknownRecipe(role_id.clone()))?;
        children.push(ChildDescriptor {
            role_id,
            logic_ref: recipe.logic_ref,
            nd_class: recipe.nd_class,
            effect_pattern: recipe.effect_pattern,
        });
    }
    Ok(TopologyDecision { children })
}
