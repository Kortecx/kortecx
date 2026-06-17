//! A STATEFUL newline-delimited JSON-RPC stdio MCP test server.
//!
//! Unlike `kx-mcp`'s single-shot `kx-mcp-echo` (one line, then exit), this server
//! loops — handling the full `initialize → tools/list → tools/call` lifecycle over
//! ONE process — so the integration tests exercise the REAL [`kx_mcp`] session
//! seam + the gateway's dial/discover/register path. Test-support only.

use std::io::{BufRead, Write};

use serde::Deserialize;
use serde_json::value::RawValue;

#[derive(Deserialize)]
struct Req {
    #[serde(default)]
    id: u64,
    method: String,
    #[serde(default)]
    params: Option<Params>,
}

#[derive(Deserialize)]
struct Params {
    #[serde(default)]
    arguments: Option<Box<RawValue>>,
}

fn main() {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Req>(&line) {
            Ok(req) => handle(&req),
            Err(_) => r#"{"jsonrpc":"2.0","id":0,"error":{"code":-32700,"message":"parse error"}}"#
                .to_string(),
        };
        if writeln!(out, "{reply}").is_err() || out.flush().is_err() {
            break;
        }
    }
}

fn handle(req: &Req) -> String {
    let id = req.id;
    match req.method.as_str() {
        "initialize" => format!(
            r#"{{"jsonrpc":"2.0","id":{id},"result":{{"protocolVersion":"2025-06-18","capabilities":{{}},"serverInfo":{{"name":"kx-test","version":"1"}}}}}}"#
        ),
        "tools/list" => format!(
            r#"{{"jsonrpc":"2.0","id":{id},"result":{{"tools":[{{"name":"echo","description":"echo the args back","inputSchema":{{"type":"object","properties":{{"q":{{"type":"string"}}}},"required":["q"]}}}},{{"name":"ping","description":"liveness","inputSchema":{{"type":"object","properties":{{}}}}}}]}}}}"#
        ),
        "tools/call" => {
            let args = req
                .params
                .as_ref()
                .and_then(|p| p.arguments.as_ref())
                .map_or_else(|| "{}".to_string(), |a| a.get().to_string());
            format!(r#"{{"jsonrpc":"2.0","id":{id},"result":{{"echoed":{args}}}}}"#)
        }
        other => format!(
            r#"{{"jsonrpc":"2.0","id":{id},"error":{{"code":-32601,"message":"no such method: {other}"}}}}"#
        ),
    }
}
