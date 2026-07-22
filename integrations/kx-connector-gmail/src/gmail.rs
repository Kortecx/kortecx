// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! The Gmail client: the OAuth token exchange + the Gmail REST calls — or a
//! deterministic offline `fake` mode for tests / CI / conformance.
//!
//! ## Credential (D81)
//! The OAuth credential arrives by-reference in the environment variable named by
//! [`CREDENTIAL_ENV`] (`KX_GMAIL_CREDENTIAL`), as a JSON object holding
//! `client_id`, `client_secret`, and `refresh_token`. It is read here, exchanged
//! for a short-lived access token against Google's token endpoint, and used to call
//! the Gmail REST API. The credential value is never returned in a tool result, a
//! log line, or an error string.

use serde::Deserialize;
use serde_json::{json, Value};

use crate::message;

/// The environment variable the operator wires as the connection's `credential_ref`.
/// It holds the OAuth credential as JSON: `{"client_id","client_secret","refresh_token"}`.
pub const CREDENTIAL_ENV: &str = "KX_GMAIL_CREDENTIAL";

/// When set to a truthy value, the connector runs OFFLINE with canned responses —
/// no network, no credential. Used by tests, CI, and the conformance harness.
pub const FAKE_ENV: &str = "KX_GMAIL_FAKE";

const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const API_BASE: &str = "https://gmail.googleapis.com/gmail/v1/users/me";

/// A typed connector error. Rendered into a fail-closed JSON-RPC error message;
/// no variant ever carries the credential value.
#[derive(Debug, thiserror::Error)]
pub enum GmailError {
    /// The tool arguments were missing or ill-typed.
    #[error("invalid arguments: {0}")]
    BadArgs(String),
    /// No usable credential was injected (the connection has no resolvable `credential_ref`).
    #[error("no Gmail credential available (set the connection's credential_ref)")]
    NoCredential,
    /// The Gmail API returned a non-success HTTP status.
    #[error("gmail api error (status {0})")]
    Status(u16),
    /// The Gmail API (or the token endpoint) could not be reached.
    #[error("gmail unreachable: {0}")]
    Unreachable(String),
    /// The Gmail API returned a body that did not match the expected shape.
    #[error("unexpected gmail response")]
    BadResponse,
}

/// The OAuth credential, parsed from [`CREDENTIAL_ENV`]. Private — never serialized back.
#[derive(Deserialize)]
struct OAuthConfig {
    client_id: String,
    client_secret: String,
    refresh_token: String,
}

/// The connector's Gmail client — an offline fake or a live OAuth client. The mode
/// is a private detail; construct via [`GmailClient::from_env`] or [`GmailClient::fake`].
pub struct GmailClient {
    mode: Mode,
}

/// Private client state — kept out of the public API so `OAuthConfig` stays internal.
enum Mode {
    Fake,
    Live(Option<OAuthConfig>),
}

impl GmailClient {
    /// Build from the process environment.
    ///
    /// `KX_GMAIL_FAKE` (truthy) selects offline mode. Otherwise the injected
    /// `KX_GMAIL_CREDENTIAL` JSON is parsed; an absent or malformed value yields a
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
            .and_then(|raw| serde_json::from_str::<OAuthConfig>(&raw).ok());
        Self {
            mode: Mode::Live(cfg),
        }
    }

    /// A client that always returns deterministic offline responses (tests / embedding).
    #[must_use]
    pub fn fake() -> Self {
        Self { mode: Mode::Fake }
    }

    /// Search the mailbox; returns `{messages:[{id,thread_id}], result_size_estimate}`.
    ///
    /// # Errors
    /// [`GmailError::NoCredential`] when live with no credential, or a `Status` /
    /// `Unreachable` / `BadResponse` error from the token exchange or the API call.
    pub fn search(&self, query: &str, max_results: u32) -> Result<String, GmailError> {
        if let Mode::Fake = self.mode {
            return Ok(fake::search(query));
        }
        let token = self.access_token()?;
        let max = max_results.clamp(1, 50);
        let url = format!(
            "{API_BASE}/messages?maxResults={max}&q={}",
            urlencode(query)
        );
        let v = parse(&http_get(&url, &token)?)?;
        let messages = v.get("messages").cloned().unwrap_or_else(|| json!([]));
        let estimate = v
            .get("resultSizeEstimate")
            .cloned()
            .unwrap_or_else(|| json!(0));
        Ok(stringify(&json!({
            "messages": normalize_ids(&messages),
            "result_size_estimate": estimate,
        })))
    }

    /// Read one message: headers, snippet, and a plain-text body when present.
    ///
    /// # Errors
    /// [`GmailError::BadArgs`] for an empty id, [`GmailError::NoCredential`], or a
    /// `Status` / `Unreachable` / `BadResponse` error from the API call.
    pub fn read(&self, message_id: &str) -> Result<String, GmailError> {
        if let Mode::Fake = self.mode {
            return Ok(fake::read(message_id));
        }
        if message_id.is_empty() {
            return Err(GmailError::BadArgs("message_id is required".to_string()));
        }
        let token = self.access_token()?;
        let url = format!("{API_BASE}/messages/{}?format=full", urlencode(message_id));
        let v = parse(&http_get(&url, &token)?)?;
        Ok(stringify(&shape_message(&v)))
    }

    /// Create a draft (does not send); returns `{draft_id, message}`.
    ///
    /// # Errors
    /// [`GmailError::BadArgs`] on header injection, [`GmailError::NoCredential`], or
    /// a `Status` / `Unreachable` / `BadResponse` error from the API call.
    pub fn draft(&self, to: &str, subject: &str, body: &str) -> Result<String, GmailError> {
        if let Mode::Fake = self.mode {
            return Ok(fake::draft());
        }
        let raw = message::build_raw(to, subject, body)?;
        let token = self.access_token()?;
        let url = format!("{API_BASE}/drafts");
        let v = parse(&http_post_json(
            &url,
            &token,
            &json!({ "message": { "raw": raw } }),
        )?)?;
        Ok(stringify(&json!({
            "draft_id": v.get("id").and_then(Value::as_str).unwrap_or_default(),
            "message": v.get("message").cloned().unwrap_or(Value::Null),
        })))
    }

    /// Send an email immediately; returns `{message_id, thread_id, label_ids}`.
    ///
    /// # Errors
    /// [`GmailError::BadArgs`] on header injection, [`GmailError::NoCredential`], or
    /// a `Status` / `Unreachable` / `BadResponse` error from the API call.
    pub fn send(&self, to: &str, subject: &str, body: &str) -> Result<String, GmailError> {
        if let Mode::Fake = self.mode {
            return Ok(fake::send());
        }
        let raw = message::build_raw(to, subject, body)?;
        let token = self.access_token()?;
        let url = format!("{API_BASE}/messages/send");
        let v = parse(&http_post_json(&url, &token, &json!({ "raw": raw }))?)?;
        Ok(stringify(&json!({
            "message_id": v.get("id").and_then(Value::as_str).unwrap_or_default(),
            "thread_id": v.get("threadId").and_then(Value::as_str).unwrap_or_default(),
            "label_ids": v.get("labelIds").cloned().unwrap_or_else(|| json!([])),
        })))
    }

    /// Exchange the refresh token for a short-lived access token (live mode only).
    fn access_token(&self) -> Result<String, GmailError> {
        match &self.mode {
            Mode::Fake => Ok("fake-access-token".to_string()),
            Mode::Live(cfg) => {
                let cfg = cfg.as_ref().ok_or(GmailError::NoCredential)?;
                let body = ureq::post(TOKEN_URL)
                    .send_form(&[
                        ("client_id", cfg.client_id.as_str()),
                        ("client_secret", cfg.client_secret.as_str()),
                        ("refresh_token", cfg.refresh_token.as_str()),
                        ("grant_type", "refresh_token"),
                    ])
                    .map_err(classify)?
                    .into_string()
                    .map_err(|e| GmailError::Unreachable(e.to_string()))?;
                parse(&body)?
                    .get("access_token")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .ok_or(GmailError::BadResponse)
            }
        }
    }
}

fn fake_enabled() -> bool {
    std::env::var(FAKE_ENV).is_ok_and(|v| {
        let v = v.to_ascii_lowercase();
        !v.is_empty() && v != "0" && v != "false"
    })
}

/// Reduce a Gmail `messages` array to `[{id, thread_id}]` (drop everything else).
fn normalize_ids(messages: &Value) -> Value {
    let Some(arr) = messages.as_array() else {
        return json!([]);
    };
    let out: Vec<Value> = arr
        .iter()
        .map(|m| {
            json!({
                "id": m.get("id").and_then(Value::as_str).unwrap_or_default(),
                "thread_id": m.get("threadId").and_then(Value::as_str).unwrap_or_default(),
            })
        })
        .collect();
    Value::Array(out)
}

/// Extract the useful fields of a full Gmail message into a flat, model-friendly object.
fn shape_message(v: &Value) -> Value {
    let payload = v.get("payload");
    let header = |name: &str| -> String {
        payload
            .and_then(|p| p.get("headers"))
            .and_then(Value::as_array)
            .and_then(|hs| {
                hs.iter().find(|h| {
                    h.get("name")
                        .and_then(Value::as_str)
                        .is_some_and(|n| n.eq_ignore_ascii_case(name))
                })
            })
            .and_then(|h| h.get("value"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    json!({
        "id": v.get("id").and_then(Value::as_str).unwrap_or_default(),
        "thread_id": v.get("threadId").and_then(Value::as_str).unwrap_or_default(),
        "from": header("From"),
        "to": header("To"),
        "subject": header("Subject"),
        "date": header("Date"),
        "snippet": v.get("snippet").and_then(Value::as_str).unwrap_or_default(),
        "body": payload.and_then(decode_body).unwrap_or_default(),
    })
}

/// Best-effort decode of a message's `text/plain` body (recursing into parts).
fn decode_body(payload: &Value) -> Option<String> {
    if let Some(data) = payload
        .get("body")
        .and_then(|b| b.get("data"))
        .and_then(Value::as_str)
    {
        if let Some(text) = b64url_decode(data).and_then(|b| String::from_utf8(b).ok()) {
            return Some(text);
        }
    }
    let parts = payload.get("parts").and_then(Value::as_array)?;
    parts
        .iter()
        .find(|p| p.get("mimeType").and_then(Value::as_str) == Some("text/plain"))
        .and_then(decode_body)
        .or_else(|| parts.iter().find_map(decode_body))
}

fn b64url_decode(s: &str) -> Option<Vec<u8>> {
    use base64::engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD};
    use base64::Engine;
    URL_SAFE
        .decode(s)
        .ok()
        .or_else(|| URL_SAFE_NO_PAD.decode(s).ok())
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

/// GET a URL with a bearer token; returns the response body as a string.
fn http_get(url: &str, token: &str) -> Result<String, GmailError> {
    ureq::get(url)
        .set("Authorization", &format!("Bearer {token}"))
        .call()
        .map_err(classify)?
        .into_string()
        .map_err(|e| GmailError::Unreachable(e.to_string()))
}

/// POST a JSON body with a bearer token; returns the response body as a string.
fn http_post_json(url: &str, token: &str, body: &Value) -> Result<String, GmailError> {
    let payload = serde_json::to_string(body).map_err(|_| GmailError::BadResponse)?;
    ureq::post(url)
        .set("Authorization", &format!("Bearer {token}"))
        .set("Content-Type", "application/json")
        .send_string(&payload)
        .map_err(classify)?
        .into_string()
        .map_err(|e| GmailError::Unreachable(e.to_string()))
}

fn parse(body: &str) -> Result<Value, GmailError> {
    serde_json::from_str::<Value>(body).map_err(|_| GmailError::BadResponse)
}

fn stringify(v: &Value) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "{}".to_string())
}

fn classify(err: ureq::Error) -> GmailError {
    match err {
        ureq::Error::Status(code, _) => GmailError::Status(code),
        ureq::Error::Transport(t) => GmailError::Unreachable(t.to_string()),
    }
}

/// Deterministic canned responses for offline mode (no network, no credential).
mod fake {
    /// A search hit list that echoes the (non-secret) query for test assertions.
    pub(super) fn search(query: &str) -> String {
        let q = serde_json::to_string(query).unwrap_or_else(|_| "\"\"".to_string());
        format!(
            r#"{{"messages":[{{"id":"fake-msg-1","thread_id":"fake-thread-1"}}],"result_size_estimate":1,"echoed_query":{q}}}"#
        )
    }

    /// A single fake message.
    pub(super) fn read(id: &str) -> String {
        let id = serde_json::to_string(id).unwrap_or_else(|_| "\"fake\"".to_string());
        format!(
            r#"{{"id":{id},"thread_id":"fake-thread-1","from":"alice@example.com","to":"me@example.com","subject":"Hello","date":"Tue, 01 Jul 2026 10:00:00 +0000","snippet":"a fake message","body":"This is a fake message body."}}"#
        )
    }

    /// A fake draft id.
    pub(super) fn draft() -> String {
        r#"{"draft_id":"fake-draft-1","message":{"id":"fake-msg-2"}}"#.to_string()
    }

    /// A fake sent-message id.
    pub(super) fn send() -> String {
        r#"{"message_id":"fake-msg-3","thread_id":"fake-thread-1","label_ids":["SENT"]}"#
            .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::{fake_enabled, shape_message, urlencode, GmailClient, Mode};
    use serde_json::json;

    #[test]
    fn urlencode_escapes_spaces_and_specials() {
        assert_eq!(
            urlencode("is:unread from:a b"),
            "is%3Aunread%20from%3Aa%20b"
        );
        assert_eq!(urlencode("plain-text_1.0~"), "plain-text_1.0~");
    }

    #[test]
    fn shape_message_pulls_headers_snippet_and_body() {
        let raw = json!({
            "id": "m1", "threadId": "t1", "snippet": "hi there",
            "payload": {
                "headers": [
                    {"name": "From", "value": "a@b.com"},
                    {"name": "Subject", "value": "Hello"},
                ],
                "body": {"data": "SGVsbG8gYm9keQ=="}
            }
        });
        let shaped = shape_message(&raw);
        assert_eq!(shaped.get("from").unwrap(), "a@b.com");
        assert_eq!(shaped.get("subject").unwrap(), "Hello");
        assert_eq!(shaped.get("snippet").unwrap(), "hi there");
        assert_eq!(shaped.get("body").unwrap(), "Hello body");
    }

    #[test]
    fn live_without_credential_fails_closed() {
        let client = GmailClient {
            mode: Mode::Live(None),
        };
        let err = client.search("q", 5).unwrap_err();
        assert!(matches!(err, super::GmailError::NoCredential));
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
