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

/// Gemma-4's NATIVE tool-call open delimiter (`<|tool_call>call:NAME{ARGS}<tool_call|>`).
const GEMMA_TOOL_OPEN: &str = "<|tool_call>";
/// The optional `call:` marker after the open delimiter (observed: `call:fs_list{}`).
const GEMMA_CALL_MARKER: &str = "call:";

/// A model-NATIVE (non-envelope) call shape, post-extraction: the raw tool name
/// and the verbatim args-object bytes. The version is resolved against the grant
/// set by the caller (Gemma emits no version).
struct NativeCall<'a> {
    raw_name: &'a str,
    args: &'a str,
}

/// Extract a Gemma-4 native `<|tool_call>call:NAME{ARGS}<tool_call|>` call from the
/// (reasoning-stripped) text, or `None` if the text is not this shape. NAME is the
/// run after the (optional) `call:` marker up to the FIRST `{` — a DEFINED
/// NAME/ARGS boundary, exactly like the markdown fence (NEVER a mid-string `{`
/// search, so the SN-8 injection boundary is unchanged: only bytes the model
/// fenced inside `<|tool_call>…` are promoted to a call). The `<tool_call|>` close
/// is optional (truncation-tolerant) — `balanced_object` bounds the args object so
/// trailing prose / the close delim can never leak in. Total + panic-free.
fn extract_gemma_native(text: &str) -> Option<NativeCall<'_>> {
    let after_open = text.trim_start().strip_prefix(GEMMA_TOOL_OPEN)?;
    let after_marker = after_open
        .trim_start()
        .strip_prefix(GEMMA_CALL_MARKER)
        .unwrap_or_else(|| after_open.trim_start());
    let brace = after_marker.find('{')?;
    let raw_name = after_marker[..brace].trim();
    if raw_name.is_empty() {
        return None;
    }
    let args = balanced_object(&after_marker[brace..])?;
    Some(NativeCall { raw_name, args })
}

/// Return the prefix of `s` (which MUST start with `{`) spanning the first
/// brace-balanced JSON object, ignoring braces inside double-quoted strings
/// (with `\"` escapes). `None` if unbalanced or past `MAX_DEPTH`. Total +
/// panic-free; the depth bound rejects pathological nesting cheaply (serde
/// re-parses for shape downstream).
fn balanced_object(s: &str) -> Option<&str> {
    const MAX_DEPTH: usize = 64;
    let bytes = s.as_bytes();
    if bytes.first() != Some(&b'{') {
        return None;
    }
    let mut depth = 0usize;
    let mut in_str = false;
    let mut escaped = false;
    for (i, &b) in bytes.iter().enumerate() {
        if in_str {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_str = false;
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'{' => {
                depth += 1;
                if depth > MAX_DEPTH {
                    return None;
                }
            }
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&s[..=i]);
                }
            }
            _ => {}
        }
    }
    None
}

/// Resolve a model-emitted (often separator-variant, version-less) tool name to a
/// GRANTED `(ToolName, ToolVersion)`, SN-8-safe. Normalization is SEPARATOR-ONLY
/// (`_`→`-`, matching how Gemma renders `fs-list` as `fs_list`) — never an
/// arbitrary remap. Resolution = the UNIQUE granted tool whose name, after the
/// SAME separator-normalization on BOTH sides, equals the model's. Ambiguity (two
/// granted tools collapsing to one normalized name) ⇒ `None` (fail-closed — no
/// guessing). The returned version is whatever the grant pins, so the downstream
/// `tool_grants.contains` check is exact by construction.
fn resolve_granted_name(raw_name: &str, warrant: &WarrantSpec) -> Option<ToolGrant> {
    fn norm(s: &str) -> String {
        s.replace('_', "-")
    }
    let target = norm(raw_name);
    let mut hit: Option<&ToolGrant> = None;
    for g in &warrant.tool_grants {
        if norm(&g.tool_id.0) == target {
            if hit.is_some() {
                return None; // ambiguous ⇒ fail-closed
            }
            hit = Some(g);
        }
    }
    hit.cloned()
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

    // (1a) Gemma-4 NATIVE shape: `<|tool_call>call:NAME{ARGS}<tool_call|>`. A SECOND
    //      DEFINED delimiter set (not a `{` search) — recognized BEFORE the JSON
    //      gate. Version-less + separator-variant names (`fs_list`) are resolved
    //      against the grant set, and the result is gated by the SAME exact
    //      `tool_grants` equality (SN-8). Anything not opening with this exact
    //      delimiter falls through to the JSON envelope path, byte-identical for
    //      every existing row (no current input begins with `<|tool_call>`).
    if let Some(native) = extract_gemma_native(trimmed) {
        let Some(grant) = resolve_granted_name(native.raw_name, warrant) else {
            // The model COMMITTED to a native call but named an unknown/ambiguous
            // tool ⇒ fail-closed (a bad name is a refusal, never silent prose).
            return Err(DecodeError::UngrantedTool {
                name: ToolName(native.raw_name.to_string()),
                version: ToolVersion(String::new()),
            });
        };
        let args_bytes = native.args.as_bytes().to_vec();
        if args_bytes.len() > max_args_bytes {
            return Err(DecodeError::Oversize {
                got: args_bytes.len(),
                max: max_args_bytes,
            });
        }
        return Ok(Some(ToolCall {
            name: grant.tool_id,
            version: grant.tool_version,
            args_bytes,
        }));
    }

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

    // ---- BUG-28: Gemma-4 native `<|tool_call>call:NAME{ARGS}<tool_call|>` arm ----

    #[test]
    fn gemma_native_call_is_decoded_name_normalized_version_resolved() {
        let w = warrant_granting(Some(("fs-list", "1")));
        // Gemma's native syntax: underscore name, no version, empty args.
        let env = b"<|tool_call>call:fs_list{}<tool_call|>";
        let call = parse_tool_call(env, &w, 4096)
            .unwrap()
            .expect("a native call");
        assert_eq!(call.name, ToolName("fs-list".into())); // `_`→`-` normalized
        assert_eq!(call.version, ToolVersion("1".into())); // resolved from the grant
        assert_eq!(call.args_bytes, b"{}".to_vec());
    }

    #[test]
    fn gemma_native_call_with_args_carries_bytes_verbatim() {
        let w = warrant_granting(Some(("fs-list", "1")));
        let env = br#"<|tool_call>call:fs_list{"path":"sub"}<tool_call|>"#;
        let call = parse_tool_call(env, &w, 4096)
            .unwrap()
            .expect("a native call");
        assert_eq!(call.args_bytes, br#"{"path":"sub"}"#.to_vec());
    }

    #[test]
    fn gemma_native_tolerates_missing_close_delim() {
        let w = warrant_granting(Some(("fs-list", "1")));
        // Truncated close delimiter — brace-balancing still bounds the args object.
        let env = br#"<|tool_call>call:fs_list{"path":"x"}"#;
        let call = parse_tool_call(env, &w, 4096)
            .unwrap()
            .expect("a native call");
        assert_eq!(call.args_bytes, br#"{"path":"x"}"#.to_vec());
    }

    #[test]
    fn gemma_native_after_channel_reasoning_is_decoded() {
        let w = warrant_granting(Some(("fs-list", "1")));
        // The `<|channel>` reasoning block is stripped first, then the native shape.
        let env = b"<|channel>thinking<channel|><|tool_call>call:fs_list{}<tool_call|>";
        let call = parse_tool_call(env, &w, 4096)
            .unwrap()
            .expect("a native call");
        assert_eq!(call.name, ToolName("fs-list".into()));
    }

    #[test]
    fn gemma_native_ungranted_tool_is_refused() {
        let w = warrant_granting(Some(("fs-list", "1")));
        let env = b"<|tool_call>call:rm_rf{}<tool_call|>";
        assert!(matches!(
            parse_tool_call(env, &w, 4096),
            Err(DecodeError::UngrantedTool { .. })
        ));
    }

    #[test]
    fn gemma_native_empty_grants_is_none() {
        let w = warrant_granting(None); // step (0) short-circuits before any parse
        let env = b"<|tool_call>call:fs_list{}<tool_call|>";
        assert_eq!(parse_tool_call(env, &w, 4096), Ok(None));
    }

    #[test]
    fn gemma_native_oversize_args_refused() {
        let w = warrant_granting(Some(("fs-list", "1")));
        let big = "x".repeat(100);
        let env = format!(r#"<|tool_call>call:fs_list{{"p":"{big}"}}<tool_call|>"#);
        assert!(matches!(
            parse_tool_call(env.as_bytes(), &w, 8),
            Err(DecodeError::Oversize { .. })
        ));
    }

    #[test]
    fn gemma_native_overdeep_args_falls_through_to_none() {
        let w = warrant_granting(Some(("fs-list", "1")));
        let deep = format!(
            "<|tool_call>call:fs_list{}{}",
            "{".repeat(80),
            "}".repeat(80)
        );
        // balanced_object returns None past MAX_DEPTH ⇒ not a native call ⇒ falls to
        // the JSON gate, which sees a non-`{` start ⇒ Ok(None) (fail-closed).
        assert_eq!(parse_tool_call(deep.as_bytes(), &w, 4096), Ok(None));
    }

    #[test]
    fn gemma_native_no_brace_is_not_a_call() {
        let w = warrant_granting(Some(("fs-list", "1")));
        // Open delim but no `{` ⇒ not extractable ⇒ falls through ⇒ Ok(None).
        let env = b"<|tool_call>call:fs_list no args here";
        assert_eq!(parse_tool_call(env, &w, 4096), Ok(None));
    }
}
