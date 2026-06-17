//! The fail-closed inbound decoder (IMP-5 / IMP-16).
//!
//! [`decode_tool_result`] is the single point where untrusted MCP server bytes
//! cross into the runtime. It is **total** (never panics on arbitrary or truncated
//! input), **size-capped** (rejects before parsing a huge body), and **strict**
//! (accepts only a well-formed JSON-RPC 2.0 `tools/call` result; anything else is
//! refused). It never deserializes into a dynamic `serde_json::Value` — the result
//! object is carried verbatim as bytes, so no float/`Value` interpretation occurs.

use serde::Deserialize;
use serde_json::value::RawValue;

use crate::errors::DecodeError;

/// Default per-call response-size cap (IMP-16) when a warrant supplies no positive
/// ceiling: 1 MiB. The capability prefers a warrant-derived cap; this is the floor.
pub const MAX_TOOL_RESULT_BYTES_DEFAULT: usize = 1 << 20;

/// A JSON-RPC 2.0 response envelope, decoded into a fixed shape.
///
/// `jsonrpc` / `id` are ignored (unknown fields are dropped by serde). Exactly one
/// of `result` / `error` is expected; `result` is kept verbatim as a [`RawValue`]
/// so its contents are never interpreted by the adapter.
#[derive(Debug, Deserialize)]
struct JsonRpcResponse {
    #[serde(default)]
    result: Option<Box<RawValue>>,
    #[serde(default)]
    error: Option<JsonRpcError>,
}

/// A JSON-RPC 2.0 `error` object.
#[derive(Debug, Deserialize)]
struct JsonRpcError {
    code: i64,
    #[serde(default)]
    message: String,
}

/// Decode an MCP `tools/call` JSON-RPC response into the verbatim bytes of its
/// `result` object — fail-closed.
///
/// # Errors
///
/// - [`DecodeError::Oversize`] if `bytes.len() > max_bytes` (checked *before*
///   parsing — a hostile server cannot force a huge allocation).
/// - [`DecodeError::Malformed`] if the bytes are not JSON, are truncated, are not a
///   JSON object, or carry neither a `result` nor an `error` member.
/// - [`DecodeError::ProtocolError`] if the server returned a JSON-RPC `error`.
///
/// On success the returned `Vec<u8>` is the canonical JSON of the `result` object
/// (opaque to the adapter; staged verbatim as the effect's result bytes).
pub fn decode_tool_result(bytes: &[u8], max_bytes: usize) -> Result<Vec<u8>, DecodeError> {
    if bytes.len() > max_bytes {
        return Err(DecodeError::Oversize {
            got: bytes.len(),
            max: max_bytes,
        });
    }

    // serde_json is total over arbitrary bytes (truncation, non-JSON, non-object,
    // and over-deep nesting all return Err, never panic) — the fail-closed contract.
    let resp: JsonRpcResponse =
        serde_json::from_slice(bytes).map_err(|e| DecodeError::Malformed {
            // The diagnostic is the parser's structural message, never the payload.
            diagnostic: e.to_string(),
        })?;

    // A server-side protocol error takes precedence over an absent result.
    if let Some(err) = resp.error {
        return Err(DecodeError::ProtocolError {
            code: err.code,
            message: err.message,
        });
    }

    match resp.result {
        Some(raw) => Ok(raw.get().as_bytes().to_vec()),
        None => Err(DecodeError::Malformed {
            diagnostic: "JSON-RPC response carried neither `result` nor `error`".to_string(),
        }),
    }
}

/// PR-6b-1: a single remote tool declaration from an MCP `tools/list` response.
///
/// `input_schema_json` is the verbatim bytes of the tool's `inputSchema` object
/// (a JSON Schema), or empty when the server omitted it. It is carried opaque —
/// the gateway maps the subset it understands into the typed registry schema and
/// otherwise leaves remote-side validation to the server (fail-closed: an absent
/// or unmappable schema yields no client-side arg gate, never a fabricated one).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteToolDecl {
    /// The tool's remote name (the `name` passed to `tools/call`).
    pub name: String,
    /// The tool's human description (may be empty).
    pub description: String,
    /// Verbatim bytes of the tool's `inputSchema` object, or empty when absent.
    pub input_schema_json: Vec<u8>,
}

/// The inner `result` of an MCP `tools/list` response: `{ "tools": [ … ] }`.
#[derive(Deserialize)]
struct ToolsListResult {
    #[serde(default)]
    tools: Vec<RemoteToolWire>,
}

/// A `tools/list` tool entry, decoded into a fixed shape. `inputSchema` is kept
/// verbatim as a [`RawValue`] (never interpreted into a dynamic `Value`/floats).
#[derive(Deserialize)]
struct RemoteToolWire {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default, rename = "inputSchema")]
    input_schema: Option<Box<RawValue>>,
}

/// Decode an MCP `tools/list` JSON-RPC response into the remote tool declarations
/// — fail-closed, reusing [`decode_tool_result`]'s envelope/error/size handling.
///
/// # Errors
///
/// - [`DecodeError::Oversize`] / [`DecodeError::Malformed`] / [`DecodeError::ProtocolError`]
///   exactly as [`decode_tool_result`] (the envelope is shared), plus
///   [`DecodeError::Malformed`] if the `result` is not a `{ "tools": [...] }`
///   object. A server returning a `result` with no `tools` member yields an
///   EMPTY list (a server may legitimately expose zero tools).
pub fn decode_tools_list(
    bytes: &[u8],
    max_bytes: usize,
) -> Result<Vec<RemoteToolDecl>, DecodeError> {
    // Reuse the envelope decode: size-cap before parse, JSON-RPC error precedence,
    // and verbatim extraction of the `result` object's canonical bytes.
    let result = decode_tool_result(bytes, max_bytes)?;
    let parsed: ToolsListResult =
        serde_json::from_slice(&result).map_err(|e| DecodeError::Malformed {
            diagnostic: e.to_string(),
        })?;
    Ok(parsed
        .tools
        .into_iter()
        .map(|t| RemoteToolDecl {
            name: t.name,
            description: t.description,
            input_schema_json: t
                .input_schema
                .map(|s| s.get().as_bytes().to_vec())
                .unwrap_or_default(),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_a_well_formed_result_verbatim() {
        let body =
            br#"{"jsonrpc":"2.0","id":1,"result":{"content":[{"type":"text","text":"ok"}]}}"#;
        let out = decode_tool_result(body, 4096).unwrap();
        // The result object is returned verbatim (whitespace-free canonical form).
        assert_eq!(out, br#"{"content":[{"type":"text","text":"ok"}]}"#);
    }

    #[test]
    fn rejects_oversize_before_parsing() {
        let body = br#"{"jsonrpc":"2.0","id":1,"result":{}}"#;
        let err = decode_tool_result(body, 4).unwrap_err();
        assert!(matches!(err, DecodeError::Oversize { max: 4, .. }));
    }

    #[test]
    fn rejects_truncated_json() {
        let body = br#"{"jsonrpc":"2.0","id":1,"result":{"content":"#;
        assert!(matches!(
            decode_tool_result(body, 4096),
            Err(DecodeError::Malformed { .. })
        ));
    }

    #[test]
    fn rejects_non_object_json() {
        for body in [
            &b"[1,2,3]"[..],
            &b"\"a string\""[..],
            &b"42"[..],
            &b"null"[..],
        ] {
            assert!(
                matches!(
                    decode_tool_result(body, 4096),
                    Err(DecodeError::Malformed { .. })
                ),
                "non-object JSON must be refused: {:?}",
                std::str::from_utf8(body)
            );
        }
    }

    #[test]
    fn surfaces_a_server_protocol_error() {
        let body = br#"{"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"no such tool"}}"#;
        match decode_tool_result(body, 4096) {
            Err(DecodeError::ProtocolError { code, message }) => {
                assert_eq!(code, -32601);
                assert_eq!(message, "no such tool");
            }
            other => panic!("expected ProtocolError, got {other:?}"),
        }
    }

    #[test]
    fn rejects_response_with_neither_result_nor_error() {
        let body = br#"{"jsonrpc":"2.0","id":1}"#;
        assert!(matches!(
            decode_tool_result(body, 4096),
            Err(DecodeError::Malformed { .. })
        ));
    }

    #[test]
    fn empty_input_is_malformed_not_a_panic() {
        assert!(matches!(
            decode_tool_result(b"", 4096),
            Err(DecodeError::Malformed { .. })
        ));
    }

    #[test]
    fn deeply_nested_result_does_not_panic() {
        // The `result` object is carried verbatim as a RawValue (a flat token scan,
        // not recursive descent), so deep nesting INSIDE it is accepted without a
        // stack overflow. This locks the panic-safety property against a future
        // refactor that might swap RawValue for a recursive `Value`.
        let depth = 50_000;
        let nested = format!("[{}{}", "[".repeat(depth), "]".repeat(depth));
        let body = format!(r#"{{"jsonrpc":"2.0","id":1,"result":{{"d":{nested}}}}}"#);
        // Cap above the body size so we exercise the parser, not the size guard.
        let cap = body.len() + 16;
        let _ = decode_tool_result(body.as_bytes(), cap); // must not panic
    }

    #[test]
    fn decode_tools_list_extracts_decls_verbatim() {
        let body = br#"{"jsonrpc":"2.0","id":1,"result":{"tools":[
            {"name":"search","description":"web search","inputSchema":{"type":"object","properties":{"q":{"type":"string"}},"required":["q"]}},
            {"name":"noschema"}
        ]}}"#;
        let decls = decode_tools_list(body, 4096).unwrap();
        assert_eq!(decls.len(), 2);
        assert_eq!(decls[0].name, "search");
        assert_eq!(decls[0].description, "web search");
        assert!(!decls[0].input_schema_json.is_empty());
        assert_eq!(decls[1].name, "noschema");
        assert!(decls[1].input_schema_json.is_empty());
    }

    #[test]
    fn decode_tools_list_empty_when_no_tools_member() {
        // A server may legitimately expose zero tools.
        let body = br#"{"jsonrpc":"2.0","id":1,"result":{}}"#;
        assert!(decode_tools_list(body, 4096).unwrap().is_empty());
    }

    #[test]
    fn decode_tools_list_surfaces_protocol_error() {
        let body = br#"{"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"no tools/list"}}"#;
        assert!(matches!(
            decode_tools_list(body, 4096),
            Err(DecodeError::ProtocolError { code: -32601, .. })
        ));
    }

    #[test]
    fn decode_tools_list_rejects_oversize_before_parse() {
        let body = br#"{"jsonrpc":"2.0","id":1,"result":{"tools":[]}}"#;
        assert!(matches!(
            decode_tools_list(body, 4),
            Err(DecodeError::Oversize { max: 4, .. })
        ));
    }

    #[test]
    fn deeply_nested_top_level_array_does_not_panic() {
        // A deeply-nested top-level array is NOT recursively descended: serde_json
        // deserializes the struct from a sequence and captures the nested element
        // verbatim as a RawValue (flat scan). The point is panic-safety — the call
        // returns (Ok or Err) and never overflows the stack.
        let depth = 50_000;
        let body = format!("{}{}", "[".repeat(depth), "]".repeat(depth));
        let _ = decode_tool_result(body.as_bytes(), body.len() + 16); // must not panic
    }
}
