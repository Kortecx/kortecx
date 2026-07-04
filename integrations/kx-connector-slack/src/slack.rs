// SPDX-License-Identifier: Apache-2.0
//! The Slack client: the Slack Web API calls authenticated with a bot token — or a
//! deterministic offline `fake` mode for tests / CI / conformance.
//!
//! ## Credential (D81)
//! The bot token arrives by-reference in the environment variable named by
//! [`CREDENTIAL_ENV`] (`KX_SLACK_CREDENTIAL`), as a JSON object holding `bot_token`
//! (an `xoxb-…` token). It is read here and sent as an `Authorization: Bearer
//! <token>` header to the Slack Web API. The token value is never returned in a tool
//! result, a log line, or an error string.
//!
//! ## Slack's `ok` envelope
//! Unlike a REST API that signals failure with an HTTP status, the Slack Web API
//! returns HTTP 200 with `{"ok":false,"error":"…"}` on a logical failure. The
//! shapers therefore check `ok` after parsing and map `ok == false` to
//! [`SlackError::BadResponse`] (the value never carries the credential).

use serde::Deserialize;
use serde_json::{json, Value};

use crate::validate::{check_channel_id, check_text};

/// The environment variable the operator wires as the connection's `credential_ref`.
/// It holds the credential as JSON: `{"bot_token": "xoxb-…"}`.
pub const CREDENTIAL_ENV: &str = "KX_SLACK_CREDENTIAL";

/// When set to a truthy value, the connector runs OFFLINE with canned responses —
/// no network, no credential. Used by tests, CI, and the conformance harness.
pub const FAKE_ENV: &str = "KX_SLACK_FAKE";

const API_BASE: &str = "https://slack.com/api";

/// A typed connector error. Rendered into a fail-closed JSON-RPC error message;
/// no variant ever carries the credential value.
#[derive(Debug, thiserror::Error)]
pub enum SlackError {
    /// The tool arguments were missing, ill-typed, or failed a guard.
    #[error("invalid arguments: {0}")]
    BadArgs(String),
    /// No usable credential was injected (the connection has no resolvable `credential_ref`).
    #[error("no Slack credential available (set the connection's credential_ref)")]
    NoCredential,
    /// The Slack API returned a non-success HTTP status.
    #[error("slack api error (status {0})")]
    Status(u16),
    /// The Slack API could not be reached.
    #[error("slack unreachable: {0}")]
    Unreachable(String),
    /// The Slack API returned a body that did not match the expected shape, or an
    /// `{"ok":false}` envelope. The error string never carries the credential.
    #[error("unexpected slack response")]
    BadResponse,
}

/// The bot credential, parsed from [`CREDENTIAL_ENV`]. Private — never serialized back.
#[derive(Deserialize)]
struct BotConfig {
    bot_token: String,
}

/// The connector's Slack client — an offline fake or a live bot client. The mode is
/// a private detail; construct via [`SlackClient::from_env`] or [`SlackClient::fake`].
pub struct SlackClient {
    mode: Mode,
}

/// Private client state — kept out of the public API so `BotConfig` stays internal.
enum Mode {
    Fake,
    Live(Option<BotConfig>),
}

impl SlackClient {
    /// Build from the process environment.
    ///
    /// `KX_SLACK_FAKE` (truthy) selects offline mode. Otherwise the injected
    /// `KX_SLACK_CREDENTIAL` JSON is parsed; an absent or malformed value yields a
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

    /// Post a message to a channel; returns `{ts, channel}`.
    ///
    /// # Errors
    /// [`SlackError::BadArgs`] on a bad id/text, [`SlackError::NoCredential`], or a
    /// `Status` / `Unreachable` / `BadResponse` error from the API call.
    pub fn post_message(&self, channel_id: &str, text: &str) -> Result<String, SlackError> {
        check_channel_id("channel_id", channel_id)?;
        check_text(text)?;
        if let Mode::Fake = self.mode {
            return Ok(fake::post(channel_id));
        }
        let token = self.bot_token()?;
        let url = format!("{API_BASE}/chat.postMessage");
        let v = require_ok(&parse(&http_post_json(
            &url,
            token,
            &json!({ "channel": channel_id, "text": text }),
        )?)?)?;
        Ok(stringify(&json!({
            "ts": v.get("ts").and_then(Value::as_str).unwrap_or_default(),
            "channel": v.get("channel").and_then(Value::as_str).unwrap_or(channel_id),
        })))
    }

    /// Read recent messages from a channel; returns `[{user, text, ts}]`.
    ///
    /// # Errors
    /// [`SlackError::BadArgs`] on a bad id, [`SlackError::NoCredential`], or a
    /// `Status` / `Unreachable` / `BadResponse` error from the API call.
    pub fn read_channel(&self, channel_id: &str, limit: u32) -> Result<String, SlackError> {
        check_channel_id("channel_id", channel_id)?;
        if let Mode::Fake = self.mode {
            return Ok(fake::read());
        }
        let token = self.bot_token()?;
        let max = limit.clamp(1, 100);
        let url = format!("{API_BASE}/conversations.history?channel={channel_id}&limit={max}");
        let v = require_ok(&parse(&http_get(&url, token)?)?)?;
        Ok(stringify(&shape_messages(&v)))
    }

    /// Search messages across the workspace; returns `[{text, channel, ts}]`.
    ///
    /// # Errors
    /// [`SlackError::NoCredential`], or a `Status` / `Unreachable` / `BadResponse`
    /// error from the API call.
    pub fn search(&self, query: &str) -> Result<String, SlackError> {
        if let Mode::Fake = self.mode {
            return Ok(fake::search());
        }
        let token = self.bot_token()?;
        let url = format!("{API_BASE}/search.messages?query={}", urlencode(query));
        let v = require_ok(&parse(&http_get(&url, token)?)?)?;
        Ok(stringify(&shape_matches(&v)))
    }

    /// List the workspace's channels; returns `[{id, name}]`.
    ///
    /// # Errors
    /// [`SlackError::NoCredential`], or a `Status` / `Unreachable` / `BadResponse`
    /// error from the API call.
    pub fn list_channels(&self) -> Result<String, SlackError> {
        if let Mode::Fake = self.mode {
            return Ok(fake::list());
        }
        let token = self.bot_token()?;
        let url = format!("{API_BASE}/conversations.list");
        let v = require_ok(&parse(&http_get(&url, token)?)?)?;
        Ok(stringify(&shape_channels(&v)))
    }

    /// The bot token (live mode only); `NoCredential` when none was injected.
    fn bot_token(&self) -> Result<&str, SlackError> {
        match &self.mode {
            Mode::Fake => Ok("fake-bot-token"),
            Mode::Live(cfg) => cfg
                .as_ref()
                .map(|c| c.bot_token.as_str())
                .ok_or(SlackError::NoCredential),
        }
    }
}

fn fake_enabled() -> bool {
    std::env::var(FAKE_ENV).is_ok_and(|v| {
        let v = v.to_ascii_lowercase();
        !v.is_empty() && v != "0" && v != "false"
    })
}

/// Slack signals a logical failure with HTTP 200 + `{"ok":false,"error":"…"}`.
/// Treat a missing or `false` `ok` as an unexpected response (never leak the error
/// into a credential sink; the value carries no secret).
fn require_ok(v: &Value) -> Result<Value, SlackError> {
    if v.get("ok").and_then(Value::as_bool) == Some(true) {
        Ok(v.clone())
    } else {
        Err(SlackError::BadResponse)
    }
}

/// Reduce a Slack `conversations.history` body to `[{user, text, ts}]`.
fn shape_messages(v: &Value) -> Value {
    let Some(arr) = v.get("messages").and_then(Value::as_array) else {
        return json!([]);
    };
    let out: Vec<Value> = arr
        .iter()
        .map(|m| {
            json!({
                "user": m.get("user").and_then(Value::as_str).unwrap_or_default(),
                "text": m.get("text").and_then(Value::as_str).unwrap_or_default(),
                "ts": m.get("ts").and_then(Value::as_str).unwrap_or_default(),
            })
        })
        .collect();
    Value::Array(out)
}

/// Reduce a Slack `search.messages` body (`messages.matches`) to `[{text, channel, ts}]`.
fn shape_matches(v: &Value) -> Value {
    let Some(arr) = v
        .get("messages")
        .and_then(|m| m.get("matches"))
        .and_then(Value::as_array)
    else {
        return json!([]);
    };
    let out: Vec<Value> = arr
        .iter()
        .map(|m| {
            let channel = m
                .get("channel")
                .and_then(|c| c.get("id").and_then(Value::as_str).or_else(|| c.as_str()))
                .unwrap_or_default();
            json!({
                "text": m.get("text").and_then(Value::as_str).unwrap_or_default(),
                "channel": channel,
                "ts": m.get("ts").and_then(Value::as_str).unwrap_or_default(),
            })
        })
        .collect();
    Value::Array(out)
}

/// Reduce a Slack `conversations.list` body to `[{id, name}]`.
fn shape_channels(v: &Value) -> Value {
    let Some(arr) = v.get("channels").and_then(Value::as_array) else {
        return json!([]);
    };
    let out: Vec<Value> = arr
        .iter()
        .map(|c| {
            json!({
                "id": c.get("id").and_then(Value::as_str).unwrap_or_default(),
                "name": c.get("name").and_then(Value::as_str).unwrap_or_default(),
            })
        })
        .collect();
    Value::Array(out)
}

/// Percent-encode a query value (RFC-3986 unreserved set passes through).
fn urlencode(s: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                out.push('%');
                out.push(HEX[(b >> 4) as usize] as char);
                out.push(HEX[(b & 0x0f) as usize] as char);
            }
        }
    }
    out
}

/// GET a URL with the bot token; returns the response body as a string.
fn http_get(url: &str, token: &str) -> Result<String, SlackError> {
    ureq::get(url)
        .set("Authorization", &format!("Bearer {token}"))
        .call()
        .map_err(classify)?
        .into_string()
        .map_err(|e| SlackError::Unreachable(e.to_string()))
}

/// POST a JSON body with the bot token; returns the response body as a string.
fn http_post_json(url: &str, token: &str, body: &Value) -> Result<String, SlackError> {
    let payload = serde_json::to_string(body).map_err(|_| SlackError::BadResponse)?;
    ureq::post(url)
        .set("Authorization", &format!("Bearer {token}"))
        .set("Content-Type", "application/json; charset=utf-8")
        .send_string(&payload)
        .map_err(classify)?
        .into_string()
        .map_err(|e| SlackError::Unreachable(e.to_string()))
}

fn parse(body: &str) -> Result<Value, SlackError> {
    serde_json::from_str::<Value>(body).map_err(|_| SlackError::BadResponse)
}

fn stringify(v: &Value) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "{}".to_string())
}

fn classify(err: ureq::Error) -> SlackError {
    match err {
        ureq::Error::Status(code, _) => SlackError::Status(code),
        ureq::Error::Transport(t) => SlackError::Unreachable(t.to_string()),
    }
}

/// Deterministic canned responses for offline mode (no network, no credential).
mod fake {
    /// A fake posted-message result, echoing the (non-secret) channel for test assertions.
    pub(super) fn post(channel_id: &str) -> String {
        let ch = serde_json::to_string(channel_id).unwrap_or_else(|_| "\"\"".to_string());
        format!(r#"{{"ts":"1600000000.000100","channel":{ch}}}"#)
    }

    /// A fake one-message history.
    pub(super) fn read() -> String {
        r#"[{"user":"fake-user","text":"a fake message","ts":"1600000000.000100"}]"#.to_string()
    }

    /// A fake single search hit.
    pub(super) fn search() -> String {
        r#"[{"text":"a fake match","channel":"C0FAKE","ts":"1600000000.000200"}]"#.to_string()
    }

    /// A fake channel list.
    pub(super) fn list() -> String {
        r#"[{"id":"C0FAKE","name":"general"}]"#.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        require_ok, shape_channels, shape_matches, shape_messages, urlencode, Mode, SlackClient,
    };
    use serde_json::json;

    #[test]
    fn urlencode_escapes_spaces_and_specials() {
        assert_eq!(urlencode("in:general budget"), "in%3Ageneral%20budget");
        assert_eq!(urlencode("plain-text_1.0~"), "plain-text_1.0~");
    }

    #[test]
    fn require_ok_rejects_ok_false() {
        assert!(require_ok(&json!({"ok": false, "error": "channel_not_found"})).is_err());
        assert!(require_ok(&json!({"messages": []})).is_err());
        assert!(require_ok(&json!({"ok": true, "messages": []})).is_ok());
    }

    #[test]
    fn shape_messages_pulls_user_text_ts() {
        let raw = json!({
            "ok": true,
            "messages": [{ "user": "U1", "text": "hi there", "ts": "1600000000.000100" }]
        });
        let shaped = shape_messages(&raw);
        let first = &shaped.as_array().unwrap()[0];
        assert_eq!(first.get("user").unwrap(), "U1");
        assert_eq!(first.get("text").unwrap(), "hi there");
    }

    #[test]
    fn shape_matches_pulls_text_channel_ts() {
        let raw = json!({
            "ok": true,
            "messages": { "matches": [
                { "text": "found it", "channel": {"id": "C1"}, "ts": "1600000000.000200" }
            ]}
        });
        let shaped = shape_matches(&raw);
        let first = &shaped.as_array().unwrap()[0];
        assert_eq!(first.get("text").unwrap(), "found it");
        assert_eq!(first.get("channel").unwrap(), "C1");
    }

    #[test]
    fn shape_channels_pulls_id_and_name() {
        let raw = json!({ "ok": true, "channels": [{ "id": "C1", "name": "general" }] });
        let shaped = shape_channels(&raw);
        let first = &shaped.as_array().unwrap()[0];
        assert_eq!(first.get("id").unwrap(), "C1");
        assert_eq!(first.get("name").unwrap(), "general");
    }

    #[test]
    fn live_without_credential_fails_closed() {
        let client = SlackClient {
            mode: Mode::Live(None),
        };
        let err = client.post_message("C0123ABCD", "hi").unwrap_err();
        assert!(matches!(err, super::SlackError::NoCredential));
    }

    #[test]
    fn bad_channel_id_fails_before_any_network() {
        // A channel id with a path separator is rejected by the guard regardless of mode.
        let client = SlackClient {
            mode: Mode::Live(None),
        };
        let err = client.read_channel("../secrets", 5).unwrap_err();
        assert!(matches!(err, super::SlackError::BadArgs(_)));
    }

    #[test]
    fn fake_env_truthiness() {
        std::env::set_var(super::FAKE_ENV, "1");
        assert!(super::fake_enabled());
        std::env::set_var(super::FAKE_ENV, "false");
        assert!(!super::fake_enabled());
        std::env::remove_var(super::FAKE_ENV);
        assert!(!super::fake_enabled());
    }
}
