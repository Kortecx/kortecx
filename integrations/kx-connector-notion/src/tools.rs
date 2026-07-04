// SPDX-License-Identifier: Apache-2.0
//! The Notion tool catalog (the `tools/list` payload) and `tools/call` routing.
//!
//! Argument decoding is fail-closed: a missing/ill-typed field yields a JSON-RPC
//! invalid-params error (`-32602`), never a fabricated call. A Notion execution
//! failure yields a server error (`-32000`) carrying the typed reason — never the
//! credential.

use serde::Deserialize;

use crate::mcp::{error, result, Req};
use crate::notion::{NotionClient, NotionError};

/// The `tools/list` array: the four Notion tools with typed JSON-Schema inputs.
#[must_use]
pub fn catalog_json() -> &'static str {
    concat!(
        r#"[{"name":"search","description":"Search pages and databases the integration can access. Returns [{id,object,url}].","inputSchema":{"type":"object","properties":{"query":{"type":"string"}},"required":["query"]}},"#,
        r#"{"name":"read_page","description":"Read a Notion page's metadata. Returns {id,url,created_time,last_edited_time}.","inputSchema":{"type":"object","properties":{"page_id":{"type":"string"}},"required":["page_id"]}},"#,
        r#"{"name":"create_page","description":"Create a page under a parent page. Returns {page_id,url}.","inputSchema":{"type":"object","properties":{"parent_id":{"type":"string"},"title":{"type":"string"}},"required":["parent_id","title"]}},"#,
        r#"{"name":"append_block","description":"Append a paragraph block to a page. Returns [{id,type}].","inputSchema":{"type":"object","properties":{"page_id":{"type":"string"},"text":{"type":"string"}},"required":["page_id","text"]}}]"#
    )
}

#[derive(Deserialize)]
struct SearchArgs {
    query: String,
}

#[derive(Deserialize)]
struct ReadArgs {
    page_id: String,
}

#[derive(Deserialize)]
struct CreateArgs {
    parent_id: String,
    title: String,
}

#[derive(Deserialize)]
struct AppendArgs {
    page_id: String,
    text: String,
}

/// Route a `tools/call` request to the Notion client and frame the reply.
///
/// Unknown tool or bad arguments -> invalid-params (`-32602`); a Notion failure ->
/// server error (`-32000`); success -> a JSON-RPC `result` object. Never returns
/// the credential in any branch.
#[must_use]
pub fn call(req: &Req, client: &NotionClient) -> String {
    let id = req.id;
    let params = req.params.as_ref();
    let name = params.and_then(|p| p.name.as_deref()).unwrap_or_default();
    let args_raw = params
        .and_then(|p| p.arguments.as_ref())
        .map_or_else(|| "{}".to_string(), |a| a.get().to_string());

    match name {
        "search" => match decode::<SearchArgs>(&args_raw) {
            Ok(a) => finish(id, client.search(&a.query)),
            Err(e) => error(id, -32602, &e),
        },
        "read_page" => match decode::<ReadArgs>(&args_raw) {
            Ok(a) => finish(id, client.read_page(&a.page_id)),
            Err(e) => error(id, -32602, &e),
        },
        "create_page" => match decode::<CreateArgs>(&args_raw) {
            Ok(a) => finish(id, client.create_page(&a.parent_id, &a.title)),
            Err(e) => error(id, -32602, &e),
        },
        "append_block" => match decode::<AppendArgs>(&args_raw) {
            Ok(a) => finish(id, client.append_block(&a.page_id, &a.text)),
            Err(e) => error(id, -32602, &e),
        },
        other => error(id, -32602, &format!("no such tool: {other}")),
    }
}

/// Frame a Notion result (or its typed error) as a JSON-RPC reply.
fn finish(id: u64, outcome: Result<String, NotionError>) -> String {
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
    use crate::notion::NotionClient;

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
        call(&req, &NotionClient::fake())
    }

    #[test]
    fn search_fake_returns_results() {
        let reply = call_tool("search", r#"{"query":"roadmap"}"#);
        assert!(reply.contains(r#""result""#));
        assert!(reply.contains("fake-page-1"));
    }

    #[test]
    fn read_page_fake_returns_metadata_and_echoes_id() {
        let reply = call_tool(
            "read_page",
            r#"{"page_id":"0123456789abcdef0123456789abcdef"}"#,
        );
        assert!(reply.contains(r#""result""#));
        assert!(reply.contains("0123456789abcdef0123456789abcdef"));
    }

    #[test]
    fn create_page_fake_returns_page_id() {
        let reply = call_tool(
            "create_page",
            r#"{"parent_id":"0123456789abcdef0123456789abcdef","title":"Notes"}"#,
        );
        assert!(reply.contains(r#""result""#));
        assert!(reply.contains("page_id"));
    }

    #[test]
    fn append_block_fake_returns_blocks() {
        let reply = call_tool(
            "append_block",
            r#"{"page_id":"0123456789abcdef0123456789abcdef","text":"a line"}"#,
        );
        assert!(reply.contains(r#""result""#));
        assert!(reply.contains("fake-block-1"));
    }

    #[test]
    fn missing_required_arg_is_invalid_params() {
        let reply = call_tool(
            "create_page",
            r#"{"parent_id":"0123456789abcdef0123456789abcdef"}"#,
        );
        assert!(reply.contains("-32602"));
    }

    #[test]
    fn unknown_tool_is_invalid_params() {
        let reply = call_tool("delete_everything", r#"{}"#);
        assert!(reply.contains("-32602"));
        assert!(reply.contains("no such tool"));
    }
}
