//! `kx-mcp-echo` — the BUNDLED deterministic stdio MCP tool (PR-2d-2,
//! react-tools-live): a newline-delimited JSON-RPC `tools/call` responder that
//! echoes the request's `arguments` verbatim inside the result.
//!
//! This is the demo "Act" tool the live `kx serve` `ReAct` loop fires through
//! [`kx_mcp::StdioTransport`] under the server-built `mcp-echo@1` grant. It is
//! deliberately the SMALLEST possible effect surface:
//! - **Deterministic in the request args** (identical args ⇒ identical reply
//!   bytes ⇒ content-addressed dedup on a crash-recovery re-dispatch — the
//!   exactly-once contract at the world boundary, D58 §7).
//! - **No egress** (`net_scope_required: None` in its `ToolDef`) — it reads one
//!   line from stdin and writes one line to stdout; SSRF/egress vetting is N/A.
//! - **No modes, no env-selected behaviour** (unlike the chaos-mode
//!   `kx-mcp-mock-stdio` TEST support bin this was distilled from): a
//!   production-bundled tool must not carry failure-injection switches.
//! - **Never echoes its environment**, so an injected credential (D81) cannot
//!   reach any sink through it.

use std::io::{BufRead, Write};

use serde::Deserialize;
use serde_json::value::RawValue;

#[derive(Deserialize)]
struct Req {
    params: Params,
}

#[derive(Deserialize)]
struct Params {
    #[serde(default)]
    arguments: Option<Box<RawValue>>,
}

fn main() {
    // Read exactly one JSON-RPC request line from stdin (the StdioTransport
    // round-trip contract: one request, one reply, exit).
    let mut line = String::new();
    let stdin = std::io::stdin();
    let _ = stdin.lock().read_line(&mut line);

    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let _ = writeln!(out, "{}", echo(&line));
    let _ = out.flush();
}

/// Echo the request's `arguments` verbatim inside a JSON-RPC result. A request
/// that does not decode is answered with a JSON-RPC parse error (fail-closed:
/// the caller's `decode_tool_result` surfaces it as a capability failure).
fn echo(request_line: &str) -> String {
    match serde_json::from_str::<Req>(request_line) {
        Ok(req) => {
            let args = req
                .params
                .arguments
                .map_or_else(|| "{}".to_string(), |a| a.get().to_string());
            format!(r#"{{"jsonrpc":"2.0","id":1,"result":{{"echoed":{args}}}}}"#)
        }
        Err(_) => r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32700,"message":"parse error"}}"#
            .to_string(),
    }
}
