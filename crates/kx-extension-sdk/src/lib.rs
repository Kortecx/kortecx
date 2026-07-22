// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! # kx-extension-sdk — the Connector/Extension authoring surface (D167 E0)
//!
//! This crate is the **curated, semver-pinned front door** for authoring an
//! external **connector** to a `kx serve` runtime. It adds NO new machinery: it
//! re-exports the (already-public, but scattered) seams a connector author needs
//! through one [`prelude`], documents the dial path + the security contract, and
//! ships a per-connector [`conformance`] harness so an author can prove their
//! connector is safe to register BEFORE it ever touches a live runtime.
//!
//! It is an **FFI-FREE leaf** (the `tests/dep_wall.rs` pins it): it never depends
//! on the journal writer, the gateway service, the frozen trio
//! (`kx-executor`/`kx-scheduler`/`kx-inference` dispatcher), or `kx-llamacpp`.
//! Re-exporting types adds no journal facts, so the canonical projection digest is
//! invariant by construction.
//!
//! ## What a connector is
//!
//! A **connector** is an external **MCP tool server** (an
//! [Model Context Protocol](https://modelcontextprotocol.io) server) — a separate
//! process the runtime dials over **stdio** (a subprocess) or **Streamable-HTTP**.
//! The runtime is a SECURE GATEWAY (D132): it never runs arbitrary in-process code;
//! a connector contributes *tools* the agentic loop may fire only under a warrant
//! that grants them (SN-8 — model proposes, runtime enforces).
//!
//! ## The dial path (what happens when a connector is registered)
//!
//! ```text
//! kx connections add  ──►  RegisterMcpServer RPC  ──►  McpGateway::register_server
//!   ├─ admission: deny-by-default SSRF host vetting (HTTP; stdio has no egress)
//!   ├─ dial:      open a session over the kx-mcp transport
//!   ├─ discover:  tools/list  (the connector's JSON-Schema tool manifests)
//!   ├─ register:  each tool into the durable registry as ToolKind::Mcp,
//!   │             namespaced `<server>/<remote>` (server-derived id, SN-8)
//!   └─ govern:    persist the connection in the off-journal connections.db
//! ```
//!
//! ## The security contract (every connector must honor)
//!
//! - **Out-of-process.** A connector runs as its own process; the runtime links
//!   none of its code. The conformance harness asserts the registered tool is
//!   [`ToolKind::Mcp`](crate::prelude::ToolKind), never `Builtin`.
//! - **Warrant-gated (SN-8).** A registered tool fires ONLY through a warrant that
//!   grants its `(name, version)` and whose scopes cover the tool's
//!   `required_capability`. Mere presence never fires anything.
//! - **Secrets by reference (D81).** A credential is referenced by NAME
//!   ([`SecretRef`](crate::prelude::SecretRef) /
//!   [`CredentialRef`](crate::prelude::CredentialRef)); the value is resolved
//!   transiently at dial and reaches no journal, content, MoteId, or telemetry sink.
//! - **Egress is two-gated (HTTP).** Admission host vetting + dial-time
//!   SSRF/DNS-rebind vetting; a tool's `net_scope` is egress to ONLY its host.
//!
//! ## Authoring + chaining
//!
//! A connector author writes (or wraps) an MCP server in any language. Register +
//! reach it the same way across every surface (one chaining entry point):
//!
//! ```text
//! # CLI (operator)
//! kx connections add --name fs --command "npx -y @modelcontextprotocol/server-filesystem /data"
//! kx agent run --goal "list /data" --tools fs/list_directory
//!
//! # Python / TypeScript SDK (the single chaining entry point)
//! flow().with_mcp("fs", transport="stdio", endpoint="npx", \
//!                 args=["-y", "@modelcontextprotocol/server-filesystem", "/data"]) \
//!       .agent("list /data", tools=["fs/list_directory"]).run()
//! ```
//!
//! A connector must implement the full MCP `initialize → tools/list → tools/call`
//! lifecycle so the runtime can DISCOVER its tools at registration. The SDK ships a
//! minimal, complete reference connector (`kx-connector-example`,
//! `src/bin/reference_connector.rs`) to copy from.
//!
//! Rust authors building an in-tree connector or a custom host import the seams
//! from [`prelude`]:
//!
//! ```
//! use kx_extension_sdk::prelude::*;
//! // e.g. inspect the tool vocabulary, build an InputSchema, or check that a
//! // tool's required_capability is within a warrant before granting it:
//! let _ = ToolKind::Builtin; // the dispatch discriminant
//! ```
//!
//! ## Proving a connector is safe (the conformance harness)
//!
//! ```no_run
//! use kx_extension_sdk::conformance::{run_conformance, ConnectorUnderTest};
//! use kx_extension_sdk::prelude::{SessionMode, TransportSpec};
//!
//! let cut = ConnectorUnderTest {
//!     name: "example".into(),
//!     transport: TransportSpec::Stdio { command: "./kx-connector-example".into(), args: vec![] },
//!     credential_ref: None,
//!     session_mode: SessionMode::Stateless,
//! };
//! let report = run_conformance(&cut);
//! assert!(report.passed(), "{report:#?}");
//! ```
//!
//! Or from the shell: `just test-connector ./kx-connector-example`.
//!
//! ## The `gateway-admin` feature (host authoring, off by default)
//!
//! The default surface is everything a CONNECTOR author needs. The HOST admin
//! seams — `McpGatewayAdmin` / `ToolRegistryAdmin`, which the runtime
//! (`kx-gateway`) implements, not a connector — live behind the opt-in
//! `gateway-admin` feature (it adds the `kx-gateway-core` edge → proto/tonic).
//! Enable it only when wiring a custom gateway host; see the `kx-gateway-core` crate.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::doc_markdown
)]

pub mod conformance;
pub mod prelude;
pub mod skill_conformance;

/// The HOST admin seams (`McpGatewayAdmin` / `ToolRegistryAdmin`) — the surface a
/// custom gateway host implements, NOT a connector author. Gated behind the
/// opt-in `gateway-admin` feature (it pulls `kx-gateway-core` → proto/tonic).
#[cfg(feature = "gateway-admin")]
pub mod gateway_admin {
    pub use kx_gateway_core::{McpGatewayAdmin, ToolRegistryAdmin};
}
