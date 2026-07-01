// SPDX-License-Identifier: Apache-2.0
//! The Discord client: the Discord REST calls authenticated with a bot token — or
//! a deterministic offline `fake` mode for tests / CI / conformance.
//!
//! ## Credential (D81)
//! The bot token arrives by-reference in the environment variable named by
//! [`CREDENTIAL_ENV`] (`KX_DISCORD_CREDENTIAL`), as a JSON object holding
//! `bot_token`. It is read here and sent as an `Authorization: Bot <token>` header
//! to the Discord REST API. The token value is never returned in a tool result, a
//! log line, or an error string.

use serde::Deserialize;
use serde_json::{json, Value};

use crate::validate::{check_content, check_snowflake};

/// The environment variable the operator wires as the connection's `credential_ref`.
/// It holds the credential as JSON: `{"bot_token": "..."}`.
pub const CREDENTIAL_ENV: &str = "KX_DISCORD_CREDENTIAL";

/// When set to a truthy value, the connector runs OFFLINE with canned responses —
/// no network, no credential. Used by tests, CI, and the conformance harness.
pub const FAKE_ENV: &str = "KX_DISCORD_FAKE";

const API_BASE: &str = "https://discord.com/api/v10";

/// Discord requires a descriptive `User-Agent` on REST requests (dev docs).
const USER_AGENT: &str = "DiscordBot (https://github.com/Kortecx/kortecx, 0.1)";

/// A typed connector error. Rendered into a fail-closed JSON-RPC error message;
/// no variant ever carries the credential value.
#[derive(Debug, thiserror::Error)]
pub enum DiscordError {
    /// The tool arguments were missing, ill-typed, or failed a guard.
    #[error("invalid arguments: {0}")]
    BadArgs(String),
    /// No usable credential was injected (the connection has no resolvable `credential_ref`).
    #[error("no Discord credential available (set the connection's credential_ref)")]
    NoCredential,
    /// The Discord API returned a non-success HTTP status.
    #[error("discord api error (status {0})")]
    Status(u16),
    /// The Discord API could not be reached.
    #[error("discord unreachable: {0}")]
    Unreachable(String),
    /// The Discord API returned a body that did not match the expected shape.
    #[error("unexpected discord response")]
    BadResponse,
}

/// The bot credential, parsed from [`CREDENTIAL_ENV`]. Private — never serialized back.
#[derive(Deserialize)]
struct BotConfig {
    bot_token: String,
}

/// The connector's Discord client — an offline fake or a live bot client. The mode
/// is a private detail; construct via [`DiscordClient::from_env`] or [`DiscordClient::fake`].
pub struct DiscordClient {
    mode: Mode,
}

/// Private client state — kept out of the public API so `BotConfig` stays internal.
enum Mode {
    Fake,
    Live(Option<BotConfig>),
}

impl DiscordClient {
    /// Build from the process environment.
    ///
    /// `KX_DISCORD_FAKE` (truthy) selects offline mode. Otherwise the injected
    /// `KX_DISCORD_CREDENTIAL` JSON is parsed; an absent or malformed value yields a
    /// live client with no credential, so tool calls fail closed (the runtime is
    /// never handed a fabricated credential).
    #[must_use]
    pub fn from_env() -> Self {
        if fake_enabled() {
            return Self::fake();
        }
        let cfg = std::env::var(CREDENTIAL_ENV)
            .ok()
            .filter(|s| !s.is_empty())
            .and_then(|raw| serde_json::from_str::<BotConfig>(&raw).ok());
        Self {
            mode: Mode::Live(cfg),
        }
    }

    /// A client that always returns deterministic offline responses (tests / embedding).
    #[must_use]
    pub fn fake() -> Self {
        Self { mode: Mode::Fake }
    }

    /// Post a message to a channel; returns `{message_id, channel_id}`.
    ///
    /// # Errors
    /// [`DiscordError::BadArgs`] on a bad id/content, [`DiscordError::NoCredential`],
    /// or a `Status` / `Unreachable` / `BadResponse` error from the API call.
    pub fn send_message(&self, channel_id: &str, content: &str) -> Result<String, DiscordError> {
        check_snowflake("channel_id", channel_id)?;
        check_content(content)?;
        if let Mode::Fake = self.mode {
            return Ok(fake::send(channel_id));
        }
        let token = self.bot_token()?;
        let url = format!("{API_BASE}/channels/{channel_id}/messages");
        let v = parse(&http_post_json(
            &url,
            token,
            &json!({ "content": content }),
        )?)?;
        Ok(stringify(&json!({
            "message_id": v.get("id").and_then(Value::as_str).unwrap_or_default(),
            "channel_id": channel_id,
        })))
    }

    /// Read recent messages from a channel; returns `[{id, author, content, timestamp}]`.
    ///
    /// # Errors
    /// [`DiscordError::BadArgs`] on a bad id, [`DiscordError::NoCredential`], or a
    /// `Status` / `Unreachable` / `BadResponse` error from the API call.
    pub fn read_channel(&self, channel_id: &str, limit: u32) -> Result<String, DiscordError> {
        check_snowflake("channel_id", channel_id)?;
        if let Mode::Fake = self.mode {
            return Ok(fake::read());
        }
        let token = self.bot_token()?;
        let max = limit.clamp(1, 100);
        let url = format!("{API_BASE}/channels/{channel_id}/messages?limit={max}");
        let v = parse(&http_get(&url, token)?)?;
        Ok(stringify(&shape_messages(&v)))
    }

    /// List a guild's channels; returns `[{id, name, type}]`.
    ///
    /// # Errors
    /// [`DiscordError::BadArgs`] on a bad id, [`DiscordError::NoCredential`], or a
    /// `Status` / `Unreachable` / `BadResponse` error from the API call.
    pub fn list_channels(&self, guild_id: &str) -> Result<String, DiscordError> {
        check_snowflake("guild_id", guild_id)?;
        if let Mode::Fake = self.mode {
            return Ok(fake::list());
        }
        let token = self.bot_token()?;
        let url = format!("{API_BASE}/guilds/{guild_id}/channels");
        let v = parse(&http_get(&url, token)?)?;
        Ok(stringify(&shape_channels(&v)))
    }

    /// The bot token (live mode only); `NoCredential` when none was injected.
    fn bot_token(&self) -> Result<&str, DiscordError> {
        match &self.mode {
            Mode::Fake => Ok("fake-bot-token"),
            Mode::Live(cfg) => cfg
                .as_ref()
                .map(|c| c.bot_token.as_str())
                .ok_or(DiscordError::NoCredential),
        }
    }
}

fn fake_enabled() -> bool {
    std::env::var(FAKE_ENV).is_ok_and(|v| {
        let v = v.to_ascii_lowercase();
        !v.is_empty() && v != "0" && v != "false"
    })
}

/// Reduce a Discord messages array to `[{id, author, content, timestamp}]`.
fn shape_messages(v: &Value) -> Value {
    let Some(arr) = v.as_array() else {
        return json!([]);
    };
    let out: Vec<Value> = arr
        .iter()
        .map(|m| {
            let author = m
                .get("author")
                .and_then(|a| {
                    a.get("global_name")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                        .or_else(|| a.get("username").and_then(Value::as_str))
                })
                .unwrap_or_default();
            json!({
                "id": m.get("id").and_then(Value::as_str).unwrap_or_default(),
                "author": author,
                "content": m.get("content").and_then(Value::as_str).unwrap_or_default(),
                "timestamp": m.get("timestamp").and_then(Value::as_str).unwrap_or_default(),
            })
        })
        .collect();
    Value::Array(out)
}

/// Reduce a Discord channels array to `[{id, name, type}]`.
fn shape_channels(v: &Value) -> Value {
    let Some(arr) = v.as_array() else {
        return json!([]);
    };
    let out: Vec<Value> = arr
        .iter()
        .map(|c| {
            json!({
                "id": c.get("id").and_then(Value::as_str).unwrap_or_default(),
                "name": c.get("name").and_then(Value::as_str).unwrap_or_default(),
                "type": c.get("type").and_then(Value::as_u64).unwrap_or_default(),
            })
        })
        .collect();
    Value::Array(out)
}

/// GET a URL with the bot token; returns the response body as a string.
fn http_get(url: &str, token: &str) -> Result<String, DiscordError> {
    ureq::get(url)
        .set("Authorization", &format!("Bot {token}"))
        .set("User-Agent", USER_AGENT)
        .call()
        .map_err(classify)?
        .into_string()
        .map_err(|e| DiscordError::Unreachable(e.to_string()))
}

/// POST a JSON body with the bot token; returns the response body as a string.
fn http_post_json(url: &str, token: &str, body: &Value) -> Result<String, DiscordError> {
    let payload = serde_json::to_string(body).map_err(|_| DiscordError::BadResponse)?;
    ureq::post(url)
        .set("Authorization", &format!("Bot {token}"))
        .set("User-Agent", USER_AGENT)
        .set("Content-Type", "application/json")
        .send_string(&payload)
        .map_err(classify)?
        .into_string()
        .map_err(|e| DiscordError::Unreachable(e.to_string()))
}

fn parse(body: &str) -> Result<Value, DiscordError> {
    serde_json::from_str::<Value>(body).map_err(|_| DiscordError::BadResponse)
}

fn stringify(v: &Value) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "{}".to_string())
}

fn classify(err: ureq::Error) -> DiscordError {
    match err {
        ureq::Error::Status(code, _) => DiscordError::Status(code),
        ureq::Error::Transport(t) => DiscordError::Unreachable(t.to_string()),
    }
}

/// Deterministic canned responses for offline mode (no network, no credential).
mod fake {
    /// A fake sent-message id, echoing the (non-secret) channel for test assertions.
    pub(super) fn send(channel_id: &str) -> String {
        let ch = serde_json::to_string(channel_id).unwrap_or_else(|_| "\"\"".to_string());
        format!(r#"{{"message_id":"fake-msg-3","channel_id":{ch}}}"#)
    }

    /// A fake one-message history.
    pub(super) fn read() -> String {
        r#"[{"id":"fake-msg-1","author":"alice","content":"a fake message","timestamp":"2026-07-01T10:00:00.000000+00:00"}]"#
            .to_string()
    }

    /// A fake channel list.
    pub(super) fn list() -> String {
        r#"[{"id":"fake-chan-1","name":"general","type":0}]"#.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::{fake_enabled, shape_channels, shape_messages, DiscordClient, Mode};
    use serde_json::json;

    #[test]
    fn shape_messages_pulls_author_content_and_timestamp() {
        let raw = json!([
            {
                "id": "m1",
                "author": {"username": "bob", "global_name": "Bob B"},
                "content": "hi there",
                "timestamp": "2026-07-01T10:00:00+00:00"
            }
        ]);
        let shaped = shape_messages(&raw);
        let first = &shaped.as_array().unwrap()[0];
        assert_eq!(first.get("id").unwrap(), "m1");
        assert_eq!(first.get("author").unwrap(), "Bob B");
        assert_eq!(first.get("content").unwrap(), "hi there");
    }

    #[test]
    fn shape_messages_falls_back_to_username() {
        let raw = json!([{ "id": "m2", "author": {"username": "carol"}, "content": "x" }]);
        let shaped = shape_messages(&raw);
        assert_eq!(
            shaped.as_array().unwrap()[0].get("author").unwrap(),
            "carol"
        );
    }

    #[test]
    fn shape_channels_pulls_id_name_type() {
        let raw = json!([{ "id": "c1", "name": "general", "type": 0 }]);
        let shaped = shape_channels(&raw);
        let first = &shaped.as_array().unwrap()[0];
        assert_eq!(first.get("name").unwrap(), "general");
        assert_eq!(first.get("type").unwrap(), 0);
    }

    #[test]
    fn live_without_credential_fails_closed() {
        let client = DiscordClient {
            mode: Mode::Live(None),
        };
        let err = client.send_message("123", "hi").unwrap_err();
        assert!(matches!(err, super::DiscordError::NoCredential));
    }

    #[test]
    fn bad_channel_id_fails_before_any_network() {
        // A non-digit id is rejected by the guard regardless of credential/mode.
        let client = DiscordClient {
            mode: Mode::Live(None),
        };
        let err = client.read_channel("../secrets", 5).unwrap_err();
        assert!(matches!(err, super::DiscordError::BadArgs(_)));
    }

    #[test]
    fn fake_env_truthiness() {
        std::env::set_var(super::FAKE_ENV, "1");
        assert!(fake_enabled());
        std::env::set_var(super::FAKE_ENV, "false");
        assert!(!fake_enabled());
        std::env::remove_var(super::FAKE_ENV);
        assert!(!fake_enabled());
    }
}
