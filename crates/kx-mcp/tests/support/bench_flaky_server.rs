//! Test-support MCP server for the benchmark's `failure` family: a stdio tool whose
//! failure MODE is chosen at registration, so one binary can be registered several times
//! as several differently-broken tools.
//!
//! Separate from `mock_stdio_server` on purpose. That one exists to exercise the
//! transport from `kx-mcp`'s own tests and takes its mode from the process ENVIRONMENT,
//! which cannot vary per registration — a serve that spawns it four times would get four
//! identical tools. This one takes its mode from ARGV, which `RegisterMcpServer` carries
//! per connection, and it answers `tools/list` so it can be DISCOVERED like any real
//! connector rather than hand-wired.
//!
//! Modes (argv[1], default `healthy`):
//! - `healthy`   — echo the arguments back. The control: same bin, same path, working.
//! - `error`     — a JSON-RPC `error` object. The call reached the tool and it refused.
//! - `malformed` — truncated JSON. The call SUCCEEDED at the transport and the payload is
//!                 unusable, which is what a half-written response looks like.
//! - `slow`      — sleep `argv[2]` ms (default 600_000) before replying, to be cut off by
//!                 the operator's per-Mote deadline.
//!
//! Deliberately NOT a bundled production tool: a shipped tool must not carry
//! failure-injection switches, which is why this lives under `tests/support` and is built
//! only alongside the test suite.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::io::{BufRead, Write};

fn main() {
    let mode = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "healthy".to_string());
    let sleep_ms: u64 = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(600_000);

    // One request line in, one reply line out, exit — the stdio transport's contract.
    let mut line = String::new();
    let stdin = std::io::stdin();
    let _ = stdin.lock().read_line(&mut line);

    let request: serde_json::Value = serde_json::from_str(&line).unwrap_or(serde_json::Value::Null);
    let id = request.get("id").cloned().unwrap_or(serde_json::json!(1));
    let method = request.get("method").and_then(|m| m.as_str()).unwrap_or("");

    let reply = match method {
        // Discovery must work in EVERY mode, including the broken ones: a tool that
        // cannot be discovered is a tool the model never sees, and the failure families
        // would then measure a missing grant instead of a failing call.
        "tools/list" => serde_json::json!({
            "jsonrpc": "2.0", "id": id,
            "result": {"tools": [{
                "name": "probe",
                "description": "Read the incident status.",
                "inputSchema": {
                    "type": "object",
                    "properties": {"input": {"type": "string"}},
                    "required": []
                }
            }]}
        })
        .to_string(),
        "initialize" => serde_json::json!({
            "jsonrpc": "2.0", "id": id,
            "result": {"protocolVersion": "2025-06-18", "capabilities": {},
                       "serverInfo": {"name": "kx-bench-flaky", "version": "1"}}
        })
        .to_string(),
        _ => match mode.as_str() {
            "error" => serde_json::json!({
                "jsonrpc": "2.0", "id": id,
                "error": {"code": -32000, "message": "incident feed unavailable"}
            })
            .to_string(),
            "malformed" => r#"{"jsonrpc":"2.0","id":1,"result":{"status":"#.to_string(),
            "slow" => {
                std::thread::sleep(std::time::Duration::from_millis(sleep_ms));
                echoed(&id, &request)
            }
            _ => echoed(&id, &request),
        },
    };

    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let _ = writeln!(out, "{reply}");
    let _ = out.flush();
}

/// Echo the call's `arguments` back inside a result — deterministic in the request, so a
/// crash-recovery re-dispatch content-addresses to the same bytes.
fn echoed(id: &serde_json::Value, request: &serde_json::Value) -> String {
    let args = request
        .get("params")
        .and_then(|p| p.get("arguments"))
        .cloned()
        .unwrap_or(serde_json::json!({}));
    serde_json::json!({"jsonrpc": "2.0", "id": id, "result": {"echoed": args}}).to_string()
}
