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
    // PR-6b-3: the advertised protocol version is selectable via
    // `--protocol-version <v>` (default `2025-06-18`) so an interop test can dial
    // an OLD vs a NEW (`2026-07-28`) server. A per-process `tools/call` counter is
    // echoed back so a test can PROVE stateful session reuse (the same process
    // serves call 1 then call 2) vs stateless (a fresh process resets to 1).
    let version = protocol_version_from_args();
    // T-CONN: `--tools-list-error` makes `initialize` SUCCEED but `tools/list`
    // return a JSON-RPC error — a server that handshakes but can't list tools.
    // Pre-fix, `test` (initialize-only) called this reachable while `register`
    // (initialize + tools/list) called it unreachable.
    let tools_list_error = std::env::args().any(|a| a == "--tools-list-error");
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let mut call_count: u64 = 0;
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let req = serde_json::from_str::<Req>(&line).ok();
        // Before a tools/call reply, emit an UNSOLICITED JSON-RPC notification
        // (no `id`) — a spec-compliant server may interleave logging/progress on
        // stdout. The client's stateful session MUST skip it and correlate the
        // following reply by `id` (PR-6b-1 review finding #1).
        if req.as_ref().map(|r| r.method.as_str()) == Some("tools/call")
            && (writeln!(
                out,
                r#"{{"jsonrpc":"2.0","method":"notifications/message","params":{{"level":"info","data":"working"}}}}"#
            )
            .is_err()
                || out.flush().is_err())
        {
            break;
        }
        let reply = match req {
            Some(req) => {
                if req.method == "tools/call" {
                    call_count += 1;
                }
                handle(&req, &version, call_count, tools_list_error)
            }
            None => r#"{"jsonrpc":"2.0","id":0,"error":{"code":-32700,"message":"parse error"}}"#
                .to_string(),
        };
        if writeln!(out, "{reply}").is_err() || out.flush().is_err() {
            break;
        }
    }
}

/// Read `--protocol-version <v>` from argv (default `2025-06-18`).
fn protocol_version_from_args() -> String {
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--protocol-version" {
            if let Some(v) = args.next() {
                return v;
            }
        }
    }
    "2025-06-18".to_string()
}

fn handle(req: &Req, version: &str, call_count: u64, tools_list_error: bool) -> String {
    let id = req.id;
    match req.method.as_str() {
        "initialize" => format!(
            r#"{{"jsonrpc":"2.0","id":{id},"result":{{"protocolVersion":"{version}","capabilities":{{}},"serverInfo":{{"name":"kx-test","version":"1"}}}}}}"#
        ),
        // T-CONN: a server up + handshaking but failing tools/list.
        "tools/list" if tools_list_error => format!(
            r#"{{"jsonrpc":"2.0","id":{id},"error":{{"code":-32603,"message":"tools/list unavailable"}}}}"#
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
            format!(
                r#"{{"jsonrpc":"2.0","id":{id},"result":{{"echoed":{args},"call":{call_count}}}}}"#
            )
        }
        other => format!(
            r#"{{"jsonrpc":"2.0","id":{id},"error":{{"code":-32601,"message":"no such method: {other}"}}}}"#
        ),
    }
}
