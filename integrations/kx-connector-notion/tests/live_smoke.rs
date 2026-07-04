// SPDX-License-Identifier: Apache-2.0
//! Live witness (GR15) — `#[ignore]` by default because it needs a REAL Notion
//! integration token and network access, neither of which belong in CI.
//!
//! To run it, export a real credential and drop offline mode, then:
//! ```text
//! export KX_NOTION_CREDENTIAL='{"token":"secret_…"}'
//! export KX_NOTION_TEST_PAGE_ID='…'   # a page the integration is shared with
//! unset KX_NOTION_FAKE
//! cargo test -p kx-connector-notion --test live_smoke -- --ignored --nocapture
//! ```
//! The full agentic witness (a live Gemma ReAct loop firing `notion/read_page` then
//! `notion/append_block` on BOTH engines) is run at registration time via a
//! `kx serve` end-to-end, per GR24 — this smoke test just proves the connector
//! reaches Notion.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use kx_connector_notion::NotionClient;

#[test]
#[ignore = "requires a real Notion integration token in KX_NOTION_CREDENTIAL + KX_NOTION_TEST_PAGE_ID + network"]
fn live_read_page_reaches_notion() {
    let page_id = std::env::var("KX_NOTION_TEST_PAGE_ID")
        .expect("set KX_NOTION_TEST_PAGE_ID to a page the integration is shared with");
    // Build straight from the environment (real credential, offline mode NOT set).
    let client = NotionClient::from_env();
    let out = client
        .read_page(&page_id)
        .expect("a live read_page should succeed with a valid integration token");
    // A real response is a JSON object with the page id; the secret is never echoed.
    assert!(out.starts_with('{'), "unexpected page shape: {out}");
    assert!(
        !out.contains("token") && !out.contains("secret_"),
        "the credential must never appear in a tool result"
    );
}
