// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! Live witness (GR15) — `#[ignore]` by default because it needs a REAL Slack bot
//! token and network access, neither of which belong in CI.
//!
//! To run it, export a real credential and drop offline mode, then:
//! ```text
//! export KX_SLACK_CREDENTIAL='{"bot_token":"xoxb-…"}'
//! unset KX_SLACK_FAKE
//! cargo test -p kx-connector-slack --test live_smoke -- --ignored --nocapture
//! ```
//! The full agentic witness (a live Gemma ReAct loop firing `slack/read_channel`
//! then `slack/post_message` on BOTH engines) is run at registration time via a
//! `kx serve` end-to-end, per GR24 — this smoke test just proves the connector
//! reaches Slack.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use kx_connector_slack::SlackClient;

#[test]
#[ignore = "requires a real Slack bot token in KX_SLACK_CREDENTIAL + network"]
fn live_list_channels_reaches_slack() {
    // Build straight from the environment (real credential, offline mode NOT set).
    let client = SlackClient::from_env();
    let out = client
        .list_channels()
        .expect("a live list_channels should succeed with a valid bot token");
    // A real response is a JSON array of channels; the secret is never echoed.
    assert!(out.starts_with('['), "unexpected list shape: {out}");
    assert!(
        !out.contains("bot_token") && !out.contains("xoxb-"),
        "the credential must never appear in a tool result"
    );
}
