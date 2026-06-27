// SPDX-License-Identifier: Apache-2.0
//! `kx-connector-example` — a minimal, COMPLETE reference MCP connector.
//!
//! This is the canonical "how to author a connector" artifact AND the deterministic
//! positive control for the [`kx_extension_sdk::conformance`] harness. Unlike the
//! single-shot `kx-mcp-echo` bundled in the runtime, this server loops over stdin,
//! handling the full `initialize → tools/list → tools/call` MCP lifecycle over ONE
//! process — exactly what `McpGateway::register_server` dials.
//!
//! It exposes two pure, deterministic tools (no egress, no clock, no randomness ⇒
//! content-addressed dedup on a crash-recovery re-dispatch, the exactly-once
//! contract at the world boundary):
//!   - `echo`    — return the call arguments verbatim.
//!   - `reverse` — return the `text` argument reversed (a real transform).
//!
//! Security discipline a connector MUST honor:
//!   - **Never echo the environment.** An injected credential (D81) reaches no reply,
//!     so it never lands in a journal/content/telemetry sink.
//!   - **Fail closed.** An unknown method / unparseable line yields a JSON-RPC error,
//!     which the runtime surfaces as a capability failure — never a fabricated success.
//!
//! Wire shape: newline-delimited JSON-RPC 2.0 over stdio. Each request is one line;
//! each reply is one line correlated by `id`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::print_stdout)]

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
    name: Option<String>,
    #[serde(default)]
    arguments: Option<Box<RawValue>>,
}

/// The MCP protocol version this reference connector advertises.
const PROTOCOL_VERSION: &str = "2025-06-18";

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
            r#"{{"jsonrpc":"2.0","id":{id},"result":{{"protocolVersion":"{PROTOCOL_VERSION}","capabilities":{{"tools":{{}}}},"serverInfo":{{"name":"kx-connector-example","version":"1"}}}}}}"#
        ),
        "tools/list" => format!(
            r#"{{"jsonrpc":"2.0","id":{id},"result":{{"tools":[{}]}}}}"#,
            concat!(
                r#"{"name":"echo","description":"Return the arguments verbatim.","inputSchema":{"type":"object","properties":{"q":{"type":"string"}},"required":["q"]}},"#,
                r#"{"name":"reverse","description":"Return the text argument reversed.","inputSchema":{"type":"object","properties":{"text":{"type":"string"}},"required":["text"]}}"#
            )
        ),
        "tools/call" => call(req, id),
        other => format!(
            r#"{{"jsonrpc":"2.0","id":{id},"error":{{"code":-32601,"message":"no such method: {other}"}}}}"#
        ),
    }
}

/// Execute a `tools/call`: dispatch on the tool name, never touching the environment.
fn call(req: &Req, id: u64) -> String {
    let params = req.params.as_ref();
    let name = params.and_then(|p| p.name.as_deref()).unwrap_or_default();
    let args_raw = params
        .and_then(|p| p.arguments.as_ref())
        .map_or_else(|| "{}".to_string(), |a| a.get().to_string());
    match name {
        "echo" => format!(r#"{{"jsonrpc":"2.0","id":{id},"result":{{"echoed":{args_raw}}}}}"#),
        "reverse" => {
            let text = serde_json::from_str::<serde_json::Value>(&args_raw)
                .ok()
                .and_then(|v| v.get("text").and_then(|t| t.as_str()).map(str::to_string))
                .unwrap_or_default();
            let reversed: String = text.chars().rev().collect();
            // Re-encode the reversed string as a JSON string value (escaping-safe).
            let encoded = serde_json::to_string(&reversed).unwrap_or_else(|_| "\"\"".to_string());
            format!(r#"{{"jsonrpc":"2.0","id":{id},"result":{{"reversed":{encoded}}}}}"#)
        }
        other => format!(
            r#"{{"jsonrpc":"2.0","id":{id},"error":{{"code":-32602,"message":"no such tool: {other}"}}}}"#
        ),
    }
}
