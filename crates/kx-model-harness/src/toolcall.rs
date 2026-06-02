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

use kx_mote::{ToolName, ToolVersion};
use kx_warrant::{ToolGrant, WarrantSpec};
use serde::Deserialize;
use serde_json::value::RawValue;

/// A validated, warrant-granted tool call the model proposed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCall {
    /// The tool's name — guaranteed `∈ warrant.tool_grants`.
    pub name: ToolName,
    /// The tool's pinned version — matched exactly against the grant.
    pub version: ToolVersion,
    /// The proposed arguments, verbatim JSON bytes (size-capped, never executed).
    pub args_bytes: Vec<u8>,
}

/// Why a model output that *looked like* a tool call was refused. (A normal
/// completion is `Ok(None)`, not an error.)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    /// The output began as a JSON object but the tool-call envelope was malformed,
    /// truncated, or carried trailing garbage. Fail-closed — a half-formed proposal
    /// never fires an effect.
    Malformed {
        /// A short structural diagnostic (never the raw payload).
        diagnostic: String,
    },
    /// The model named a tool that is not in `warrant.tool_grants` (SN-8: the model
    /// cannot authorize an action the runtime did not grant).
    UngrantedTool {
        /// The proposed (ungranted) tool name.
        name: ToolName,
        /// The proposed version.
        version: ToolVersion,
    },
    /// The proposed arguments exceed the per-call size cap (IMP-16).
    Oversize {
        /// Observed args size in bytes.
        got: usize,
        /// The cap.
        max: usize,
    },
}

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

/// Decode a model-proposed tool call from raw model output, fail-closed.
///
/// Returns `Ok(None)` for a normal completion (prose, non-envelope JSON, or — the
/// important security default — *any* output when the warrant grants no tools).
/// Returns `Ok(Some(call))` for a well-formed, warrant-granted, size-bounded call.
/// Returns `Err` when the model committed to a tool-call envelope that is malformed,
/// names an ungranted tool, or overshoots the args cap.
///
/// Total + panic-free over arbitrary `bytes`.
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
    let trimmed = text.trim_start();
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
}
