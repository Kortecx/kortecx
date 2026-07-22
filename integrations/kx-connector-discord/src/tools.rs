// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! The Discord tool catalog (the `tools/list` payload) and `tools/call` routing.
//!
//! Argument decoding is fail-closed: a missing/ill-typed field yields a JSON-RPC
//! invalid-params error (`-32602`), never a fabricated call. A Discord execution
//! failure yields a server error (`-32000`) carrying the typed reason — never the
//! credential.

use serde::Deserialize;

use crate::discord::{DiscordClient, DiscordError};
use crate::mcp::{error, result, Req};

/// The `tools/list` array: the three Discord tools with typed JSON-Schema inputs.
#[must_use]
pub fn catalog_json() -> &'static str {
    concat!(
        r#"[{"name":"send_message","description":"Post a message to a Discord channel. Returns the new message id.","inputSchema":{"type":"object","properties":{"channel_id":{"type":"string"},"content":{"type":"string"}},"required":["channel_id","content"]}},"#,
        r#"{"name":"read_channel","description":"Read recent messages from a Discord channel (most recent first). Returns [{id,author,content,timestamp}].","inputSchema":{"type":"object","properties":{"channel_id":{"type":"string"},"limit":{"type":"integer"}},"required":["channel_id"]}},"#,
        r#"{"name":"list_channels","description":"List the channels of a Discord guild (server). Returns [{id,name,type}].","inputSchema":{"type":"object","properties":{"guild_id":{"type":"string"}},"required":["guild_id"]}}]"#
    )
}

#[derive(Deserialize)]
struct SendArgs {
    channel_id: String,
    content: String,
}

#[derive(Deserialize)]
struct ReadArgs {
    channel_id: String,
    #[serde(default)]
    limit: Option<u32>,
}

#[derive(Deserialize)]
struct ListArgs {
    guild_id: String,
}

/// Route a `tools/call` request to the Discord client and frame the reply.
///
/// Unknown tool or bad arguments -> invalid-params (`-32602`); a Discord failure ->
/// server error (`-32000`); success -> a JSON-RPC `result` object. Never returns
/// the credential in any branch.
#[must_use]
pub fn call(req: &Req, client: &DiscordClient) -> String {
    let id = req.id;
    let params = req.params.as_ref();
    let name = params.and_then(|p| p.name.as_deref()).unwrap_or_default();
    let args_raw = params
        .and_then(|p| p.arguments.as_ref())
        .map_or_else(|| "{}".to_string(), |a| a.get().to_string());

    match name {
        "send_message" => match decode::<SendArgs>(&args_raw) {
            Ok(a) => finish(id, client.send_message(&a.channel_id, &a.content)),
            Err(e) => error(id, -32602, &e),
        },
        "read_channel" => match decode::<ReadArgs>(&args_raw) {
            Ok(a) => finish(
                id,
                client.read_channel(&a.channel_id, a.limit.unwrap_or(20)),
            ),
            Err(e) => error(id, -32602, &e),
        },
        "list_channels" => match decode::<ListArgs>(&args_raw) {
            Ok(a) => finish(id, client.list_channels(&a.guild_id)),
            Err(e) => error(id, -32602, &e),
        },
        other => error(id, -32602, &format!("no such tool: {other}")),
    }
}

/// Frame a Discord result (or its typed error) as a JSON-RPC reply.
fn finish(id: u64, outcome: Result<String, DiscordError>) -> String {
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
    use crate::discord::DiscordClient;
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
        call(&req, &DiscordClient::fake())
    }

    #[test]
    fn send_message_fake_returns_a_message_id_and_echoes_channel() {
        let reply = call_tool(
            "send_message",
            r#"{"channel_id":"123","content":"hello world"}"#,
        );
        assert!(reply.contains(r#""result""#));
        assert!(reply.contains("message_id"));
        assert!(reply.contains("123"));
    }

    #[test]
    fn read_channel_fake_returns_messages() {
        let reply = call_tool("read_channel", r#"{"channel_id":"123"}"#);
        assert!(reply.contains(r#""result""#));
        assert!(reply.contains("fake-msg-1"));
    }

    #[test]
    fn list_channels_fake_returns_channels() {
        let reply = call_tool("list_channels", r#"{"guild_id":"999"}"#);
        assert!(reply.contains(r#""result""#));
        assert!(reply.contains("general"));
    }

    #[test]
    fn missing_required_arg_is_invalid_params() {
        let reply = call_tool("send_message", r#"{"channel_id":"123"}"#);
        assert!(reply.contains("-32602"));
    }

    #[test]
    fn unknown_tool_is_invalid_params() {
        let reply = call_tool("delete_everything", r#"{}"#);
        assert!(reply.contains("-32602"));
        assert!(reply.contains("no such tool"));
    }
}
