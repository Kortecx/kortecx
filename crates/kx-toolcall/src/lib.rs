//! IMP-5 — the fail-closed decode of a **model-proposed** tool call.
//!
//! M5.1 put a tool *menu* in front of the model; M5.2 lets the model *pick* one.
//! Model output is untrusted: [`parse_tool_call`] decodes it into a validated
//! [`ToolCall`] (or `None` for a normal completion) and is **total + panic-free**
//! over arbitrary bytes. "Model proposes, runtime enforces" (SN-8): the only tools
//! a proposal may name are those already in `warrant.tool_grants` — selection is
//! exact (crypto-equality of the `(name, version)` grant), never fuzzy. The broker
//! re-checks the grant at dispatch; this is the first, defense-in-depth gate.
//!
//! The decoded `args_bytes` are carried VERBATIM (the args object's bytes) into the
//! `EffectRequest.payload` — validated for *shape* (well-formed JSON), never
//! executed, never interpreted into a dynamic value here.
//!
//! **Why a crate (PR-2d-1)**: the gate moved here from `kx-model-harness` so the
//! live-serve gateway (pre-commit fence) and the coordinator (settle decode) share
//! the ONE implementation with the harness `ReAct` loop. This is an *authority gate*:
//! a byte-mirror that drifted would fail **silently** — it would admit an ungranted
//! tool — so it is extracted (moved), never forked. Pure leaf: depends only on
//! `kx-mote`, `kx-warrant`, and serde.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod dedup;
mod parse;
mod types;

pub use dedup::{duplicate_call_reason, is_duplicate_call};
pub use parse::{max_args_bytes, parse_permutation, parse_tool_call, parse_tool_calls};
pub use types::{DecodeError, ToolCall};
