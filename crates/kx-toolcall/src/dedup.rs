//! RC2 loop-hardening — redundant-call detection (the minimal dedup slice).
//!
//! A small agentic model sometimes loops: it re-proposes the SAME tool with the
//! SAME arguments turn after turn instead of using the result it already has. This
//! is an *efficiency* problem, never an authority one — so the helpers here are
//! pure comparisons, kept in the shared gate crate so the live coordinator settle
//! and the harness `ReAct` loop render the refusal BYTE-IDENTICAL (the §2.274 twin
//! discipline): the reason flows into the re-prompted turn's `MoteId`, so a cold
//! re-fold during recovery must reproduce the exact same string.

use crate::types::ToolCall;

/// The stable distinguishing phrase of [`duplicate_call_reason`] — the ONLY react
/// reject reason that contains it (every other reason opens differently: "the tool
/// proposal was malformed…", "the proposed tool `…` is not granted…", "the tool name
/// `…` is ambiguous…", "the arguments for `…` do not match…", "the model proposed N
/// tool calls…"). `T-GEMMA3-TOOL-LOOP-ANSWER-FORCE`: the gateway matches this marker in
/// the frozen re-prompt instruction to arm the answer-only decode constraint — pinned as
/// a real substring of the reason by `marker_is_a_substring_of_the_reason` (a unit test), so
/// the generator and the matcher cannot drift. Positioned early in the reason (right after
/// the tool id), so `bounded_reason`'s 512-char cap (`kx_journal::MAX_REJECTED_REASON_LEN`)
/// only drops it for a pathologically long (>~440-char) tool id — not any real MCP tool.
pub const DUPLICATE_REJECT_MARKER: &str = "already called this run with identical arguments";

/// The stable distinguishing phrase of the coordinator's `render_settle_nudge`
/// (`kx_coordinator::react_shape`) — the near-budget "stop calling tools, answer now"
/// nudge. `T-GEMMA3-TOOL-LOOP-ANSWER-FORCE`: the gateway matches this marker in the frozen
/// nudge instruction to arm the answer-only decode constraint. Lives here (a shared dep of
/// both `kx-coordinator` and `kx-gateway`) so the render side and the match side reference
/// ONE literal; a `kx-coordinator` test pins that `render_settle_nudge` output contains it.
pub const SETTLE_NUDGE_MARKER: &str =
    "your tool-call budget is nearly exhausted. Do NOT call another tool";

/// `true` iff `reason` is (or embeds) the duplicate-call rejection — i.e. it contains
/// [`DUPLICATE_REJECT_MARKER`]. Used gateway-side against the frozen re-prompt
/// instruction (which embeds the rejection reason) to arm the answer-only decode
/// constraint ONLY for the duplicate-loop case, leaving genuine self-correction re-prompts
/// (bad args, ungranted, ambiguous) free to retry a tool. Pure + total.
#[must_use]
pub fn is_duplicate_reason(reason: &str) -> bool {
    reason.contains(DUPLICATE_REJECT_MARKER)
}

/// `true` iff `call` exactly repeats one of `prior` — the same resolved tool
/// `(name, version)` AND byte-identical `args_bytes`. **Conservative**: only a
/// truly identical re-proposal matches, so a legitimate retry with refined or
/// different arguments always fires. This is purely an efficiency guard; the
/// warrant grant-check + `inputSchema` validation remain the independent authority
/// gate (SN-8).
#[must_use]
pub fn is_duplicate_call(call: &ToolCall, prior: &[ToolCall]) -> bool {
    prior.iter().any(|p| {
        p.name == call.name && p.version == call.version && p.args_bytes == call.args_bytes
    })
}

/// The fail-closed reason a settle freezes when a proposal exactly repeats a call
/// already fired this run. The re-prompt built from it steers the model to use the
/// result it already has, or to answer — bounded loop progress instead of a wasted
/// re-fire. Shared so the coordinator + harness render it byte-identical.
#[must_use]
pub fn duplicate_call_reason(call: &ToolCall) -> String {
    format!(
        "the tool `{}@{}` was already called this run with identical arguments — \
         use the result you already have, or answer the question directly",
        call.name.0, call.version.0
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use kx_mote::{ToolName, ToolVersion};

    fn call(name: &str, args: &[u8]) -> ToolCall {
        ToolCall {
            name: ToolName(name.into()),
            version: ToolVersion("1".into()),
            args_bytes: args.to_vec(),
        }
    }

    #[test]
    fn exact_repeat_is_a_duplicate() {
        let prior = vec![call("calc/add", br#"{"a":1,"b":2}"#)];
        assert!(is_duplicate_call(
            &call("calc/add", br#"{"a":1,"b":2}"#),
            &prior
        ));
    }

    #[test]
    fn different_args_are_not_a_duplicate() {
        let prior = vec![call("calc/add", br#"{"a":1,"b":2}"#)];
        // Different args ⇒ a legitimate new call, never suppressed.
        assert!(!is_duplicate_call(
            &call("calc/add", br#"{"a":1,"b":3}"#),
            &prior
        ));
        // Different tool ⇒ not a duplicate.
        assert!(!is_duplicate_call(
            &call("kv/get", br#"{"a":1,"b":2}"#),
            &prior
        ));
        // No prior calls ⇒ never a duplicate.
        assert!(!is_duplicate_call(
            &call("calc/add", br#"{"a":1,"b":2}"#),
            &[]
        ));
    }

    #[test]
    fn reason_names_the_tool() {
        let r = duplicate_call_reason(&call("calc/add", b"{}"));
        assert!(r.contains("calc/add@1"));
        assert!(r.contains("already called"));
    }

    #[test]
    fn marker_is_a_substring_of_the_reason() {
        // Pins the DUPLICATE_REJECT_MARKER as a real substring of duplicate_call_reason —
        // if the reason text ever changes and drops the phrase, this fails (the gateway
        // matcher relies on it). We MUST NOT change the reason text to fix a break here (it
        // is harness-twin/golden byte-pinned) — instead update the marker to a new substring.
        let r = duplicate_call_reason(&call("calc/add", b"{}"));
        assert!(
            r.contains(DUPLICATE_REJECT_MARKER),
            "reason must carry the marker: {r}"
        );
        assert!(is_duplicate_reason(&r));
    }

    #[test]
    fn is_duplicate_reason_rejects_other_reject_reasons() {
        // The marker is unique to the duplicate reason — the other react reject reasons
        // (verbatim from kx-coordinator::state) must NOT match, so a genuine self-correction
        // re-prompt is never mistaken for a loop.
        for other in [
            "the tool proposal was malformed: unexpected token",
            "the proposed tool `x@1` is not granted to this run",
            "the tool name `echo` is ambiguous — use the full id: mcp-echo/echo",
            "the arguments for `calc/add@1` do not match its schema",
            "the model proposed 9 tool calls in one turn, exceeding the per-turn batch cap of 8",
        ] {
            assert!(!is_duplicate_reason(other), "must not match: {other}");
        }
    }

    #[test]
    fn marker_survives_truncation_for_realistic_tool_ids() {
        // The marker sits early in the reason (right after the tool id), so `bounded_reason`'s
        // 512-char cap keeps it for any realistic MCP tool id. Guard with a deliberately long
        // (100-char) namespaced tool name: the marker still survives a truncate-to-512.
        let long_name = format!("really-long-connector-namespace/{}", "x".repeat(70));
        let r = duplicate_call_reason(&call(&long_name, b"{}"));
        let truncated: String = r.chars().take(512).collect();
        assert!(
            is_duplicate_reason(&truncated),
            "marker must survive truncation for realistic tool ids: {truncated}"
        );
    }
}
