//! Minimal JSON-RPC 2.0 framing for the MCP `tools/call` method.
//!
//! Deliberately tiny: a single request shape (the outbound `tools/call`) and a
//! single response shape (a fixed struct, never a dynamic `Value`). Tool
//! arguments + the server's result are carried verbatim as
//! [`serde_json::value::RawValue`] — opaque bytes the adapter never interprets,
//! so no float/`Value` decoding ever reaches the effect path.

use serde::Serialize;
use serde_json::value::RawValue;

/// The JSON-RPC method the adapter speaks. M5.2a is single-shot `tools/call`; the
/// `initialize`/`initialized` handshake a stateful MCP server expects is a
/// documented forward seam (M5.2b — the test server is handshake-free).
pub(crate) const METHOD_TOOLS_CALL: &str = "tools/call";

/// An outbound JSON-RPC 2.0 `tools/call` request.
///
/// `params.arguments` is the model-proposed args object, carried verbatim from the
/// validated `EffectRequest.payload` as a borrowed [`RawValue`] (never re-parsed
/// into a dynamic value).
#[derive(Debug, Serialize)]
pub(crate) struct ToolsCallRequest<'a> {
    pub(crate) jsonrpc: &'static str,
    pub(crate) id: u64,
    pub(crate) method: &'static str,
    pub(crate) params: ToolsCallParams<'a>,
}

/// `params` for [`ToolsCallRequest`]: the remote tool name + its arguments.
#[derive(Debug, Serialize)]
pub(crate) struct ToolsCallParams<'a> {
    pub(crate) name: &'a str,
    pub(crate) arguments: &'a RawValue,
}

impl<'a> ToolsCallRequest<'a> {
    /// Build a `tools/call` request for `remote_name` with verbatim `arguments`.
    pub(crate) fn new(id: u64, remote_name: &'a str, arguments: &'a RawValue) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            method: METHOD_TOOLS_CALL,
            params: ToolsCallParams {
                name: remote_name,
                arguments,
            },
        }
    }
}
