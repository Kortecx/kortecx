// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! The Slack tool catalog (the `tools/list` payload) and `tools/call` routing.
//!
//! Argument decoding is fail-closed: a missing/ill-typed field yields a JSON-RPC
//! invalid-params error (`-32602`), never a fabricated call. A Slack execution
//! failure yields a server error (`-32000`) carrying the typed reason — never the
//! credential.

use serde::Deserialize;

use crate::mcp::{error, result, Req};
use crate::slack::{SlackClient, SlackError};

/// The `tools/list` array: the four Slack tools with typed JSON-Schema inputs.
///
/// The channel-addressed tools accept EITHER `channel_id` or `channel` (the alias
/// is honoured at decode time); the schema's `anyOf` states that at least one is
/// required so a caller-side validator can prompt for it.
#[must_use]
pub fn catalog_json() -> &'static str {
    concat!(
        r#"[{"name":"post_message","description":"Post a message to a Slack channel. Returns {ts, channel}.","inputSchema":{"type":"object","properties":{"channel_id":{"type":"string"},"channel":{"type":"string"},"text":{"type":"string"}},"required":["text"],"anyOf":[{"required":["channel_id"]},{"required":["channel"]}]}},"#,
        r#"{"name":"read_channel","description":"Read recent messages from a Slack channel (most recent first). Returns [{user,text,ts}].","inputSchema":{"type":"object","properties":{"channel_id":{"type":"string"},"channel":{"type":"string"},"limit":{"type":"integer"}},"anyOf":[{"required":["channel_id"]},{"required":["channel"]}]}},"#,
        r#"{"name":"search","description":"Search messages across the Slack workspace. Returns [{text,channel,ts}].","inputSchema":{"type":"object","properties":{"query":{"type":"string"}},"required":["query"]}},"#,
        r#"{"name":"list_channels","description":"List the channels of the Slack workspace. Returns [{id,name}].","inputSchema":{"type":"object","properties":{}}}]"#
    )
}

#[derive(Deserialize)]
struct PostArgs {
    #[serde(alias = "channel")]
    channel_id: String,
    text: String,
}

#[derive(Deserialize)]
struct ReadArgs {
    #[serde(alias = "channel")]
    channel_id: String,
    #[serde(default)]
    limit: Option<u32>,
}

#[derive(Deserialize)]
struct SearchArgs {
    query: String,
}

/// `list_channels` takes no arguments (an empty object decodes fine).
#[derive(Deserialize)]
struct ListArgs {}

/// Route a `tools/call` request to the Slack client and frame the reply.
///
/// Unknown tool or bad arguments -> invalid-params (`-32602`); a Slack failure ->
/// server error (`-32000`); success -> a JSON-RPC `result` object. Never returns
/// the credential in any branch.
#[must_use]
pub fn call(req: &Req, client: &SlackClient) -> String {
    let id = req.id;
    let params = req.params.as_ref();
    let name = params.and_then(|p| p.name.as_deref()).unwrap_or_default();
    let args_raw = params
        .and_then(|p| p.arguments.as_ref())
        .map_or_else(|| "{}".to_string(), |a| a.get().to_string());

    match name {
        "post_message" => match decode::<PostArgs>(&args_raw) {
            Ok(a) => finish(id, client.post_message(&a.channel_id, &a.text)),
            Err(e) => error(id, -32602, &e),
        },
        "read_channel" => match decode::<ReadArgs>(&args_raw) {
            Ok(a) => finish(
                id,
                client.read_channel(&a.channel_id, a.limit.unwrap_or(20)),
            ),
            Err(e) => error(id, -32602, &e),
        },
        "search" => match decode::<SearchArgs>(&args_raw) {
            Ok(a) => finish(id, client.search(&a.query)),
            Err(e) => error(id, -32602, &e),
        },
        "list_channels" => match decode::<ListArgs>(&args_raw) {
            Ok(_) => finish(id, client.list_channels()),
            Err(e) => error(id, -32602, &e),
        },
        other => error(id, -32602, &format!("no such tool: {other}")),
    }
}

/// Frame a Slack result (or its typed error) as a JSON-RPC reply.
fn finish(id: u64, outcome: Result<String, SlackError>) -> String {
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
    use crate::mcp::{Params, Req};
    use crate::slack::SlackClient;

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
        call(&req, &SlackClient::fake())
    }

    #[test]
    fn post_message_fake_returns_ts_and_echoes_channel() {
        let reply = call_tool(
            "post_message",
            r#"{"channel_id":"C0123ABCD","text":"hello world"}"#,
        );
        assert!(reply.contains(r#""result""#));
        assert!(reply.contains("ts"));
        assert!(reply.contains("C0123ABCD"));
    }

    #[test]
    fn post_message_accepts_channel_alias() {
        // `channel` is the Slack-native field name; the decode struct aliases it.
        let reply = call_tool("post_message", r#"{"channel":"C0123ABCD","text":"hi"}"#);
        assert!(reply.contains(r#""result""#));
        assert!(reply.contains("C0123ABCD"));
    }

    #[test]
    fn read_channel_fake_returns_messages() {
        let reply = call_tool("read_channel", r#"{"channel_id":"C0123ABCD"}"#);
        assert!(reply.contains(r#""result""#));
        assert!(reply.contains("fake-user"));
    }

    #[test]
    fn search_fake_returns_matches() {
        let reply = call_tool("search", r#"{"query":"budget"}"#);
        assert!(reply.contains(r#""result""#));
        assert!(reply.contains("a fake match"));
    }

    #[test]
    fn list_channels_fake_returns_channels() {
        let reply = call_tool("list_channels", r#"{}"#);
        assert!(reply.contains(r#""result""#));
        assert!(reply.contains("general"));
    }

    #[test]
    fn missing_required_arg_is_invalid_params() {
        let reply = call_tool("post_message", r#"{"channel_id":"C0123ABCD"}"#);
        assert!(reply.contains("-32602"));
    }

    #[test]
    fn unknown_tool_is_invalid_params() {
        let reply = call_tool("delete_everything", r#"{}"#);
        assert!(reply.contains("-32602"));
        assert!(reply.contains("no such tool"));
    }
}
