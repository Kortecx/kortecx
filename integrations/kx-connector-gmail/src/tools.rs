// SPDX-License-Identifier: Apache-2.0
//! The Gmail tool catalog (the `tools/list` payload) and `tools/call` routing.
//!
//! Argument decoding is fail-closed: a missing/ill-typed field yields a JSON-RPC
//! invalid-params error (`-32602`), never a fabricated call. A Gmail execution
//! failure yields a server error (`-32000`) carrying the typed reason — never the
//! credential.

use serde::Deserialize;

use crate::gmail::{GmailClient, GmailError};
use crate::mcp::{error, result, Req};

/// The `tools/list` array: the four Gmail tools with typed JSON-Schema inputs.
#[must_use]
pub fn catalog_json() -> &'static str {
    concat!(
        r#"[{"name":"search","description":"Search the mailbox with a Gmail query (e.g. 'from:alice is:unread'). Returns matching message ids.","inputSchema":{"type":"object","properties":{"query":{"type":"string"},"max_results":{"type":"integer"}},"required":["query"]}},"#,
        r#"{"name":"read","description":"Read one message by id: headers (from/to/subject/date), snippet, and a plain-text body when present.","inputSchema":{"type":"object","properties":{"message_id":{"type":"string"}},"required":["message_id"]}},"#,
        r#"{"name":"draft","description":"Create a draft email (does not send). Returns the draft id.","inputSchema":{"type":"object","properties":{"to":{"type":"string"},"subject":{"type":"string"},"body":{"type":"string"}},"required":["to","subject","body"]}},"#,
        r#"{"name":"send","description":"Send an email immediately. Returns the sent message id.","inputSchema":{"type":"object","properties":{"to":{"type":"string"},"subject":{"type":"string"},"body":{"type":"string"}},"required":["to","subject","body"]}}]"#
    )
}

#[derive(Deserialize)]
struct SearchArgs {
    query: String,
    #[serde(default)]
    max_results: Option<u32>,
}

#[derive(Deserialize)]
struct ReadArgs {
    message_id: String,
}

#[derive(Deserialize)]
struct ComposeArgs {
    to: String,
    subject: String,
    body: String,
}

/// Route a `tools/call` request to the Gmail client and frame the reply.
///
/// Unknown tool or bad arguments -> invalid-params (`-32602`); a Gmail failure ->
/// server error (`-32000`); success -> a JSON-RPC `result` object. Never returns
/// the credential in any branch.
#[must_use]
pub fn call(req: &Req, client: &GmailClient) -> String {
    let id = req.id;
    let params = req.params.as_ref();
    let name = params.and_then(|p| p.name.as_deref()).unwrap_or_default();
    let args_raw = params
        .and_then(|p| p.arguments.as_ref())
        .map_or_else(|| "{}".to_string(), |a| a.get().to_string());

    match name {
        "search" => match decode::<SearchArgs>(&args_raw) {
            Ok(a) => finish(id, client.search(&a.query, a.max_results.unwrap_or(10))),
            Err(e) => error(id, -32602, &e),
        },
        "read" => match decode::<ReadArgs>(&args_raw) {
            Ok(a) => finish(id, client.read(&a.message_id)),
            Err(e) => error(id, -32602, &e),
        },
        "draft" => match decode::<ComposeArgs>(&args_raw) {
            Ok(a) => finish(id, client.draft(&a.to, &a.subject, &a.body)),
            Err(e) => error(id, -32602, &e),
        },
        "send" => match decode::<ComposeArgs>(&args_raw) {
            Ok(a) => finish(id, client.send(&a.to, &a.subject, &a.body)),
            Err(e) => error(id, -32602, &e),
        },
        other => error(id, -32602, &format!("no such tool: {other}")),
    }
}

/// Frame a Gmail result (or its typed error) as a JSON-RPC reply.
fn finish(id: u64, outcome: Result<String, GmailError>) -> String {
    match outcome {
        Ok(result_json) => result(id, &result_json),
        Err(e) => error(id, -32000, &e.to_string()),
    }
}

/// Fail-closed argument decode; the `Err` string is a human-readable reason.
fn decode<T: serde::de::DeserializeOwned>(raw: &str) -> Result<T, String> {
    serde_json::from_str::<T>(raw).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::call;
    use crate::gmail::GmailClient;
    use crate::mcp::{Params, Req};

    fn call_tool(name: &str, args: &str) -> String {
        let req = Req {
            id: 7,
            method: "tools/call".to_string(),
            params: Some(Params {
                name: Some(name.to_string()),
                arguments: Some(
                    serde_json::value::RawValue::from_string(args.to_string()).unwrap(),
                ),
            }),
        };
        call(&req, &GmailClient::fake())
    }

    #[test]
    fn search_fake_returns_a_result_and_echoes_the_query() {
        let reply = call_tool("search", r#"{"query":"is:unread"}"#);
        assert!(reply.contains(r#""result""#));
        assert!(reply.contains("is:unread"));
        assert!(reply.contains("fake-msg-1"));
    }

    #[test]
    fn send_fake_returns_a_message_id() {
        let reply = call_tool("send", r#"{"to":"a@b.com","subject":"hi","body":"yo"}"#);
        assert!(reply.contains("message_id"));
        assert!(reply.contains("SENT"));
    }

    #[test]
    fn missing_required_arg_is_invalid_params() {
        let reply = call_tool("search", r#"{}"#);
        assert!(reply.contains("-32602"));
    }

    #[test]
    fn unknown_tool_is_invalid_params() {
        let reply = call_tool("delete_everything", r#"{}"#);
        assert!(reply.contains("-32602"));
        assert!(reply.contains("no such tool"));
    }
}
