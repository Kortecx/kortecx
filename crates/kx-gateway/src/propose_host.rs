//! `HostWorkflowProposer` — the served-model side of the `ProposeWorkflow` seam.
//!
//! Runs the served model ONCE (the [`crate::prompt_library::PLANNER_SYSTEM`] contract plus
//! the goal), then decodes and compiles the result through the vetted `kx-planner` path
//! (`decode_plan` then `compile_plan`). It VALIDATES ONLY: it registers nothing, submits
//! nothing, and writes no journal, so it is unaffected by the run-registration dedup and is
//! digest-invariant. A model, decode, or compile failure is an honest
//! [`WorkflowProposal::Rejected`], never a panic (D142). SN-8: the model names only a role,
//! an intent, and edges; every capability axis comes from the vetted role catalog
//! ([`build_authoring_role_catalog`]).

use std::collections::BTreeMap;
use std::sync::Arc;

use kx_gateway_core::{ProposedStep, WorkflowProposal, WorkflowProposer};
use kx_inference::InferenceBackend;
use kx_mote::{ModelId, RoleId};
use kx_planner::{
    compile_plan, decode_plan, max_plan_bytes, seed_from_plan_bytes, PlanStepKind,
    RoleRecipeResolver,
};
use kx_warrant::{ExecutorClass, RoleRegistry};

use crate::model_exec::{build_authoring_role_catalog, shaper_warrant};
use crate::prompt_library::{planner_user_message, PLANNER_SYSTEM};
use crate::routing_backend::RoutingBackend;

/// The host proposer: the served-model backend + the vetted role catalog it compiles a
/// proposal against. Built once per serve (the catalog is fixed for the served model).
pub(crate) struct HostWorkflowProposer {
    backend: Arc<RoutingBackend>,
    model_id: ModelId,
    exec_class: ExecutorClass,
    role_registry: Arc<dyn RoleRegistry>,
    recipes: Arc<dyn RoleRecipeResolver>,
}

impl HostWorkflowProposer {
    /// Wire the proposer for a served model. The role catalog is the curated authoring
    /// palette resolved against `model_id` (pure model roles; SN-8 axes come from the
    /// vetted recipes).
    pub(crate) fn new(
        backend: Arc<RoutingBackend>,
        model_id: ModelId,
        exec_class: ExecutorClass,
    ) -> Self {
        let (role_registry, recipes) = build_authoring_role_catalog(&model_id, exec_class);
        Self {
            backend,
            model_id,
            exec_class,
            role_registry,
            recipes,
        }
    }
}

#[tonic::async_trait]
impl WorkflowProposer for HostWorkflowProposer {
    async fn propose(&self, goal: &str) -> WorkflowProposal {
        // Model inference is BLOCKING — run the whole render→decode→compile off the async
        // worker (the backend + catalog are cheap Arc clones; the join failure is honest).
        let backend = self.backend.clone();
        let model_id = self.model_id.clone();
        let exec_class = self.exec_class;
        let role_registry = self.role_registry.clone();
        let recipes = self.recipes.clone();
        let goal = goal.to_string();
        match tokio::task::spawn_blocking(move || {
            propose_blocking(
                backend.as_ref(),
                &model_id,
                exec_class,
                role_registry.as_ref(),
                recipes.as_ref(),
                &goal,
            )
        })
        .await
        {
            Ok(outcome) => outcome,
            Err(e) => rejected(&format!("the planner task failed: {e}")),
        }
    }
}

/// The synchronous render→decode→compile→map core (generic over the backend so a stub can
/// drive it in a unit test). Validate-only; never mutates state.
fn propose_blocking<B: InferenceBackend>(
    backend: &B,
    model_id: &ModelId,
    exec_class: ExecutorClass,
    role_registry: &dyn RoleRegistry,
    recipes: &dyn RoleRecipeResolver,
    goal: &str,
) -> WorkflowProposal {
    // (1) Run the served model once with the planner contract + the goal + role palette.
    let user = planner_user_message(goal);
    let Some(raw) = backend.render_chat(model_id, PLANNER_SYSTEM, &user) else {
        return rejected(
            "no served model can render a plan (start `kx serve` with an inference or \
             serve-engine build and a resolved model)",
        );
    };

    // (2) Decode the strict `{"plan":…}` envelope (fail-closed; strips fences/reasoning/
    //     trailing prose internally) then (3) compile it — the structural gate that resolves
    //     every role against the vetted catalog and intersects each warrant (narrowing-only).
    let parent = shaper_warrant(model_id, exec_class);
    let cap = max_plan_bytes(&parent);
    let plan = match decode_plan(raw.as_bytes(), cap) {
        Ok(p) => p,
        Err(e) => return rejected(&format!("the model did not return a valid plan: {e}")),
    };
    let seed = seed_from_plan_bytes(raw.as_bytes());
    if let Err(e) = compile_plan(&plan, seed, &parent, role_registry, recipes) {
        return rejected(&format!("the proposed plan is not admissible: {e}"));
    }

    // (4) Map the validated plan → the DISPLAY proposal (role/intent from the plan;
    //     model_id/tool_contract resolved from the vetted recipe — never trusted back).
    let steps = plan
        .steps
        .iter()
        .map(|s| {
            let recipe = recipes.recipe(&RoleId(s.role.clone()));
            ProposedStep {
                role: s.role.clone(),
                intent: s.intent.clone(),
                kind: kind_str(s.kind).to_string(),
                model_id: recipe.as_ref().map(|r| r.model_id.0.clone()).unwrap_or_default(),
                tool_contract: recipe
                    .map(|r| {
                        r.tool_contract
                            .iter()
                            .map(|(k, v)| (k.0.clone(), v.0.clone()))
                            .collect::<BTreeMap<String, String>>()
                    })
                    .unwrap_or_default(),
            }
        })
        .collect();
    let edges = plan
        .edges
        .iter()
        .filter_map(|e| Some((u32::try_from(e.parent).ok()?, u32::try_from(e.child).ok()?)))
        .collect();
    WorkflowProposal::Proposed { steps, edges }
}

/// The wire kind string for a proposed step (the console maps it back to a builder kind).
fn kind_str(kind: PlanStepKind) -> &'static str {
    match kind {
        PlanStepKind::Plain => "plain",
        PlanStepKind::Critic => "critic",
        PlanStepKind::DeterministicCritic => "deterministic_critic",
        PlanStepKind::TopologyShaper => "topology_shaper",
    }
}

fn rejected(reason: &str) -> WorkflowProposal {
    WorkflowProposal::Rejected {
        reason: reason.to_string(),
    }
}
