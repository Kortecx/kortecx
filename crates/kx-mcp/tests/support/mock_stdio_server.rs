//! Test-support MCP server: a newline-delimited JSON-RPC `tools/call` responder
//! the integration tests spawn to exercise the REAL [`kx_mcp::StdioTransport`]
//! (env injection + round-trip). NOT part of the library surface.
//!
//! Behaviour is selected by `KX_MCP_MOCK_MODE` (default `echo`):
//! - `echo` — reply `{"result":{"echoed":<arguments verbatim>}}`, deterministic in the request args (content-addressed dedup on replay).
//! - `big` — reply with a result string of `KX_MCP_MOCK_BIG_BYTES` chars (drives the IMP-16 oversize-cap refusal).
//! - `error` — reply with a JSON-RPC `error` object.
//! - `malformed` — reply with truncated/garbled JSON.
//! - `slow` — sleep `KX_MCP_MOCK_SLEEP_MS` then `echo` (drives the timeout path).
//!
//! A well-behaved server NEVER echoes its environment, so the secrets-never-leak
//! test can prove an injected credential does not reach any sink.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

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
    let mode = std::env::var("KX_MCP_MOCK_MODE").unwrap_or_else(|_| "echo".to_string());

    // Read exactly one JSON-RPC request line from stdin.
    let mut line = String::new();
    let stdin = std::io::stdin();
    let _ = stdin.lock().read_line(&mut line);

    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    let response: String = match mode.as_str() {
        "big" => {
            let n: usize = std::env::var("KX_MCP_MOCK_BIG_BYTES")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(1_000_000);
            let filler = "x".repeat(n);
            format!(r#"{{"jsonrpc":"2.0","id":1,"result":{{"blob":"{filler}"}}}}"#)
        }
        "error" => {
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"mock error"}}"#.to_string()
        }
        "malformed" => r#"{"jsonrpc":"2.0","id":1,"result":{"content":"#.to_string(),
        "slow" => {
            let ms: u64 = std::env::var("KX_MCP_MOCK_SLEEP_MS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(60_000);
            std::thread::sleep(std::time::Duration::from_millis(ms));
            echo(&line)
        }
        _ => echo(&line),
    };

    let _ = writeln!(out, "{response}");
    let _ = out.flush();
}

/// Echo the request's `arguments` verbatim inside a JSON-RPC result.
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
