// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! `kx-planner` (M6) — turn an untrusted model **proposal** into a **registered
//! Mote DAG**. The front of the agentic pipe: prompt → plan → tools → result.
//!
//! # The contract (D74)
//!
//! A plan is a **PROPOSAL captured as a fact**, never an authorization. The
//! model's plan-generation is itself a Mote (ROND if it samples, PURE if a
//! deterministic template); its committed `result_ref` IS the structured plan —
//! a content-addressed fact re-read on replay, **never re-run**. The runtime
//! compiles that committed plan and registers the resulting Motes; identity is
//! `kx_mote::Mote::new`-derived. **Reproducibility comes from committing the
//! plan, not from model determinism** — a planner step re-samples on re-run by
//! design.
//!
//! # What the model may name (IMP-5 / D70 / D75 — minimal trust surface)
//!
//! The model output is the new untrusted input. The strict plan envelope carries
//! **only** what a model may legitimately choose — role *names*, per-step *intent*
//! strings, and *edges* — and **nothing** that participates in Mote identity or
//! capability. [`decode_plan`] is fail-closed (total, panic-free, size-capped,
//! strict envelope). The heavy `MoteDef` axes (`logic_ref`, `model_id`,
//! `tool_contract`, `nd_class`, `effect_pattern`, …) come from a **vetted**
//! [`RoleRecipe`] keyed by exact [`kx_mote::RoleId`] equality — never from model
//! output, never by fuzzy match.
//!
//! # The two lowering targets
//!
//! - [`lower_plan`] / [`compile_plan`] — a static plan → [`kx_workflow::WorkflowDef`]
//!   → [`kx_workflow::compile`] (the **structural gate**: acyclic /
//!   critic-precedes-producer / deterministic order). Every produced step's
//!   warrant is `kx_warrant::intersect(parent, role)` — narrowing-only, so the
//!   planner can **never escalate privilege** (D75).
//! - [`lower_loop_to_topology_decision`] — an agentic loop is **not** a DAG cycle;
//!   it lowers to a [`kx_mote::TopologyDecision`] a ROND shaper commits (D76). The
//!   projection materializes children deterministically (reusing the shipped
//!   `DefaultTopologyMaterializer`); a re-plan **appends** a fresh round, never
//!   mutating a committed Mote.
//!
//! # Thesis test
//!
//! This crate sits ABOVE the scheduler and depends ONLY on `kx-workflow` +
//! `kx-mote` + `kx-warrant` + `kx-critic-types` — never `kx-scheduler` /
//! `kx-projection` / `kx-executor` / `kx-inference`. It is a **pure lowering
//! library**: no I/O, no model loop. The model-runs-once + context-assembly
//! wiring lives in `kx-model-harness`.

#![forbid(unsafe_code)]
// Pedantic lints kept as warnings workspace-wide; these few are noise for this
// crate's shape (mirrors the allow-set used by `kx-workflow` / `kx-warrant`).
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::must_use_candidate
)]
// Inline test modules are exempt from the workspace deny on unwrap/expect.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod decode;
mod error;
mod lower;
mod plan;
mod role;

pub use decode::{
    decode_loop_proposal, decode_plan, decode_replan_proposal, max_plan_bytes,
    MAX_FLAG_HUMAN_BYTES, MAX_LOOP_STEPS, MAX_PLAN_EDGES, MAX_PLAN_STEPS,
};
pub use error::PlanError;
pub use lower::{
    compile_plan, lower_loop_to_topology_decision, lower_plan, seed_from_plan_bytes, LoopProposal,
    ReplanProposal, PLAN_PROMPT_KEY,
};
pub use plan::{Plan, PlanEdge, PlanStep, PlanStepKind};
pub use role::{InMemoryRoleRecipes, RoleRecipe, RoleRecipeResolver};

#[cfg(test)]
mod tests;
