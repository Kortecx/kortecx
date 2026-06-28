//! `kx-mcp-kv` — a BUNDLED deterministic stdio MCP key-value lookup tool (RC1, D172): a
//! newline-delimited JSON-RPC `tools/call` responder that returns the value for a `key`
//! from a small FIXED seed map. It gives the kx-eval golden suite a deterministic
//! RETRIEVAL oracle for lookup / grounded-answer tasks (paired with `kx-mcp-calc` for
//! sequential reasoning).
//!
//! Same minimal-surface contract as `kx-mcp-echo`: deterministic in the request args, no
//! egress (`net_scope_required: None`), no modes, never echoes its environment. The seed
//! map is a compile-time constant — a bundled tool carries no external state. An unknown
//! key is a fail-closed JSON-RPC error.

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
    key: String,
}

/// The fixed seed map (deterministic). Values are simple constants — no quotes / control
/// bytes — so they embed safely in the JSON-RPC reply string.
const SEED: &[(&str, &str)] = &[
    ("a", "alpha"),
    ("b", "beta"),
    ("capital_of_france", "Paris"),
    ("x", "42"),
];

fn main() {
    let mut line = String::new();
    let stdin = std::io::stdin();
    let _ = stdin.lock().read_line(&mut line);

    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let _ = writeln!(out, "{}", reply(&line));
    let _ = out.flush();
}

/// The value for `key`, or `None` when the key is not in the seed map.
fn lookup(key: &str) -> Option<&'static str> {
    SEED.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
}

/// Decode the request, look up the key, and frame the JSON-RPC reply. Fail-closed.
fn reply(request_line: &str) -> String {
    let Ok(req) = serde_json::from_str::<Req>(request_line) else {
        return r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32700,"message":"parse error"}}"#
            .to_string();
    };
    match req.params.arguments.as_ref().and_then(|a| lookup(&a.key)) {
        Some(value) => format!(r#"{{"jsonrpc":"2.0","id":1,"result":{{"value":"{value}"}}}}"#),
        None => r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32602,"message":"unknown key"}}"#
            .to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get(key: &str) -> String {
        reply(&format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"arguments":{{"key":"{key}"}}}}}}"#
        ))
    }

    #[test]
    fn lookups_are_deterministic() {
        assert!(get("x").contains(r#""value":"42""#));
        assert!(get("capital_of_france").contains(r#""value":"Paris""#));
        assert_eq!(get("a"), get("a")); // identical args ⇒ identical reply bytes
    }

    #[test]
    fn fail_closed() {
        assert!(get("nonexistent").contains("unknown key"));
        assert!(reply("not json").contains("parse error"));
    }
}
