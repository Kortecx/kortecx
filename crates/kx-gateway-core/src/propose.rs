//! The NL→DAG workflow-proposer seam (D209.3 / SN-8 — propose-then-confirm).
//!
//! `ProposeWorkflow` turns a natural-language goal into a PROPOSED multi-step DAG by
//! running the served model ONCE and compiling the result through the vetted
//! `kx-planner` path: the model names ONLY role + intent + edges (the minimal trust
//! surface); every capability axis (model_id, tool_contract, nd_class, …) comes from a
//! server-vetted `RoleRecipe` keyed by exact role name, and `intersect(parent, role)`
//! narrows-only (the planner can never escalate privilege). It VALIDATES ONLY
//! (`compile_plan`) — it registers nothing and writes no journal, so it is unaffected by
//! the run-registration dedup and is digest-invariant.
//!
//! Like the other model-served seams (`AppScaffolder`, `AppAuthor`), the host owns the
//! runtime surface: the concrete impl (in `kx-gateway`, behind `serve-engine`) holds the
//! `RoutingBackend` + the vetted role catalog and runs the model. gateway-core defines
//! only the seam + the display-shaped outcome. A `None` seam ⇒ `ProposeWorkflow` returns
//! `unimplemented` (no served model on this gateway).

use std::collections::BTreeMap;

/// One proposed step, in DISPLAY shape. `role`/`intent`/`kind` are the model's plan
/// (the minimal trust surface); `model_id` + `tool_contract` are the SERVER-resolved
/// recipe axes, returned so the client can render the granted capabilities — the
/// authoritative axes are re-derived server-side when the confirmed DAG is authored/run
/// (SN-8), never trusted from this response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposedStep {
    /// The vetted role name this step plays.
    pub role: String,
    /// The model's free-form per-step instruction.
    pub intent: String,
    /// The structural kind: `plain` | `critic` | `deterministic_critic` | `topology_shaper`.
    pub kind: String,
    /// The model id resolved from the role recipe (display only).
    pub model_id: String,
    /// The resolved grant set `{tool_id: version}` (display only).
    pub tool_contract: BTreeMap<String, String>,
}

/// The outcome of an NL workflow proposal. Never a transport error: a failure (no served
/// model, an unknown/ungrantable role or tool, a structurally invalid plan) is a
/// `Rejected { reason }` surfaced to the author (don't-fake-gaps, D142).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkflowProposal {
    /// A compiled-and-admissible proposal, ready for the author to preview + confirm.
    Proposed {
        /// The proposed steps, in plan order.
        steps: Vec<ProposedStep>,
        /// The proposed dependency edges as `(parent_index, child_index)`.
        edges: Vec<(u32, u32)>,
    },
    /// The proposal was refused; `reason` is human-readable (never parsed for enforcement).
    Rejected {
        /// The advisory reason (surfaced to the author).
        reason: String,
    },
}

/// The host-side NL→DAG proposer seam. The host impl owns the served-model backend + the
/// vetted role catalog; `propose` runs the model once and compiles the plan (validate-only).
/// A `None` seam ⇒ `ProposeWorkflow` returns `unimplemented`.
///
/// Async so the host can offload the BLOCKING model inference (e.g. via
/// `tokio::task::spawn_blocking`) — gateway-core stays runtime-light (no direct `tokio`).
#[tonic::async_trait]
pub trait WorkflowProposer: Send + Sync {
    /// Propose a multi-step DAG for `goal`. Never errors at the transport level: a failure
    /// (no served model, an unknown/ungrantable role, a structurally invalid plan) is a
    /// [`WorkflowProposal::Rejected`].
    async fn propose(&self, goal: &str) -> WorkflowProposal;
}
