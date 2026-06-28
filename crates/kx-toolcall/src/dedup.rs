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
}
