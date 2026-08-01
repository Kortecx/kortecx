//! PR-3 (A2) graceful-recovery REASON helpers for the harness ReAct loop.
//!
//! These are DELIBERATE byte-for-byte twins of the live serve coordinator's
//! `kx_coordinator::react_shape::{bounded_reason, render_reprompt}` and the
//! coordinator's inline `DecodeError`→reason formatter (`state.rs`, the settle
//! authority site). They live HERE (not shared) for the same reason the coordinator
//! re-implements the harness's turn builder rather than depending on it: the
//! coordinator sits BELOW a dep wall and cannot depend on `kx-model-harness`, so a
//! single shared home would have to be a lower crate. `render_reprompt` is pure over
//! `&str` and `decode_error_reason` consumes `kx_toolcall::DecodeError` (which both
//! crates already depend on), so the natural shared home is `kx-toolcall` — but
//! `bounded_reason` needs `kx_journal::MAX_REJECTED_REASON_LEN`, and `kx-toolcall`
//! must NOT take a `kx-journal` dependency (it is the dependency-light authority
//! leaf). Consolidating all three into one crate is a follow-up refactor (its
//! own PR). For now the twin is guarded against drift by
//! `reprompt_text_matches_the_coordinator` (a byte-equality unit test pinning the
//! exact strings) so the cross-impl re-prompted-turn `MoteId` stays identical (R49):
//! a re-prompted turn's identity rides its instruction (`PROMPT_KEY`), so if the
//! harness and coordinator render the SAME re-prompt for the same refusal, the
//! turn builders (already pinned equivalent at turn 0) derive the same `MoteId`.

use crate::toolcall::DecodeError;

/// Twin of `kx_coordinator::react_shape::bounded_reason`: truncate a refusal reason
/// to [`kx_journal::MAX_REJECTED_REASON_LEN`] chars at a char boundary (deterministic,
/// panic-free, total). The harness never writes the reason to a journal entry, but it
/// bounds identically so a re-prompted turn's bytes match the coordinator's (R49).
#[must_use]
pub(crate) fn bounded_reason(reason: String) -> String {
    if reason.chars().count() <= kx_journal::MAX_REJECTED_REASON_LEN {
        reason
    } else {
        reason
            .chars()
            .take(kx_journal::MAX_REJECTED_REASON_LEN)
            .collect()
    }
}

/// Render the closed `DecodeError` refusal vocabulary into the SAME reason text the
/// coordinator freezes onto a `ReactBranch::Rejected` fact (the accept-side variants
/// `parse_tool_call` can return). The harness's registry-lookup + schema-validate
/// failures happen later (at dispatch, not decode), so — exactly like the
/// coordinator's decode site — only these variants reach the A2 re-prompt; a tool that
/// resolves but whose dispatch fails keeps the harness's existing fail-closed stop.
/// The `Ambiguous` arm names the candidate full-ids so the re-prompt STEERS the model
/// to a unique `<server>/<remote>` id (T-CONNECTOR-AUTOGRANT-LIVE-DEADLETTER).
#[must_use]
pub(crate) fn decode_error_reason(error: &DecodeError) -> String {
    match error {
        DecodeError::Malformed { diagnostic } => {
            format!("the tool proposal was malformed: {diagnostic}")
        }
        DecodeError::UngrantedTool { name, version } => format!(
            "the proposed tool `{}@{}` is not granted to this run",
            name.0, version.0
        ),
        DecodeError::Ambiguous { name, candidates } => format!(
            "the tool name `{}` is ambiguous — use the full id: {}",
            name.0,
            candidates
                .iter()
                .map(|c| c.0.as_str())
                .collect::<Vec<_>>()
                .join(" OR ")
        ),
        DecodeError::Oversize { got, max } => {
            format!("the proposed tool arguments are too large ({got} bytes > {max} max)")
        }
    }
}

/// Twin of `kx_coordinator::react_shape::render_reprompt`: append the fail-closed
/// `reason` plus the fixed self-correct steer to the next turn's instruction after a
/// refused proposal. PURE + total + deterministic (a function of `(base, reason)`),
/// so the harness drive and a cold re-fold build the byte-identical re-prompted turn.
#[must_use]
pub(crate) fn render_reprompt(base_instruction: &str, reason: &str) -> String {
    format!(
        "{base_instruction}\n\n[Your previous tool call was REJECTED: {reason}\n\
         Correct it — call a tool you were granted with arguments that match its \
         schema, or answer the question directly if you cannot.]"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use kx_mote::{ToolName, ToolVersion};

    /// The unreadable-tool-call reason, spelled as a LITERAL on this side of the dep
    /// wall. Its twin is `kx_coordinator::react_shape::tests`. Literals on both sides
    /// rather than an equality between the two call sites: the renderer is shared
    /// (`kx_toolcall::unreadable_tool_call_reason`), so comparing the drivers to each
    /// other would compare a function to itself and pass however wrong the text became.
    /// This reason folds into the re-prompted turn's `MoteId`, so a wording change is a
    /// chain-identity change and must redden a test on BOTH sides.
    #[test]
    fn unreadable_tool_call_reason_matches_the_coordinator() {
        assert_eq!(
            crate::toolcall::unreadable_tool_call_reason(),
            "your reply looked like a tool call but could not be read as one. Emit \
             EXACTLY one JSON object and nothing else: \
             {\"tool_call\":{\"name\":\"<tool name>\",\"version\":\"<tool version>\",\
             \"args\":{ ... }}} — using a name and version from the tool list. If you did \
             not mean to call a tool, reply with your final answer as plain text."
        );
        // It survives the bound intact, so the envelope always reaches the model.
        let bounded = bounded_reason(crate::toolcall::unreadable_tool_call_reason());
        assert_eq!(bounded, crate::toolcall::unreadable_tool_call_reason());
    }

    #[test]
    fn reprompt_text_matches_the_coordinator() {
        // The EXACT bytes the coordinator's `render_reprompt` produces (react_shape.rs)
        // — a drift here would diverge the re-prompted-turn MoteId across the dep wall.
        let r = render_reprompt("list the files", "tool `x@1` is not granted to this run");
        assert_eq!(
            r,
            "list the files\n\n[Your previous tool call was REJECTED: \
             tool `x@1` is not granted to this run\nCorrect it — call a tool you \
             were granted with arguments that match its schema, or answer the \
             question directly if you cannot.]"
        );
    }

    #[test]
    fn decode_error_reason_covers_every_variant() {
        assert_eq!(
            decode_error_reason(&DecodeError::Malformed {
                diagnostic: "trailing garbage".into()
            }),
            "the tool proposal was malformed: trailing garbage"
        );
        assert_eq!(
            decode_error_reason(&DecodeError::UngrantedTool {
                name: ToolName("mcp-danger".into()),
                version: ToolVersion("1".into()),
            }),
            "the proposed tool `mcp-danger@1` is not granted to this run"
        );
        // T-CONNECTOR-AUTOGRANT: the disambiguating reason names the candidate full-ids
        // (the EXACT bytes the coordinator's settle site freezes — twin-pinned).
        assert_eq!(
            decode_error_reason(&DecodeError::Ambiguous {
                name: ToolName("echo".into()),
                candidates: vec![
                    ToolName("mcp-echo/echo".into()),
                    ToolName("refconn/echo".into()),
                ],
            }),
            "the tool name `echo` is ambiguous — use the full id: mcp-echo/echo OR refconn/echo"
        );
        assert_eq!(
            decode_error_reason(&DecodeError::Oversize { got: 99, max: 10 }),
            "the proposed tool arguments are too large (99 bytes > 10 max)"
        );
    }

    /// v18 — the CROSS-IMPL golden for a failed OBSERVATION's reason, with the
    /// downstream system's own diagnostic carried.
    ///
    /// **Written as LITERAL bytes on both sides on purpose.** The renderer itself
    /// lives in `kx-journal`, so asserting `harness(x) == coordinator(x)` would be
    /// `f(x) == f(x)` — true no matter what either driver does, and true even if the
    /// text became wrong. The non-vacuous statement is a THIRD one: each crate spells
    /// out the exact bytes it expects. A deliberate wording change then reddens both
    /// tests (that is the review gate), while a one-sided drift is impossible because
    /// neither crate owns the string.
    ///
    /// The bytes matter beyond aesthetics: this reason folds into the re-prompted
    /// turn's `MoteId`, so the served coordinator and the single-node harness must
    /// produce the identical chain identity for the identical failure.
    #[test]
    fn observation_failure_reason_with_a_detail_matches_the_coordinator() {
        let r = kx_journal::observation_failure_reason(
            "fleet/get",
            "1",
            Some(kx_journal::FailureReason::DeadLettered),
            r#"MCP error -32004: no such vessel "x""#,
        );
        assert_eq!(
            r,
            "the tool `fleet/get@1` was called but did not complete — it failed to \
             run: MCP error -32004: no such vessel \"x\". Use that reason to decide \
             what to do next — correct the arguments and call it again, use a \
             different tool, or answer with what you already have."
        );
    }

    /// The no-detail arm, pinned SEPARATELY because it must stay byte-identical to
    /// what shipped before the detail existed — every chain whose reporter said
    /// nothing more specific has to rebuild the same `MoteId`, so no committed
    /// baseline moves on a path this change does not reach.
    #[test]
    fn observation_failure_reason_without_a_detail_is_unchanged() {
        let r = kx_journal::observation_failure_reason(
            "fleet/get",
            "1",
            Some(kx_journal::FailureReason::DeadLettered),
            "",
        );
        assert_eq!(
            r,
            "the tool `fleet/get@1` was called but did not complete — it failed to \
             run. Do not call it again with the same arguments; use a different tool \
             or different arguments, or answer with what you already have."
        );
    }

    #[test]
    fn bounded_reason_truncates_at_a_char_boundary_total() {
        let short = "short".to_string();
        assert_eq!(bounded_reason(short.clone()), short);
        let long: String = "é".repeat(kx_journal::MAX_REJECTED_REASON_LEN + 50);
        let bounded = bounded_reason(long);
        assert_eq!(bounded.chars().count(), kx_journal::MAX_REJECTED_REASON_LEN);
        assert_eq!(bounded_reason(bounded.clone()), bounded, "idempotent");
    }
}
