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
use crate::lower::{LoopProposal, ReplanProposal};
use crate::plan::{Envelope, LoopEnvelope, Plan, ReplanEnvelope};

/// Hard structural cap on declared steps — a `DoS` bound independent of the byte
/// cap. A plan with more steps is refused before lowering touches the graph.
pub const MAX_PLAN_STEPS: usize = 256;

/// Hard structural cap on declared edges (`DoS` bound).
pub const MAX_PLAN_EDGES: usize = 1024;

/// Hard structural cap on a single agentic-loop round's proposed steps — a `DoS`
/// bound independent of the byte cap, and DISTINCT from the cross-run round
/// budget (`kx_model_harness::LoopBudget::max_rounds`, enforced where the model
/// runs). A round proposing more children than this is refused before lowering.
/// Tighter than [`MAX_PLAN_STEPS`]: one re-plan round fans out far less than a
/// full authored plan.
pub const MAX_LOOP_STEPS: usize = 64;

/// Hard cap on a `flag_human` escalation reason (bytes) — defense-in-depth on top
/// of the overall `max_bytes` proposal cap, so the operator-facing reason can
/// never be an unbounded model-authored blob (PR-3 / AL2).
pub const MAX_FLAG_HUMAN_BYTES: usize = 1024;

/// The per-plan byte cap, derived from the warrant's output ceiling
/// (`max_output_tokens · 4` — the model produced the plan, so its output budget
/// bounds it). Saturating, mirroring `kx_model_harness::toolcall::max_args_bytes`
/// / `context::window_bytes_from_warrant`.
#[must_use]
pub fn max_plan_bytes(warrant: &WarrantSpec) -> usize {
    (warrant.model_route.max_output_tokens as usize).saturating_mul(4)
}

/// Extract the JSON envelope a model wrapped in reasoning and/or a markdown code
/// fence, so the strict parser sees the bare `{ … }`. Removes, in order:
///   1. a SINGLE leading reasoning block — Qwen3 `<think>…</think>` OR Gemma-4
///      `<|channel>…<channel|>` (the answer follows the close tag);
///   2. a surrounding markdown code fence — ```` ```json … ``` ```` (Gemma-4
///      reliably fences structured output) or a bare ```` ``` … ``` ````.
///
/// Model-agnostic, total + panic-free over arbitrary input. It ONLY strips known
/// wrappers — the strict, `deny_unknown_fields` serde parse downstream still gates
/// the envelope (fail-closed), so widening the accepted wrapper set never opens a
/// smuggling vector (the size cap on the ORIGINAL bytes still bounds the parse,
/// and extraction can only shrink). An unclosed reasoning tag yields `""`, which
/// the strict parse rejects (a plan is mandatory).
fn extract_json_envelope(text: &str) -> &str {
    strip_code_fence(strip_reasoning_preamble(text))
}

/// Strip a SINGLE leading reasoning block: Qwen3 `<think>…</think>` or Gemma-4
/// `<|channel>…<channel|>`. An unclosed tag yields `""`. Total + panic-free (the
/// ASCII close tag makes every post-tag slice boundary valid).
fn strip_reasoning_preamble(text: &str) -> &str {
    let t = text.trim_start();
    for (open, close) in [("<think>", "</think>"), ("<|channel>", "<channel|>")] {
        if let Some(rest) = t.strip_prefix(open) {
            return match rest.find(close) {
                Some(i) => rest[i + close.len()..].trim_start(),
                None => "",
            };
        }
    }
    t
}

/// Strip a surrounding markdown code fence (```` ``` ````), optionally with a
/// language tag (```` ```json ````). Returns the inner content trimmed; no fence
/// ⇒ `text` trimmed. Total + panic-free (the fence delimiter is ASCII, so every
/// slice boundary is valid).
fn strip_code_fence(text: &str) -> &str {
    let t = text.trim();
    let Some(rest) = t.strip_prefix("```") else {
        return t;
    };
    // Drop an optional language tag up to the first newline (```json\n…).
    let inner = match rest.find('\n') {
        Some(nl) => &rest[nl + 1..],
        None => rest,
    };
    // Drop the trailing closing fence if present.
    match inner.rfind("```") {
        Some(i) => inner[..i].trim(),
        None => inner.trim(),
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
    let stripped = extract_json_envelope(text);
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

/// Decode a model-proposed **agentic-loop round**, fail-closed.
///
/// Returns `Ok(LoopProposal)` only for a strict, size-bounded
/// `{"loop_proposal": {"version": 1, "next_steps": [ … ]}}` envelope with
/// `1..=MAX_LOOP_STEPS` steps. Returns `Err` for everything else — oversized
/// bytes, non-JSON / non-object / truncated / trailing-garbage / unexpected-key
/// payloads, an unknown version, an empty round, or an over-cap step count. A
/// leading `<think>…</think>` block (Qwen3 reasoning) is stripped before the
/// strict parse.
///
/// This is the loop counterpart of [`decode_plan`] and shares its exact
/// untrusted-bytes discipline (IMP-5): size-check BEFORE parse (so a hostile
/// model cannot force a large parse allocation), decode into fixed flat structs
/// (never a dynamic `serde_json::Value`, so no float/NaN/unbounded-recursion
/// path), and `deny_unknown_fields` on every struct (closing the "smuggle an
/// extra field" vector — a `confidence` channel can never reach the runtime, D77).
/// `max_bytes` is the warrant-derived output ceiling (`max_plan_bytes(warrant)`
/// — the model produced the proposal, so its output budget bounds it).
///
/// Total + panic-free over arbitrary `bytes`.
pub fn decode_loop_proposal(bytes: &[u8], max_bytes: usize) -> Result<LoopProposal, PlanError> {
    // (1) Size cap BEFORE parse — on the ORIGINAL bytes, so the `<think>` strip
    //     below can only ever shrink the parsed text.
    if bytes.len() > max_bytes {
        return Err(PlanError::Oversize {
            got: bytes.len(),
            max: max_bytes,
        });
    }

    // (2) UTF-8 (a proposal is mandatory — no `Ok(None)` arm), strip a leading
    //     Qwen3 `<think>…</think>` block, then parse strictly into fixed flat
    //     structs. `deny_unknown_fields` makes any unexpected key a hard refusal.
    let text = std::str::from_utf8(bytes).map_err(|_| PlanError::Malformed {
        diagnostic: "model output was not valid UTF-8".to_string(),
    })?;
    let stripped = extract_json_envelope(text);
    let envelope: LoopEnvelope =
        serde_json::from_str(stripped).map_err(|e| PlanError::Malformed {
            diagnostic: e.to_string(),
        })?;
    let wire = envelope.loop_proposal;

    // (3) Envelope invariants — fail closed on each. Reuse the closed PlanError
    //     vocabulary (Oversize / Malformed / UnknownVersion / EmptyPlan /
    //     TooManySteps): a loop round is a plan with no edges and a tighter cap.
    if wire.version != 1 {
        return Err(PlanError::UnknownVersion {
            version: wire.version,
        });
    }
    if wire.next_steps.is_empty() {
        return Err(PlanError::EmptyPlan);
    }
    if wire.next_steps.len() > MAX_LOOP_STEPS {
        return Err(PlanError::TooManySteps {
            got: wire.next_steps.len(),
            max: MAX_LOOP_STEPS,
        });
    }

    Ok(LoopProposal {
        next_steps: wire.next_steps,
    })
}

/// Decode a model-proposed **re-plan round** (PR-3 / AL2), fail-closed — the 3-way
/// router's trust boundary.
///
/// Returns `Ok(ReplanProposal::Topology(..))` for a strict, size-bounded
/// `{"replan": {"version": 1, "next_steps": [ … ]}}` envelope (the corrective
/// fan-out — corrected-context / permission-adapt), or
/// `Ok(ReplanProposal::FlagHuman(reason))` for `{"replan": {"version": 1,
/// "flag_human": "…"}}` (escalate). Exactly ONE of `next_steps` / `flag_human` may
/// be present: both, or neither, is an `Err`.
///
/// A SEPARATE boundary from [`decode_loop_proposal`] (the PR-2 initial-round
/// decode, kept byte-frozen) — but the IDENTICAL untrusted-bytes discipline
/// (IMP-5): size-check BEFORE parse, a leading `<think>` strip, decode into fixed
/// flat structs (never a dynamic `Value`), `deny_unknown_fields` on every struct
/// (no `confidence`/score smuggle — D77). The escalation reason is bounded by
/// [`MAX_FLAG_HUMAN_BYTES`]. `max_bytes` is the warrant-derived output ceiling.
///
/// Total + panic-free over arbitrary `bytes`.
pub fn decode_replan_proposal(bytes: &[u8], max_bytes: usize) -> Result<ReplanProposal, PlanError> {
    // (1) Size cap BEFORE parse — on the ORIGINAL bytes.
    if bytes.len() > max_bytes {
        return Err(PlanError::Oversize {
            got: bytes.len(),
            max: max_bytes,
        });
    }

    // (2) UTF-8, strip a leading `<think>…</think>`, then strict flat-struct parse.
    let text = std::str::from_utf8(bytes).map_err(|_| PlanError::Malformed {
        diagnostic: "model output was not valid UTF-8".to_string(),
    })?;
    let stripped = extract_json_envelope(text);
    let envelope: ReplanEnvelope =
        serde_json::from_str(stripped).map_err(|e| PlanError::Malformed {
            diagnostic: e.to_string(),
        })?;
    let wire = envelope.replan;

    // (3) Version — fail closed on anything but 1.
    if wire.version != 1 {
        return Err(PlanError::UnknownVersion {
            version: wire.version,
        });
    }

    // (4) The 3-way router: exactly one of next_steps / flag_human.
    let has_steps = !wire.next_steps.is_empty();
    match (has_steps, wire.flag_human) {
        // Corrective fan-out (corrected-context / permission-adapt).
        (true, None) => {
            if wire.next_steps.len() > MAX_LOOP_STEPS {
                return Err(PlanError::TooManySteps {
                    got: wire.next_steps.len(),
                    max: MAX_LOOP_STEPS,
                });
            }
            Ok(ReplanProposal::Topology(LoopProposal {
                next_steps: wire.next_steps,
            }))
        }
        // Escalate (flag-a-human) — bounded reason.
        (false, Some(reason)) => {
            if reason.len() > MAX_FLAG_HUMAN_BYTES {
                return Err(PlanError::Oversize {
                    got: reason.len(),
                    max: MAX_FLAG_HUMAN_BYTES,
                });
            }
            Ok(ReplanProposal::FlagHuman(reason))
        }
        // Neither: a round that proposes nothing is refused (empty), mirroring
        // `decode_loop_proposal`'s empty-round refusal.
        (false, None) => Err(PlanError::EmptyPlan),
        // Both: ambiguous — a re-plan round corrects OR escalates, never both.
        (true, Some(_)) => Err(PlanError::Malformed {
            diagnostic: "a re-plan round must propose next_steps OR flag_human, not both"
                .to_string(),
        }),
    }
}
