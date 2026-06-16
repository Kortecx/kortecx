//! The fail-closed parse: [`parse_tool_call`] + the args-size cap
//! [`max_args_bytes`]. Moved verbatim from `kx-model-harness::toolcall`
//! (PR-2d-1); the 13 gate tests moved with it and pin the behavior.

use kx_mote::{ToolName, ToolVersion};
use kx_warrant::{ToolGrant, WarrantSpec};
use serde::Deserialize;
use serde_json::value::RawValue;

use crate::types::{DecodeError, ToolCall};

/// The JSON envelope a model uses to propose a tool call:
/// `{"tool_call": {"name": "...", "version": "...", "args": { ... }}}`.
#[derive(Deserialize)]
struct Envelope {
    #[serde(default)]
    tool_call: Option<RawToolCall>,
}

#[derive(Deserialize)]
struct RawToolCall {
    name: String,
    version: String,
    args: Box<RawValue>,
}

/// The per-call args-size cap (IMP-16), derived from the warrant's output ceiling
/// (`max_output_tokens · 4` — the model produced the args, so the output budget
/// bounds them). Saturating, mirroring `context::window_bytes_from_warrant`.
#[must_use]
pub fn max_args_bytes(warrant: &WarrantSpec) -> usize {
    (warrant.model_route.max_output_tokens as usize).saturating_mul(4)
}

/// Extract the JSON envelope a model wrapped in reasoning and/or a markdown code
/// fence, so the strict parser sees the bare `{ … }`. Removes a SINGLE leading
/// reasoning block — Qwen3 `<think>…</think>` OR Gemma-4 `<|channel>…<channel|>`
/// — then a surrounding markdown code fence (```` ```json … ``` ````; Gemma-4
/// reliably fences structured output).
///
/// Leading-block + structural-wrapper ONLY — we NEVER scan for `{` mid-string
/// (the fence is a defined ```` ``` ```` delimiter, not a `{` search), so the
/// strict `starts_with('{')` gate below stays the injection boundary (SN-8).
/// Mirrors `kx_planner::decode`'s extractor — the two trust seams keep the SAME
/// discipline. Total + panic-free; an unclosed reasoning tag yields `""`, which
/// the caller treats as a normal (non-call) completion (fail-closed).
fn extract_json_envelope(text: &str) -> &str {
    strip_code_fence(strip_reasoning_preamble(text))
}

/// Strip a SINGLE leading reasoning block: Qwen3 `<think>…</think>` or Gemma-4
/// `<|channel>…<channel|>`. An unclosed tag yields `""`.
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

/// Strip a surrounding markdown code fence (```` ``` ````), optionally tagged
/// (```` ```json ````). No fence ⇒ `text` trimmed. Total + panic-free.
fn strip_code_fence(text: &str) -> &str {
    let t = text.trim();
    let Some(rest) = t.strip_prefix("```") else {
        return t;
    };
    let inner = match rest.find('\n') {
        Some(nl) => &rest[nl + 1..],
        None => rest,
    };
    match inner.rfind("```") {
        Some(i) => inner[..i].trim(),
        None => inner.trim(),
    }
}

/// Decode a model-proposed tool call from raw model output, fail-closed.
///
/// Returns `Ok(None)` for a normal completion (prose, non-envelope JSON, or — the
/// important security default — *any* output when the warrant grants no tools).
/// Returns `Ok(Some(call))` for a well-formed, warrant-granted, size-bounded call.
/// Returns `Err` when the model committed to a tool-call envelope that is malformed,
/// names an ungranted tool, or overshoots the args cap.
///
/// A leading `<think>…</think>` block (Qwen3 reasoning) is stripped before the
/// strict parse; everything after it is still gated by `starts_with('{')`.
///
/// Total + panic-free over arbitrary `bytes`.
///
/// # Errors
///
/// [`DecodeError::Malformed`] when the output committed to a JSON object but the
/// envelope is malformed/truncated/trailing-garbage; [`DecodeError::UngrantedTool`]
/// when the proposal names a tool outside `warrant.tool_grants` (SN-8);
/// [`DecodeError::Oversize`] when the args exceed `max_args_bytes` (IMP-16).
pub fn parse_tool_call(
    bytes: &[u8],
    warrant: &WarrantSpec,
    max_args_bytes: usize,
) -> Result<Option<ToolCall>, DecodeError> {
    // (0) No grants ⇒ no tool can ever be called. Preserves the M5.1 leaf path
    //     byte-for-byte (every existing harness row grants no tools) AND is the
    //     security default: a model cannot conjure a tool the warrant withheld.
    if warrant.tool_grants.is_empty() {
        return Ok(None);
    }

    // (1) Non-UTF-8 or not-a-JSON-object output is a normal completion, not a call.
    let Ok(text) = std::str::from_utf8(bytes) else {
        return Ok(None);
    };
    // Strip a leading reasoning block (Qwen3 `<think>` / Gemma-4 `<|channel>`) and
    // a surrounding ```` ```json ```` fence, then require the remainder to BEGIN
    // with `{` — leading-block + structural-fence only; no mid-string scan (SN-8).
    let trimmed = extract_json_envelope(text);
    if !trimmed.starts_with('{') {
        return Ok(None);
    }

    // (2) It looks like JSON. Parse strictly — trailing garbage / truncation /
    //     bad shape is fail-closed (the injection vector lives here).
    let envelope: Envelope = serde_json::from_str(trimmed).map_err(|e| DecodeError::Malformed {
        diagnostic: e.to_string(),
    })?;
    let Some(raw) = envelope.tool_call else {
        // Valid JSON, but not a tool-call envelope ⇒ a normal completion.
        return Ok(None);
    };

    // (3) The model committed to a tool call. Enforce tool ∈ warrant.tool_grants
    //     by EXACT (name, version) crypto-equality — never fuzzy (SN-8 / D70).
    let name = ToolName(raw.name);
    let version = ToolVersion(raw.version);
    let grant = ToolGrant {
        tool_id: name.clone(),
        tool_version: version.clone(),
    };
    if !warrant.tool_grants.contains(&grant) {
        return Err(DecodeError::UngrantedTool { name, version });
    }

    // (4) Carry the args verbatim, size-capped (IMP-16).
    let args_bytes = raw.args.get().as_bytes().to_vec();
    if args_bytes.len() > max_args_bytes {
        return Err(DecodeError::Oversize {
            got: args_bytes.len(),
            max: max_args_bytes,
        });
    }

    Ok(Some(ToolCall {
        name,
        version,
        args_bytes,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use kx_warrant::{
        ExecutorClass, FsScope, ModelRoute, MoteClass, NetScope, ResourceCeiling, WarrantSpec,
    };
    use std::collections::BTreeSet;

    fn warrant_granting(tool: Option<(&str, &str)>) -> WarrantSpec {
        let mut tool_grants = BTreeSet::new();
        if let Some((id, ver)) = tool {
            tool_grants.insert(ToolGrant {
                tool_id: ToolName(id.into()),
                tool_version: ToolVersion(ver.into()),
            });
        }
        WarrantSpec {
            mote_class: MoteClass::WorldMutating,
            nd_class: MoteClass::WorldMutating,
            fs_scope: FsScope::empty(),
            net_scope: NetScope::None,
            syscall_profile_ref: kx_content::ContentRef::from_bytes([0; 32]),
            tool_grants,
            model_route: ModelRoute {
                model_id: kx_mote::ModelId("m".into()),
                max_input_tokens: 1024,
                max_output_tokens: 256,
                max_calls: 8,
            },
            resource_ceiling: ResourceCeiling {
                cpu_milli: 0,
                mem_bytes: 0,
                wall_clock_ms: 1000,
                fd_count: 0,
                disk_bytes: 0,
            },
            environment_ref: None,
            executor_class: ExecutorClass::Bwrap,
            ..Default::default()
        }
    }

    #[test]
    fn empty_grants_is_always_none() {
        let w = warrant_granting(None);
        // Even a perfectly-formed envelope yields None when nothing is granted.
        let env = br#"{"tool_call":{"name":"mcp-echo","version":"1","args":{}}}"#;
        assert_eq!(parse_tool_call(env, &w, 4096), Ok(None));
    }

    #[test]
    fn prose_is_a_normal_completion() {
        let w = warrant_granting(Some(("mcp-echo", "1")));
        assert_eq!(
            parse_tool_call(b"The sky is blue.", &w, 4096),
            Ok(None),
            "prose ⇒ no tool call"
        );
    }

    #[test]
    fn non_envelope_json_is_a_normal_completion() {
        let w = warrant_granting(Some(("mcp-echo", "1")));
        assert_eq!(parse_tool_call(br#"{"answer":"blue"}"#, &w, 4096), Ok(None));
    }

    #[test]
    fn well_formed_granted_call_is_decoded() {
        let w = warrant_granting(Some(("mcp-echo", "1")));
        let env = br#"{"tool_call":{"name":"mcp-echo","version":"1","args":{"q":"x"}}}"#;
        let call = parse_tool_call(env, &w, 4096).unwrap().expect("a call");
        assert_eq!(call.name, ToolName("mcp-echo".into()));
        assert_eq!(call.version, ToolVersion("1".into()));
        assert_eq!(call.args_bytes, br#"{"q":"x"}"#.to_vec());
    }

    #[test]
    fn think_preamble_then_envelope_is_decoded() {
        let w = warrant_granting(Some(("mcp-echo", "1")));
        // Qwen3 thinking-mode: a reasoning block precedes the tool-call JSON.
        let env = b"<think>I should echo the query.</think>\n{\"tool_call\":{\"name\":\"mcp-echo\",\"version\":\"1\",\"args\":{\"q\":\"x\"}}}";
        let call = parse_tool_call(env, &w, 4096).unwrap().expect("a call");
        assert_eq!(call.name, ToolName("mcp-echo".into()));
        assert_eq!(call.args_bytes, br#"{"q":"x"}"#.to_vec());
    }

    #[test]
    fn fenced_envelope_is_decoded() {
        let w = warrant_granting(Some(("mcp-echo", "1")));
        // Gemma-4 reliably fences structured output in a ```json block.
        let env =
            b"```json\n{\"tool_call\":{\"name\":\"mcp-echo\",\"version\":\"1\",\"args\":{\"q\":\"x\"}}}\n```";
        let call = parse_tool_call(env, &w, 4096)
            .unwrap()
            .expect("a fenced call");
        assert_eq!(call.name, ToolName("mcp-echo".into()));
        assert_eq!(call.args_bytes, br#"{"q":"x"}"#.to_vec());
    }

    #[test]
    fn gemma_channel_then_fenced_envelope_is_decoded() {
        let w = warrant_granting(Some(("mcp-echo", "1")));
        // Gemma-4: a `<|channel>thought…<channel|>` reasoning segment then a
        // fenced tool-call.
        let env = b"<|channel>thought\nI should echo.<channel|>```json\n{\"tool_call\":{\"name\":\"mcp-echo\",\"version\":\"1\",\"args\":{}}}\n```";
        let call = parse_tool_call(env, &w, 4096)
            .unwrap()
            .expect("a channel + fence call");
        assert_eq!(call.name, ToolName("mcp-echo".into()));
    }

    #[test]
    fn think_only_no_json_is_normal_completion() {
        let w = warrant_granting(Some(("mcp-echo", "1")));
        // Reasoning then prose (no JSON) ⇒ not a tool call.
        let env = b"<think>hmm</think>\nThe answer is blue.";
        assert_eq!(parse_tool_call(env, &w, 4096), Ok(None));
    }

    #[test]
    fn unclosed_think_is_normal_completion() {
        let w = warrant_granting(Some(("mcp-echo", "1")));
        // An unterminated reasoning block strips to "" ⇒ fail-closed to None.
        let env = b"<think>reasoning with no closing tag and no json";
        assert_eq!(parse_tool_call(env, &w, 4096), Ok(None));
    }

    #[test]
    fn think_does_not_enable_midstring_injection() {
        // A `<think>` block whose body contains a JSON-looking object must NOT
        // be parsed as the call — only what FOLLOWS `</think>` is considered,
        // and here that's prose ⇒ None (the strict starts_with('{') gate holds).
        let w = warrant_granting(Some(("mcp-danger", "1")));
        let env = b"<think>{\"tool_call\":{\"name\":\"mcp-danger\",\"version\":\"1\",\"args\":{}}}</think> nope";
        assert_eq!(parse_tool_call(env, &w, 4096), Ok(None));
    }

    #[test]
    fn garbled_envelope_is_malformed_not_silently_dropped() {
        let w = warrant_granting(Some(("mcp-echo", "1")));
        // Started as a JSON object (committed to a call) but truncated → fail-closed.
        let env = br#"{"tool_call":{"name":"mcp-echo","version":"#;
        assert!(matches!(
            parse_tool_call(env, &w, 4096),
            Err(DecodeError::Malformed { .. })
        ));
    }

    #[test]
    fn trailing_garbage_after_envelope_is_malformed() {
        let w = warrant_granting(Some(("mcp-echo", "1")));
        let env = br#"{"tool_call":{"name":"mcp-echo","version":"1","args":{}}} then prose"#;
        assert!(matches!(
            parse_tool_call(env, &w, 4096),
            Err(DecodeError::Malformed { .. })
        ));
    }

    #[test]
    fn ungranted_tool_is_refused() {
        let w = warrant_granting(Some(("mcp-echo", "1")));
        // Right shape, but names a tool/version not in the grant set.
        let env = br#"{"tool_call":{"name":"mcp-danger","version":"1","args":{}}}"#;
        assert!(matches!(
            parse_tool_call(env, &w, 4096),
            Err(DecodeError::UngrantedTool { .. })
        ));
        // Same name, wrong version ⇒ also ungranted (exact match, SN-8).
        let env2 = br#"{"tool_call":{"name":"mcp-echo","version":"2","args":{}}}"#;
        assert!(matches!(
            parse_tool_call(env2, &w, 4096),
            Err(DecodeError::UngrantedTool { .. })
        ));
    }

    #[test]
    fn oversize_args_are_refused() {
        let w = warrant_granting(Some(("mcp-echo", "1")));
        let big = "x".repeat(100);
        let env = format!(
            r#"{{"tool_call":{{"name":"mcp-echo","version":"1","args":{{"q":"{big}"}}}}}}"#
        );
        assert!(matches!(
            parse_tool_call(env.as_bytes(), &w, 8),
            Err(DecodeError::Oversize { .. })
        ));
    }

    #[test]
    fn non_utf8_is_a_normal_completion_not_a_panic() {
        let w = warrant_granting(Some(("mcp-echo", "1")));
        assert_eq!(parse_tool_call(&[0xff, 0xfe, 0x00], &w, 4096), Ok(None));
    }

    #[test]
    fn args_bytes_are_byte_identical_to_the_envelope_substring() {
        // PR-2d-1 pin: the decoded args are the EXACT bytes of the envelope's
        // args object — no re-serialization, no normalization (RawValue carry).
        let w = warrant_granting(Some(("mcp-echo", "1")));
        let args_src = r#"{"q":"x","n":  7,"nested":{"a":[1,2,3]}}"#;
        let env =
            format!(r#"{{"tool_call":{{"name":"mcp-echo","version":"1","args":{args_src}}}}}"#);
        let call = parse_tool_call(env.as_bytes(), &w, 4096)
            .unwrap()
            .expect("a call");
        assert_eq!(call.args_bytes, args_src.as_bytes().to_vec());
    }
}
