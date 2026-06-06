//! IMP-5 — the fail-closed decode of a **model-proposed** plan.
//!
//! Model output is untrusted. [`decode_plan`] turns raw bytes into a validated
//! [`Plan`] and is **total + panic-free** over arbitrary input. It mirrors the
//! shipped fail-closed decoders [`kx_model_harness::toolcall::parse_tool_call`]
//! and [`kx_mcp::decode_tool_result`]: size-checked BEFORE parsing, strict
//! envelope (`deny_unknown_fields`), and decoded into fixed flat structs — never
//! a dynamic `serde_json::Value`, so no float/NaN/unbounded-recursion path
//! exists. Unlike a tool call (which is *optional* — a normal completion is
//! `Ok(None)`), a plan is *mandatory*: anything that is not a well-formed
//! `{"plan": …}` envelope is an `Err`.

use kx_warrant::WarrantSpec;

use crate::error::PlanError;
use crate::plan::{Envelope, Plan};

/// Hard structural cap on declared steps — a `DoS` bound independent of the byte
/// cap. A plan with more steps is refused before lowering touches the graph.
pub const MAX_PLAN_STEPS: usize = 256;

/// Hard structural cap on declared edges (`DoS` bound).
pub const MAX_PLAN_EDGES: usize = 1024;

/// The per-plan byte cap, derived from the warrant's output ceiling
/// (`max_output_tokens · 4` — the model produced the plan, so its output budget
/// bounds it). Saturating, mirroring `kx_model_harness::toolcall::max_args_bytes`
/// / `context::window_bytes_from_warrant`.
#[must_use]
pub fn max_plan_bytes(warrant: &WarrantSpec) -> usize {
    (warrant.model_route.max_output_tokens as usize).saturating_mul(4)
}

/// Strip a single leading `<think>…</think>` reasoning block (Qwen3 thinking
/// mode) so the strict envelope parser sees the JSON plan that follows it.
///
/// ONLY a leading block is removed — never a mid-string scan. An unclosed
/// `<think>` (reasoning with no closing tag) yields `""`, which fails the
/// strict parse below (a plan is mandatory) — fail-closed.
///
/// Total + panic-free (`find` returns a valid byte boundary; the ASCII close
/// tag makes the post-tag slice boundary always valid).
fn strip_reasoning_preamble(text: &str) -> &str {
    const OPEN: &str = "<think>";
    const CLOSE: &str = "</think>";
    let t = text.trim_start();
    let Some(rest) = t.strip_prefix(OPEN) else {
        return t;
    };
    match rest.find(CLOSE) {
        Some(i) => rest[i + CLOSE.len()..].trim_start(),
        None => "",
    }
}

/// Decode a model-proposed plan, fail-closed.
///
/// Returns `Ok(plan)` only for a strict, size-bounded `{"plan": …}` envelope with
/// `version == 1` and `1..=MAX_PLAN_STEPS` steps / `..=MAX_PLAN_EDGES` edges.
/// Returns `Err` for everything else — oversized bytes, non-JSON / non-object /
/// truncated / trailing-garbage / unexpected-key payloads, an unknown version,
/// an empty plan, or an over-cap step/edge count. A leading `<think>…</think>`
/// block (Qwen3 reasoning) is stripped before the strict parse.
///
/// Total + panic-free over arbitrary `bytes`.
pub fn decode_plan(bytes: &[u8], max_plan_bytes: usize) -> Result<Plan, PlanError> {
    // (1) Size cap BEFORE parse — a hostile model cannot force a large parse
    //     allocation by overshooting the budget. The cap is on the ORIGINAL
    //     bytes, so the `<think>` strip below can only ever shrink the parse.
    if bytes.len() > max_plan_bytes {
        return Err(PlanError::Oversize {
            got: bytes.len(),
            max: max_plan_bytes,
        });
    }

    // (2) Require UTF-8 (a plan is mandatory — no `Ok(None)` arm), strip a
    //     leading Qwen3 `<think>…</think>` block, then parse strictly into fixed
    //     flat structs. `serde_json::from_str` is total over arbitrary text
    //     (non-JSON / non-object / truncation / trailing garbage / unknown keys
    //     all → Err, never panic). `deny_unknown_fields` (on every plan struct)
    //     makes an unexpected key a hard refusal, closing the "smuggle an extra
    //     field" vector.
    let text = std::str::from_utf8(bytes).map_err(|_| PlanError::Malformed {
        diagnostic: "model output was not valid UTF-8".to_string(),
    })?;
    let stripped = strip_reasoning_preamble(text);
    let envelope: Envelope = serde_json::from_str(stripped).map_err(|e| PlanError::Malformed {
        diagnostic: e.to_string(),
    })?;
    let plan = envelope.plan;

    // (3) Envelope invariants — fail closed on each.
    if plan.version != 1 {
        return Err(PlanError::UnknownVersion {
            version: plan.version,
        });
    }
    if plan.steps.is_empty() {
        return Err(PlanError::EmptyPlan);
    }
    if plan.steps.len() > MAX_PLAN_STEPS {
        return Err(PlanError::TooManySteps {
            got: plan.steps.len(),
            max: MAX_PLAN_STEPS,
        });
    }
    if plan.edges.len() > MAX_PLAN_EDGES {
        return Err(PlanError::TooManyEdges {
            got: plan.edges.len(),
            max: MAX_PLAN_EDGES,
        });
    }

    Ok(plan)
}
