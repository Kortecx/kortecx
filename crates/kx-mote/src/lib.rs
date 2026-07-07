#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::doc_markdown,
    clippy::return_self_not_must_use,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_lossless,
    clippy::unnested_or_patterns,
    clippy::redundant_closure_for_method_calls
)]
// `.expect()` on canonical-bincode encode of types without floats and without
// non-encodable variants IS infallible. Each site below carries an inline
// message naming the precondition; the lint allow at the crate level
// suppresses the workspace `clippy::expect_used = "deny"` policy specifically
// for these legitimate documented uses.
//   - MoteDef::hash (line ~594): MoteDef has no floats / no non-encodable
//     fields; canonical_config is a frozen bincode config.
//   - TopologyDecision::hash (line ~1008): same — pure struct of plain types.
#![allow(clippy::expect_used)]
// Inline test modules (`#[cfg(test)] mod tests { ... }`) are exempted from
// the workspace `unwrap_used` deny policy. `expect_used` is already allowed
// unconditionally above (production-code documented uses); `unwrap_used` is
// allowed only under cfg(test). Integration tests under `tests/*.rs`
// compile as separate crates and carry their own per-file allows.
#![cfg_attr(test, allow(clippy::unwrap_used))]

//! # kx-mote — the atomic execution unit
//!
//! This crate defines the *Mote*: the smallest indivisible thing the kortecx runtime
//! schedules, recovers, and reasons about. Per the design corpus (private), a Mote is
//! a durably-recorded record-of-an-attempt-to-effect-something — the deliberate
//! inversion of Spark's recomputable RDD. A committed Mote is a fact in the journal
//! and is never re-run; recovery is replay-from-journal.
//!
//! The crate is a *pure-types* crate. It has **no I/O, no async, and no runtime
//! dependencies** — only `blake3`, `serde`, `bincode`, `smallvec`, and `thiserror`.
//! Every downstream kortecx crate (journal, projection, executor, scheduler, runtime)
//! imports types from here; it is the narrow waist.
//!
//! ## What lives here
//!
//! - The [`MoteId`] 32-byte identity type and its derivation from `MoteDef`-hash +
//!   `input_data_id` + `graph_position`.
//! - The [`MoteDef`] struct and its canonical [`MoteDef::hash`] over a frozen bincode
//!   configuration ([`canonical_config`]).
//! - The [`Mote`] runtime-instance type that composes a `MoteDef` with per-instance
//!   position data and parent edges.
//! - The non-determinism tag [`NdClass`] (PURE / READ-ONLY-NONDET / WORLD-MUTATING).
//! - The [`EffectPattern`] enum declaring which of the three effect/commit patterns
//!   a Mote uses (idempotent-by-construction / stage-then-commit / validate-then-commit).
//! - Typed dependency edges via [`EdgeKind`] and [`EdgeMeta`].
//! - The per-attempt lifecycle state machine ([`AttemptState`], [`transition`]).
//! - A minimal [`MoteGraph`] container for workflow-author-side composition.
//!
//! ## What does NOT live here
//!
//! - Journal types, content-store types, projection logic, executor logic, inference,
//!   networking — those land in their respective `kx-*` crates.
//! - Runtime behavior. This crate only defines the *shapes* the runtime moves around.

mod attempt;
mod context_items;
mod def;
mod edge;
mod effect;
mod graph;
mod id;
mod inference_params;
mod mote;
mod ndclass;
mod strings;
mod topology;

pub use attempt::{
    is_legal_transition, transition, AttemptState, IllegalTransition, ALL_ATTEMPT_STATES,
};
pub use context_items::{
    decode_context_items, encode_context_items, encode_context_items_ordered, ContextItemRef,
};
pub use def::{canonical_config, derive_mote_id, MoteDef, MOTE_DEF_SCHEMA_VERSION};
pub use edge::{EdgeKind, EdgeMeta, ParentRef};
pub use effect::EffectPattern;
pub use graph::MoteGraph;
pub use id::{InputDataId, LogicRef, MoteDefHash, MoteId, PromptTemplateHash};
pub use inference_params::{Grammar, InferenceParams};
pub use mote::Mote;
pub use ndclass::NdClass;
pub use strings::{
    ConfigKey, ConfigVal, GraphPosition, ModelId, ToolName, ToolVersion, CONSENSUS_VOTE_KEY,
    CONTEXT_ITEMS_KEY, IMAGE_REF_KEY, JUDGE_RUBRIC_KEY, PROMPT_KEY, REACT_INSTRUCTION_KEY,
    REACT_MAX_TOOL_CALLS_KEY, REACT_MAX_TURNS_KEY, REACT_REQUIRE_APPROVAL_KEY, REACT_TURN_KEY,
    REACT_UNCONSTRAINED_KEY, RERANK_CANDIDATES_KEY, RERANK_QUERY_KEY, RERANK_TURN_KEY,
    RETRIEVAL_MODE_KEY, TOOL_ARGS_KEY,
};
pub use topology::{ChildDescriptor, RoleId, TopologyDecision};

#[cfg(test)]
mod tests;
