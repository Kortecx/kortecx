//! Minimal JSON-RPC 2.0 framing for the MCP methods the adapter speaks.
//!
//! Deliberately tiny: a small set of request shapes (`tools/call` for the
//! single-shot capability; `initialize` + `tools/list` for the PR-6b-1 stateful
//! gateway session) and a single response shape (a fixed struct, never a dynamic
//! `Value`). Tool arguments + the server's result are carried verbatim as
//! [`serde_json::value::RawValue`] — opaque bytes the adapter never interprets,
//! so no float/`Value` decoding ever reaches the effect path.

use serde::Serialize;
use serde_json::value::RawValue;

/// The JSON-RPC method for a single tool invocation.
pub(crate) const METHOD_TOOLS_CALL: &str = "tools/call";

/// PR-6b-1: the MCP lifecycle handshake a stateful server expects before any
/// `tools/*` call (the single-shot [`crate::StdioTransport`] echo path is
/// handshake-free and never sends it).
pub(crate) const METHOD_INITIALIZE: &str = "initialize";

/// PR-6b-1: the MCP discovery method — enumerate a server's tools.
pub(crate) const METHOD_TOOLS_LIST: &str = "tools/list";

/// The MCP protocol revision this client advertises in `initialize`. A server
/// MAY negotiate a different supported revision in its response; the gateway
/// treats `initialize` as a liveness/handshake step and does not pin the reply.
pub(crate) const MCP_PROTOCOL_VERSION: &str = "2025-06-18";

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

/// PR-6b-1: an outbound JSON-RPC 2.0 `initialize` request — the MCP lifecycle
/// handshake. `clientInfo` is a fixed identity; `capabilities` is intentionally
/// empty (this client consumes `tools/*` only — no sampling/roots offered back).
#[derive(Debug, Serialize)]
pub(crate) struct InitializeRequest {
    pub(crate) jsonrpc: &'static str,
    pub(crate) id: u64,
    pub(crate) method: &'static str,
    pub(crate) params: InitializeParams,
}

/// `params` for [`InitializeRequest`].
#[derive(Debug, Serialize)]
pub(crate) struct InitializeParams {
    #[serde(rename = "protocolVersion")]
    pub(crate) protocol_version: &'static str,
    pub(crate) capabilities: EmptyCapabilities,
    #[serde(rename = "clientInfo")]
    pub(crate) client_info: ClientInfo,
}

/// An empty `capabilities` object (`{}`) — this client offers no server-callable
/// capabilities back, it only consumes `tools/*`.
#[derive(Debug, Serialize)]
pub(crate) struct EmptyCapabilities {}

/// The client identity sent in `initialize.params.clientInfo`.
#[derive(Debug, Serialize)]
pub(crate) struct ClientInfo {
    pub(crate) name: &'static str,
    pub(crate) version: &'static str,
}

impl InitializeRequest {
    /// Build the fixed `initialize` request this client sends on session open.
    pub(crate) fn new(id: u64) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            method: METHOD_INITIALIZE,
            params: InitializeParams {
                protocol_version: MCP_PROTOCOL_VERSION,
                capabilities: EmptyCapabilities {},
                client_info: ClientInfo {
                    name: "kortecx-mcp-gateway",
                    version: env!("CARGO_PKG_VERSION"),
                },
            },
        }
    }
}

/// PR-6b-1: an outbound JSON-RPC 2.0 `tools/list` request. `params` is omitted
/// (no pagination cursor): the gateway fetches the full tool set in one call and
/// bounds the response by the per-call size cap.
#[derive(Debug, Serialize)]
pub(crate) struct ToolsListRequest {
    pub(crate) jsonrpc: &'static str,
    pub(crate) id: u64,
    pub(crate) method: &'static str,
}

impl ToolsListRequest {
    /// Build a `tools/list` request.
    pub(crate) fn new(id: u64) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            method: METHOD_TOOLS_LIST,
        }
    }
}

/// Frame an `initialize` request as JSON-RPC bytes (no trailing newline).
pub(crate) fn frame_initialize(id: u64) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(&InitializeRequest::new(id))
}

/// Frame a `tools/list` request as JSON-RPC bytes (no trailing newline).
pub(crate) fn frame_tools_list(id: u64) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(&ToolsListRequest::new(id))
}

/// Frame a `tools/call` request for `remote_name` carrying `args_bytes` verbatim
/// (an empty slice means "no arguments" → `{}`). The args are parsed only as a
/// borrowed [`RawValue`] (a structural token scan, never a dynamic `Value`).
pub(crate) fn frame_tools_call(
    id: u64,
    remote_name: &str,
    args_bytes: &[u8],
) -> Result<Vec<u8>, serde_json::Error> {
    let args: &RawValue = if args_bytes.is_empty() {
        serde_json::from_str("{}")?
    } else {
        serde_json::from_slice(args_bytes)?
    };
    serde_json::to_vec(&ToolsCallRequest::new(id, remote_name, args))
}
