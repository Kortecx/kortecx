//! `kx-mcp-calc` — a BUNDLED deterministic stdio MCP arithmetic tool (RC1, D172): a
//! newline-delimited JSON-RPC `tools/call` responder that computes an INTEGER arithmetic
//! op (`add` / `sub` / `mul` / `div`) over two integer args. It gives the kx-eval golden
//! suite a real second tool for genuine multi-tool / `ToolBatch` / sequential-reasoning
//! tasks with a DETERMINISTIC answer oracle.
//!
//! Same minimal-surface contract as `kx-mcp-echo`: deterministic in the request args, no
//! egress (`net_scope_required: None`), no modes, never echoes its environment. INTEGER
//! ONLY (no float — SN-8): a non-integer op or a division by zero is a fail-closed
//! JSON-RPC error (the caller's `decode_tool_result` surfaces it as a capability failure).

use std::io::{BufRead, Write};

use serde::Deserialize;

#[derive(Deserialize)]
struct Req {
    params: Params,
}

#[derive(Deserialize)]
struct Params {
    #[serde(default)]
    arguments: Option<Args>,
}

#[derive(Deserialize)]
struct Args {
    op: String,
    a: i64,
    b: i64,
}

fn main() {
    let mut line = String::new();
    let stdin = std::io::stdin();
    let _ = stdin.lock().read_line(&mut line);

    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let _ = writeln!(out, "{}", reply(&line));
    let _ = out.flush();
}

/// Compute the op, or `None` for an unknown op / overflow / division by zero.
fn compute(args: &Args) -> Option<i64> {
    match args.op.as_str() {
        "add" => args.a.checked_add(args.b),
        "sub" => args.a.checked_sub(args.b),
        "mul" => args.a.checked_mul(args.b),
        "div" => args.a.checked_div(args.b),
        _ => None,
    }
}

/// Decode the request, compute the op, and frame the JSON-RPC reply. Fail-closed.
fn reply(request_line: &str) -> String {
    let Ok(req) = serde_json::from_str::<Req>(request_line) else {
        return r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32700,"message":"parse error"}}"#
            .to_string();
    };
    match req.params.arguments.as_ref().and_then(compute) {
        Some(result) => format!(r#"{{"jsonrpc":"2.0","id":1,"result":{{"result":{result}}}}}"#),
        None => r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32602,"message":"invalid op or division by zero"}}"#
            .to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(op: &str, a: i64, b: i64) -> String {
        reply(&format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"arguments":{{"op":"{op}","a":{a},"b":{b}}}}}}}"#
        ))
    }

    #[test]
    fn add_is_deterministic() {
        assert!(call("add", 42, 10).contains(r#""result":52"#));
        // identical args ⇒ identical reply bytes (the exactly-once dedup contract).
        assert_eq!(call("add", 42, 10), call("add", 42, 10));
    }

    #[test]
    fn ops() {
        assert!(call("sub", 10, 3).contains(r#""result":7"#));
        assert!(call("mul", 6, 7).contains(r#""result":42"#));
        assert!(call("div", 20, 4).contains(r#""result":5"#));
    }

    #[test]
    fn fail_closed() {
        assert!(call("div", 1, 0).contains("error")); // division by zero
        assert!(call("pow", 2, 3).contains("error")); // unknown op
        assert!(reply("not json").contains("parse error"));
    }
}
