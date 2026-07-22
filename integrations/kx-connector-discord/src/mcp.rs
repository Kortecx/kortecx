// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! The MCP JSON-RPC 2.0 protocol surface: `initialize` -> `tools/list` ->
//! `tools/call`, newline-delimited over stdio. Every reply is fail-closed — an
//! unparseable line or unknown method yields a JSON-RPC error, never a fabricated
//! success.

use serde::Deserialize;
use serde_json::value::RawValue;

use crate::discord::DiscordClient;
use crate::tools;

/// The MCP protocol version advertised at `initialize`.
pub const PROTOCOL_VERSION: &str = "2025-06-18";

/// One JSON-RPC 2.0 request line. `jsonrpc` is ignored (unknown fields dropped).
#[derive(Deserialize)]
pub struct Req {
    /// The request id, echoed on the reply. Absent -> `0`.
    #[serde(default)]
    pub id: u64,
    /// The method name (`initialize` / `tools/list` / `tools/call`).
    pub method: String,
    /// Method parameters (present for `tools/call`).
    #[serde(default)]
    pub params: Option<Params>,
}

/// The `tools/call` parameters: the tool `name` and its opaque `arguments` object.
#[derive(Deserialize)]
pub struct Params {
    /// The tool name to invoke.
    #[serde(default)]
    pub name: Option<String>,
    /// The tool arguments, kept as raw JSON (decoded fail-closed per tool).
    #[serde(default)]
    pub arguments: Option<Box<RawValue>>,
}

/// Parse one newline-delimited JSON-RPC request and produce its reply line.
///
/// Fail-closed: an unparseable line yields a JSON-RPC parse error (`-32700`).
#[must_use]
pub fn handle_line(line: &str, client: &DiscordClient) -> String {
    match serde_json::from_str::<Req>(line) {
        Ok(req) => handle(&req, client),
        Err(_) => error(0, -32700, "parse error"),
    }
}

fn handle(req: &Req, client: &DiscordClient) -> String {
    let id = req.id;
    match req.method.as_str() {
        "initialize" => format!(
            r#"{{"jsonrpc":"2.0","id":{id},"result":{{"protocolVersion":"{PROTOCOL_VERSION}","capabilities":{{"tools":{{}}}},"serverInfo":{{"name":"kx-connector-discord","version":"1"}}}}}}"#
        ),
        "tools/list" => format!(
            r#"{{"jsonrpc":"2.0","id":{id},"result":{{"tools":{}}}}}"#,
            tools::catalog_json()
        ),
        "tools/call" => tools::call(req, client),
        other => error(id, -32601, &format!("no such method: {other}")),
    }
}

/// Build a fail-closed JSON-RPC error reply. Never carries a credential value.
#[must_use]
pub fn error(id: u64, code: i64, message: &str) -> String {
    // Encode the message as a JSON string so quotes / control chars cannot break the frame.
    let msg = serde_json::to_string(message).unwrap_or_else(|_| "\"error\"".to_string());
    format!(r#"{{"jsonrpc":"2.0","id":{id},"error":{{"code":{code},"message":{msg}}}}}"#)
}

/// Build a JSON-RPC success reply wrapping a `result` object given as raw JSON text.
#[must_use]
pub fn result(id: u64, result_json: &str) -> String {
    format!(r#"{{"jsonrpc":"2.0","id":{id},"result":{result_json}}}"#)
}

#[cfg(test)]
mod tests {
    use super::{handle_line, PROTOCOL_VERSION};
    use crate::discord::DiscordClient;

    fn fake() -> DiscordClient {
        DiscordClient::fake()
    }

    #[test]
    fn initialize_advertises_protocol_and_name() {
        let reply = handle_line(r#"{"id":1,"method":"initialize"}"#, &fake());
        assert!(reply.contains(PROTOCOL_VERSION));
        assert!(reply.contains("kx-connector-discord"));
        assert!(reply.contains(r#""id":1"#));
    }

    #[test]
    fn tools_list_exposes_the_three_tools() {
        let reply = handle_line(r#"{"id":2,"method":"tools/list"}"#, &fake());
        for tool in ["send_message", "read_channel", "list_channels"] {
            assert!(reply.contains(tool), "tools/list missing {tool}: {reply}");
        }
    }

    #[test]
    fn unparseable_line_is_a_parse_error() {
        let reply = handle_line("not json", &fake());
        assert!(reply.contains("-32700"));
        assert!(reply.contains("error"));
    }

    #[test]
    fn unknown_method_is_fail_closed() {
        let reply = handle_line(r#"{"id":3,"method":"frobnicate"}"#, &fake());
        assert!(reply.contains("-32601"));
    }
}
