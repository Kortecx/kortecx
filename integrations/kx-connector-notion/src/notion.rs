// SPDX-License-Identifier: Apache-2.0
//! The Notion client: the Notion REST calls authenticated with an integration
//! token — or a deterministic offline `fake` mode for tests / CI / conformance.
//!
//! ## Credential (D81)
//! The integration token arrives by-reference in the environment variable named by
//! [`CREDENTIAL_ENV`] (`KX_NOTION_CREDENTIAL`), as a JSON object holding `token`. It
//! is read here and sent as an `Authorization: Bearer <token>` header (plus the
//! required `Notion-Version` header) to the Notion REST API. The token value is never
//! returned in a tool result, a log line, or an error string.

use serde::Deserialize;
use serde_json::{json, Value};

use crate::validate::{check_text, check_uuid};

/// The environment variable the operator wires as the connection's `credential_ref`.
/// It holds the credential as JSON: `{"token": "secret_…"}`.
pub const CREDENTIAL_ENV: &str = "KX_NOTION_CREDENTIAL";

/// When set to a truthy value, the connector runs OFFLINE with canned responses —
/// no network, no credential. Used by tests, CI, and the conformance harness.
pub const FAKE_ENV: &str = "KX_NOTION_FAKE";

const API_BASE: &str = "https://api.notion.com/v1";

/// The Notion REST API is versioned by a required request header (not the URL).
const NOTION_VERSION: &str = "2022-06-28";

/// A typed connector error. Rendered into a fail-closed JSON-RPC error message;
/// no variant ever carries the credential value.
#[derive(Debug, thiserror::Error)]
pub enum NotionError {
    /// The tool arguments were missing, ill-typed, or failed a guard.
    #[error("invalid arguments: {0}")]
    BadArgs(String),
    /// No usable credential was injected (the connection has no resolvable `credential_ref`).
    #[error("no Notion credential available (set the connection's credential_ref)")]
    NoCredential,
    /// The Notion API returned a non-success HTTP status.
    #[error("notion api error (status {0})")]
    Status(u16),
    /// The Notion API could not be reached.
    #[error("notion unreachable: {0}")]
    Unreachable(String),
    /// The Notion API returned a body that did not match the expected shape.
    #[error("unexpected notion response")]
    BadResponse,
}

/// The integration credential, parsed from [`CREDENTIAL_ENV`]. Private — never serialized back.
#[derive(Deserialize)]
struct BotConfig {
    token: String,
}

/// The connector's Notion client — an offline fake or a live client. The mode is a
/// private detail; construct via [`NotionClient::from_env`] or [`NotionClient::fake`].
pub struct NotionClient {
    mode: Mode,
}

/// Private client state — kept out of the public API so `BotConfig` stays internal.
enum Mode {
    Fake,
    Live(Option<BotConfig>),
}

impl NotionClient {
    /// Build from the process environment.
    ///
    /// `KX_NOTION_FAKE` (truthy) selects offline mode. Otherwise the injected
    /// `KX_NOTION_CREDENTIAL` JSON is parsed; an absent or malformed value yields a
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

    /// Search pages/databases the integration can see; returns `[{id, object, url}]`.
    ///
    /// # Errors
    /// [`NotionError::NoCredential`], or a `Status` / `Unreachable` / `BadResponse`
    /// error from the API call.
    pub fn search(&self, query: &str) -> Result<String, NotionError> {
        if let Mode::Fake = self.mode {
            return Ok(fake::search());
        }
        let token = self.token()?;
        let url = format!("{API_BASE}/search");
        let v = parse(&http_post_json(&url, token, &json!({ "query": query }))?)?;
        Ok(stringify(&shape_results(&v)))
    }

    /// Read a page's metadata; returns `{id, url, created_time, last_edited_time}`.
    ///
    /// # Errors
    /// [`NotionError::BadArgs`] on a bad id, [`NotionError::NoCredential`], or a
    /// `Status` / `Unreachable` / `BadResponse` error from the API call.
    pub fn read_page(&self, page_id: &str) -> Result<String, NotionError> {
        check_uuid("page_id", page_id)?;
        if let Mode::Fake = self.mode {
            return Ok(fake::read(page_id));
        }
        let token = self.token()?;
        let url = format!("{API_BASE}/pages/{page_id}");
        let v = parse(&http_get(&url, token)?)?;
        Ok(stringify(&shape_page(&v)))
    }

    /// Create a page under a parent page; returns `{page_id, url}`.
    ///
    /// # Errors
    /// [`NotionError::BadArgs`] on a bad parent id / title, [`NotionError::NoCredential`],
    /// or a `Status` / `Unreachable` / `BadResponse` error from the API call.
    pub fn create_page(&self, parent_id: &str, title: &str) -> Result<String, NotionError> {
        check_uuid("parent_id", parent_id)?;
        check_text("title", title)?;
        if let Mode::Fake = self.mode {
            return Ok(fake::create());
        }
        let token = self.token()?;
        let url = format!("{API_BASE}/pages");
        let body = json!({
            "parent": { "page_id": parent_id },
            "properties": {
                "title": { "title": [ { "text": { "content": title } } ] }
            }
        });
        let v = parse(&http_post_json(&url, token, &body)?)?;
        Ok(stringify(&json!({
            "page_id": v.get("id").and_then(Value::as_str).unwrap_or_default(),
            "url": v.get("url").and_then(Value::as_str).unwrap_or_default(),
        })))
    }

    /// Append a paragraph block to a page; returns `[{id, type}]`.
    ///
    /// # Errors
    /// [`NotionError::BadArgs`] on a bad id / text, [`NotionError::NoCredential`], or
    /// a `Status` / `Unreachable` / `BadResponse` error from the API call.
    pub fn append_block(&self, page_id: &str, text: &str) -> Result<String, NotionError> {
        check_uuid("page_id", page_id)?;
        check_text("text", text)?;
        if let Mode::Fake = self.mode {
            return Ok(fake::append());
        }
        let token = self.token()?;
        let url = format!("{API_BASE}/blocks/{page_id}/children");
        let body = json!({
            "children": [ {
                "object": "block",
                "type": "paragraph",
                "paragraph": { "rich_text": [ { "type": "text", "text": { "content": text } } ] }
            } ]
        });
        let v = parse(&http_patch_json(&url, token, &body)?)?;
        Ok(stringify(&shape_blocks(&v)))
    }

    /// The integration token (live mode only); `NoCredential` when none was injected.
    fn token(&self) -> Result<&str, NotionError> {
        match &self.mode {
            Mode::Fake => Ok("fake-token"),
            Mode::Live(cfg) => cfg
                .as_ref()
                .map(|c| c.token.as_str())
                .ok_or(NotionError::NoCredential),
        }
    }
}

fn fake_enabled() -> bool {
    std::env::var(FAKE_ENV).is_ok_and(|v| {
        let v = v.to_ascii_lowercase();
        !v.is_empty() && v != "0" && v != "false"
    })
}

/// Reduce a Notion `search` body (`results[]`) to `[{id, object, url}]`.
fn shape_results(v: &Value) -> Value {
    let Some(arr) = v.get("results").and_then(Value::as_array) else {
        return json!([]);
    };
    let out: Vec<Value> = arr
        .iter()
        .map(|r| {
            json!({
                "id": r.get("id").and_then(Value::as_str).unwrap_or_default(),
                "object": r.get("object").and_then(Value::as_str).unwrap_or_default(),
                "url": r.get("url").and_then(Value::as_str).unwrap_or_default(),
            })
        })
        .collect();
    Value::Array(out)
}

/// Reduce a Notion page object to `{id, url, created_time, last_edited_time}`.
fn shape_page(v: &Value) -> Value {
    json!({
        "id": v.get("id").and_then(Value::as_str).unwrap_or_default(),
        "url": v.get("url").and_then(Value::as_str).unwrap_or_default(),
        "created_time": v.get("created_time").and_then(Value::as_str).unwrap_or_default(),
        "last_edited_time": v.get("last_edited_time").and_then(Value::as_str).unwrap_or_default(),
    })
}

/// Reduce a Notion block-append body (`results[]`) to `[{id, type}]`.
fn shape_blocks(v: &Value) -> Value {
    let Some(arr) = v.get("results").and_then(Value::as_array) else {
        return json!([]);
    };
    let out: Vec<Value> = arr
        .iter()
        .map(|b| {
            json!({
                "id": b.get("id").and_then(Value::as_str).unwrap_or_default(),
                "type": b.get("type").and_then(Value::as_str).unwrap_or_default(),
            })
        })
        .collect();
    Value::Array(out)
}

/// GET a URL with the integration token + the required Notion-Version header.
fn http_get(url: &str, token: &str) -> Result<String, NotionError> {
    ureq::get(url)
        .set("Authorization", &format!("Bearer {token}"))
        .set("Notion-Version", NOTION_VERSION)
        .call()
        .map_err(classify)?
        .into_string()
        .map_err(|e| NotionError::Unreachable(e.to_string()))
}

/// POST a JSON body with the integration token + the required Notion-Version header.
fn http_post_json(url: &str, token: &str, body: &Value) -> Result<String, NotionError> {
    send_json("POST", url, token, body)
}

/// PATCH a JSON body with the integration token + the required Notion-Version header.
fn http_patch_json(url: &str, token: &str, body: &Value) -> Result<String, NotionError> {
    send_json("PATCH", url, token, body)
}

/// Shared JSON-body sender for the write verbs (POST / PATCH).
fn send_json(method: &str, url: &str, token: &str, body: &Value) -> Result<String, NotionError> {
    let payload = serde_json::to_string(body).map_err(|_| NotionError::BadResponse)?;
    ureq::request(method, url)
        .set("Authorization", &format!("Bearer {token}"))
        .set("Notion-Version", NOTION_VERSION)
        .set("Content-Type", "application/json")
        .send_string(&payload)
        .map_err(classify)?
        .into_string()
        .map_err(|e| NotionError::Unreachable(e.to_string()))
}

fn parse(body: &str) -> Result<Value, NotionError> {
    serde_json::from_str::<Value>(body).map_err(|_| NotionError::BadResponse)
}

fn stringify(v: &Value) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "{}".to_string())
}

fn classify(err: ureq::Error) -> NotionError {
    match err {
        ureq::Error::Status(code, _) => NotionError::Status(code),
        ureq::Error::Transport(t) => NotionError::Unreachable(t.to_string()),
    }
}

/// Deterministic canned responses for offline mode (no network, no credential).
mod fake {
    /// A single fake search hit.
    pub(super) fn search() -> String {
        r#"[{"id":"fake-page-1","object":"page","url":"https://notion.so/fake-page-1"}]"#
            .to_string()
    }

    /// A fake page metadata object, echoing the (non-secret) id for test assertions.
    pub(super) fn read(page_id: &str) -> String {
        let id = serde_json::to_string(page_id).unwrap_or_else(|_| "\"fake\"".to_string());
        format!(
            r#"{{"id":{id},"url":"https://notion.so/fake-page-1","created_time":"2026-07-01T10:00:00.000Z","last_edited_time":"2026-07-01T10:00:00.000Z"}}"#
        )
    }

    /// A fake created-page result.
    pub(super) fn create() -> String {
        r#"{"page_id":"fake-page-2","url":"https://notion.so/fake-page-2"}"#.to_string()
    }

    /// A fake appended-block result.
    pub(super) fn append() -> String {
        r#"[{"id":"fake-block-1","type":"paragraph"}]"#.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::{shape_blocks, shape_page, shape_results, Mode, NotionClient};
    use serde_json::json;

    #[test]
    fn shape_results_pulls_id_object_url() {
        let raw = json!({
            "results": [{ "id": "p1", "object": "page", "url": "https://notion.so/p1" }]
        });
        let shaped = shape_results(&raw);
        let first = &shaped.as_array().unwrap()[0];
        assert_eq!(first.get("id").unwrap(), "p1");
        assert_eq!(first.get("object").unwrap(), "page");
    }

    #[test]
    fn shape_page_pulls_metadata() {
        let raw = json!({
            "id": "p1",
            "url": "https://notion.so/p1",
            "created_time": "2026-07-01T10:00:00.000Z",
            "last_edited_time": "2026-07-02T10:00:00.000Z"
        });
        let shaped = shape_page(&raw);
        assert_eq!(shaped.get("id").unwrap(), "p1");
        assert_eq!(
            shaped.get("last_edited_time").unwrap(),
            "2026-07-02T10:00:00.000Z"
        );
    }

    #[test]
    fn shape_blocks_pulls_id_and_type() {
        let raw = json!({ "results": [{ "id": "b1", "type": "paragraph" }] });
        let shaped = shape_blocks(&raw);
        let first = &shaped.as_array().unwrap()[0];
        assert_eq!(first.get("id").unwrap(), "b1");
        assert_eq!(first.get("type").unwrap(), "paragraph");
    }

    #[test]
    fn live_without_credential_fails_closed() {
        let client = NotionClient {
            mode: Mode::Live(None),
        };
        let err = client.search("q").unwrap_err();
        assert!(matches!(err, super::NotionError::NoCredential));
    }

    #[test]
    fn bad_page_id_fails_before_any_network() {
        // A page id with a path separator is rejected by the guard regardless of mode.
        let client = NotionClient {
            mode: Mode::Live(None),
        };
        let err = client.read_page("../secrets").unwrap_err();
        assert!(matches!(err, super::NotionError::BadArgs(_)));
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
