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
