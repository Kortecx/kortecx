// SPDX-License-Identifier: Apache-2.0
//! `kx-workflow` — the kortecx workflow engine, codename **Morphic** (P4.1).
//!
//! Morphic sits *above* the scheduler. It takes a declarative multi-agent
//! workflow — authored as data ([`WorkflowDef`]: steps joined by typed edges) —
//! and [`compile`]s it into a **Mote DAG** the runtime can execute. It only
//! *emits* `kx-mote` + `kx-warrant` types; it never reaches into the scheduler,
//! projection, or executor (the P2 thesis test: distribution is wiring, not a
//! rewrite, and the engine that builds work is layered above the engine that
//! runs it).
//!
//! # Why "Morphic"
//!
//! The *executed* graph physically changes shape as it runs. Agentic loops,
//! conditional branches, and runtime-decided fan-out are NOT static cycles —
//! they are expressed via **topology shapers** (`kx_mote::TopologyDecision`,
//! D23/D37): a `ReadOnlyNondet` Mote commits a declarative decision and the
//! projection materializes children deterministically. Each iteration is a
//! fresh Mote with a distinct `graph_position`, so the executed graph is always
//! an unrolled, replay-deterministic DAG. The graph *morphs* — hence Morphic.
//!
//! # Determinism is the contract
//!
//! [`compile`] is pure and total: the same [`WorkflowDef`] yields byte-identical
//! `MoteId`s across runs, processes, and machines. That makes a compiled
//! workflow a *reproducible program* — pin the seed + model + inference params
//! and a synthetic corpus regenerates bit-for-bit (D50). Identity is always
//! derived, never hand-assigned, and matched by exact cryptographic equality
//! (SN-8) — never similarity.
//!
//! # Shape
//!
//! - [`WorkflowDef`] / [`StepDef`] / [`StepRole`] / [`StepEdge`] / [`StepRef`] —
//!   the authoring surface (workflow as data).
//! - [`compile`] → [`CompiledWorkflow`] of [`CompiledMote`]s (mote + warrant +
//!   capability), in topological submission order.
//! - [`synthesis_pipeline`] + the [`generator`] / [`transform`] / [`critic`] /
//!   [`topology_shaper`] builders — the concrete data-synthesis recipe.

#![forbid(unsafe_code)]
// Pedantic lints kept as warnings workspace-wide; these few are noise for this
// crate's shape (mirrors the allow-set used by `kx-warrant`).
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::must_use_candidate
)]
// Inline test modules are exempt from the workspace deny on unwrap/expect.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod compile;
mod def;
mod error;
mod prompt;
mod recipes;
mod retrieval;
mod share;
mod synthesis;

pub use compile::compile;
pub use def::{CompiledMote, CompiledWorkflow, StepDef, StepEdge, StepRef, StepRole, WorkflowDef};
pub use error::CompileError;
pub use prompt::{put_rendered_prompt, render_prompts, PromptTemplate, TEMPLATE_KEY};
pub use recipes::{
    fan_out_gather, image_batch_describe_reduce, map_reduce, rag_pipeline, rag_pipeline_hybrid,
    react_tool_loop, retry_until_critic, WorkerKind, IMAGE_REF_KEY,
};
pub use retrieval::{encode_retrieval_fact, retrieval, retrieval_result_ref};
pub use share::{Manifest, ManifestId};
pub use synthesis::{
    critic, deterministic_critic, generator, judge, permissive_warrant, rewrite_query,
    synthesis_pipeline, tool_step, topology_shaper, transform,
};
// Re-export the check vocabulary so a workflow author building a critic / judge
// step depends on `kx-workflow` alone (the `deterministic_critic` / `judge`
// builders take a `CheckSpec`).
pub use kx_critic_types::{CheckSpec, LlmJudgeSpec};

#[cfg(test)]
mod tests;
