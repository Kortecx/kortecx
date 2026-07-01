// SPDX-License-Identifier: Apache-2.0
//! `kx-connector-discord` — a bundled Discord MCP connector.
//!
//! A standalone Model Context Protocol server: newline-delimited JSON-RPC 2.0 over
//! stdio, speaking the full `initialize` -> `tools/list` -> `tools/call` lifecycle
//! that `kx-mcp-gateway`'s `register_server` dials. It exposes three tools that let
//! an agent consume / generate / publish Discord:
//!   - `send_message`  — post a message to a channel (publish),
//!   - `read_channel`  — read recent messages from a channel (consume),
//!   - `list_channels` — list a guild's channels (consume).
//!
//! ## Credential discipline (D81 — secret-by-reference)
//! The connector authenticates with a Discord **bot token** supplied
//! **out-of-band**: the runtime resolves the connection's `credential_ref` NAME
//! against the caller's own secret store and injects the value into this process's
//! environment (the `KX_DISCORD_CREDENTIAL` variable, a JSON object holding
//! `bot_token`). The connector reads it and calls the Discord REST API with an
//! `Authorization: Bot <token>` header. The secret value is never placed in a
//! reply, a log line, or an error — so it never reaches the runtime's journal, a
//! `MoteId`, or a staged effect.
//!
//! ## Offline mode (tests / CI / conformance)
//! When `KX_DISCORD_FAKE` is set the connector runs fully offline with deterministic
//! canned responses (no network, no credential), so the MCP protocol, argument
//! validation, and the secret-never-echoed contract can be gated without a live
//! Discord bot token. See [`discord::DiscordClient::from_env`].
//!
//! This crate is an **external process** the runtime dials — it has no dependency on
//! the gateway, the journal, or the frozen trio, so building or running it cannot
//! move the projection digest or perturb the core.

// Natural for a single-provider connector crate: the provider name recurs in the
// `DiscordClient` / `DiscordError` types. Re-exported at the crate root for callers.
#![allow(clippy::module_name_repetitions)]
// Test modules may unwrap/expect on known-good literals + relax pedantic style
// (per the workspace convention for tests).
#![cfg_attr(
    test,
    allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)
)]

pub mod discord;
pub mod mcp;
pub mod tools;
pub mod validate;

pub use discord::{DiscordClient, DiscordError};
pub use mcp::handle_line;
