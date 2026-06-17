#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::doc_markdown
)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! # kx-mcp-gateway — the external MCP gateway (PR-6b-1, D159)
//!
//! A thin, SYNCHRONOUS multi-server orchestrator over [`kx_mcp`]. It DIALS
//! external MCP servers (stdio + Streamable-HTTP, incl. Py/TS-SDK-exposed
//! gateways) over `kx-mcp`'s session seam, DISCOVERS their tools (`tools/list`),
//! REGISTERS each into the durable [`kx_tool_registry`] (and on the broker, so
//! the call path is live), and GOVERNS them via an off-journal, rebuildable
//! `connections.db` sidecar.
//!
//! ## Security posture (the live untrusted-egress surface, GR8)
//!
//! - **Two-gate egress:** admission host vetting (deny-by-default; the same
//!   `kx_mcp::egress::classify_ip` policy) + dial-time SSRF/DNS-rebind vetting
//!   (already enforced inside the `kx-mcp` transports' `VettingResolver`).
//! - **Per-server rate-limit:** an integer token bucket per server name.
//! - **Warrant-gated egress:** each registered tool's `net_scope` is egress to
//!   ONLY its server's host; the broker's `precheck` enforces a fired tool's
//!   `net_scope ⊆ warrant.net_scope` (SN-8 — the model fires only granted tools).
//! - **Secret-less credentials (D81):** a connection stores the credential ref
//!   NAME only; the secret is read transiently at dial and never journaled.
//! - **Server-derived ids (SN-8):** `connection_id` + the discovered tool ids are
//!   derived server-side; the client never forges them.
//!
//! Off the digest/journal path: `connections.db` is rebuildable-to-empty and is
//! NEVER a `MoteId`/journal/digest input.

mod connection;
mod errors;
mod gateway;
mod ratelimit;
mod store;

pub use connection::{connection_id_of, Connection, ConnectionHealth, TransportSpec};
pub use errors::GatewayError;
pub use gateway::{CapabilitySink, McpGateway, RegisterOutcome};
pub use ratelimit::RateLimiter;
pub use store::SqliteConnectionStore;
