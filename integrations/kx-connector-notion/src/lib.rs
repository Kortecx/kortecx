// SPDX-License-Identifier: Apache-2.0
//! `kx-connector-notion` — a bundled Notion MCP connector.
//!
//! A standalone Model Context Protocol server: newline-delimited JSON-RPC 2.0 over
//! stdio, speaking the full `initialize` -> `tools/list` -> `tools/call` lifecycle
//! that `kx-mcp-gateway`'s `register_server` dials. It exposes four tools that let
//! an agent consume / generate / publish Notion:
//!   - `search`       — search pages/databases the integration can see (consume),
//!   - `read_page`    — read a page's metadata (consume),
//!   - `create_page`  — create a page under a parent page (publish),
//!   - `append_block` — append a paragraph block to a page (publish).
//!
//! ## Credential discipline (D81 — secret-by-reference)
//! The connector authenticates with a Notion **integration token** supplied
//! **out-of-band**: the runtime resolves the connection's `credential_ref` NAME
//! against the caller's own secret store and injects the value into this process's
//! environment (the `KX_NOTION_CREDENTIAL` variable, a JSON object holding `token`).
//! The connector reads it and calls the Notion REST API with an `Authorization:
//! Bearer <token>` header plus the required `Notion-Version` header. The secret value
//! is never placed in a reply, a log line, or an error — so it never reaches the
//! runtime's journal, a `MoteId`, or a staged effect.
//!
//! ## Offline mode (tests / CI / conformance)
//! When `KX_NOTION_FAKE` is set the connector runs fully offline with deterministic
//! canned responses (no network, no credential), so the MCP protocol, argument
//! validation, and the secret-never-echoed contract can be gated without a live
//! Notion token. See [`notion::NotionClient::from_env`].
//!
//! This crate is an **external process** the runtime dials — it has no dependency on
//! the gateway, the journal, or the frozen trio, so building or running it cannot
//! move the projection digest or perturb the core.

// Natural for a single-provider connector crate: the provider name recurs in the
// `NotionClient` / `NotionError` types. Re-exported at the crate root for callers.
#![allow(clippy::module_name_repetitions)]
// Test modules may unwrap/expect on known-good literals + relax pedantic style
// (per the workspace convention for tests).
#![cfg_attr(
    test,
    allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)
)]

pub mod mcp;
pub mod notion;
pub mod tools;
pub mod validate;

pub use mcp::handle_line;
pub use notion::{NotionClient, NotionError};
