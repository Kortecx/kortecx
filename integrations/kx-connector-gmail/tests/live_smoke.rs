// SPDX-License-Identifier: Apache-2.0
//! Live witness (GR15) — `#[ignore]` by default because it needs a REAL Gmail OAuth
//! credential and network access, neither of which belong in CI.
//!
//! To run it, export a real credential and drop offline mode, then:
//! ```text
//! export KX_GMAIL_CREDENTIAL='{"client_id":"...","client_secret":"...","refresh_token":"..."}'
//! unset KX_GMAIL_FAKE
//! cargo test -p kx-connector-gmail --test live_smoke -- --ignored --nocapture
//! ```
//! The full agentic witness (a live Gemma ReAct loop firing `gmail/search` then
//! `gmail/draft` on BOTH engines) is run at registration time via a `kx serve`
//! end-to-end, per GR24 — this smoke test just proves the connector reaches Gmail.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use kx_connector_gmail::GmailClient;

#[test]
#[ignore = "requires a real Gmail OAuth credential in KX_GMAIL_CREDENTIAL + network"]
fn live_search_reaches_gmail() {
    // Build straight from the environment (real credential, offline mode NOT set).
    let client = GmailClient::from_env();
    let out = client
        .search("in:inbox", 1)
        .expect("a live Gmail search should succeed with a valid credential");
    // A real response carries the messages envelope; the secret is never echoed.
    assert!(out.contains("messages"), "unexpected search shape: {out}");
    assert!(
        !out.contains("refresh_token"),
        "the credential must never appear in a tool result"
    );
}
