// SPDX-License-Identifier: Apache-2.0
//! `kx-connector-gmail` — a bundled Gmail MCP connector.
//!
//! A standalone Model Context Protocol server: newline-delimited JSON-RPC 2.0 over
//! stdio, speaking the full `initialize` -> `tools/list` -> `tools/call` lifecycle
//! that `kx-mcp-gateway`'s `register_server` dials. It exposes four tools that
//! let an agent consume / generate / publish Gmail:
//!   - `search` — find messages with a Gmail query,
//!   - `read`   — read one message (headers, snippet, plain-text body),
//!   - `draft`  — create a draft (does not send),
//!   - `send`   — send an email.
//!
//! ## Credential discipline (D81 — secret-by-reference)
//! The connector authenticates with an OAuth refresh-token credential supplied
//! **out-of-band**: the runtime resolves the connection's `credential_ref` NAME
//! against the caller's own secret store and injects the value into this process's
//! environment (the `KX_GMAIL_CREDENTIAL` variable). The connector reads it, does
//! the refresh -> access-token exchange **inside this process**, and calls the Gmail
//! REST API. The secret value is never placed in a reply, a log line, or an error —
//! so it never reaches the runtime's journal, a `MoteId`, or a staged effect.
//!
//! ## Offline mode (tests / CI / conformance)
//! When `KX_GMAIL_FAKE` is set the connector runs fully offline with deterministic
//! canned responses (no network, no credential), so the MCP protocol, argument
//! validation, and the secret-never-echoed contract can be gated without live
//! Gmail credentials. See [`gmail::GmailClient::from_env`].
//!
//! This crate is an **external process** the runtime dials — it has no dependency on
//! the gateway, the journal, or the frozen trio, so building or running it cannot
//! move the projection digest or perturb the core.

// Natural for a single-provider connector crate: the provider name recurs in the
// `GmailClient` / `GmailError` types. Re-exported at the crate root for callers.
#![allow(clippy::module_name_repetitions)]
// Test modules may unwrap/expect on known-good literals + relax pedantic style
// (per the workspace convention for tests).
#![cfg_attr(
    test,
    allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)
)]

pub mod gmail;
pub mod mcp;
pub mod message;
pub mod tools;

pub use gmail::{GmailClient, GmailError};
pub use mcp::handle_line;
