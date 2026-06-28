//! kortecx **cost-spend price-book** (M11 / D115) — the thin, FFI-free leaf that
//! turns the runtime's DURABLE turn/tool counters into an integer micro-USD spend
//! ESTIMATE.
//!
//! ## What this is (and is NOT)
//! This is the **OSS local guardrail**: a deterministic, operator-priced estimate
//! of a run's spend, used to enforce the `cost_ceiling` warrant axis at the broker
//! precheck (a `FinOps` ceiling AND a runaway-agent kill-switch). It is **NOT** Cloud
//! per-token billing/metering — OSS surfaces no input-token / price-per-expert data
//! (D129 / D156 / GR19). Cloud swaps a rich per-token price-book behind this same
//! shape (D129 / D170.b "OSS simple seam, Cloud rich impl").
//!
//! ## Invariants
//! - **Integer micro-USD only** — never a float on any path (SN-8); a dollar
//!   ceiling is an exact enforcement decision, never a fuzzy score (D115.2).
//! - **A pure, total fold** — `spent = turns·per_turn + tool_calls·per_tool_call`,
//!   saturating so a runaway can never wrap the ceiling. The inputs are the
//!   coordinator's recovery-stable fold over committed `ReactRound` facts (the
//!   `turns_used` / `tool_calls` counters), so the spend is itself recovery-stable
//!   and re-derived per pass — never an authoritative live counter (D115.2).
//! - **Off the journal + identity path** — this crate is a leaf TYPE crate with no
//!   dependencies; the frozen trio + the journal NEVER depend on it.

mod price_book;

pub use crate::price_book::{
    PriceBook, DEFAULT_PER_TOOL_CALL_MICRO_USD, DEFAULT_PER_TURN_MICRO_USD,
    ENV_PER_TOOL_CALL_MICRO_USD, ENV_PER_TURN_MICRO_USD,
};
