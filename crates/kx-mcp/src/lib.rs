#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::doc_markdown
)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! # kx-mcp — the MCP capability adapter (M5.2, D80)
//!
//! [`McpCapability`] is the **first production** [`kx_capability::Capability`]
//! impl on the live path — the concrete dispatch for `kx_tool_registry::ToolKind::Mcp`.
//! Until now every `Capability` impl in the workspace was a test fixture; the seam
//! shipped without a body. This crate gives the runtime its first real,
//! model-selectable, world-touching tool.
//!
//! ## Why a thin, synchronous, hand-rolled client
//!
//! [`kx_capability::Capability::invoke`] is **synchronous**. The workspace's only
//! HTTP client (`ureq`) and JSON codec (`serde_json`) are synchronous too, and
//! `tokio` is quarantined to the distribution layer. Pulling the async `rmcp` SDK
//! would force a `block_on` inside `invoke` (a nested-runtime hazard under the
//! distributed worker's tokio) and a heavy tokio+hyper+reqwest tree that strains
//! `cargo-deny` + the minimalism rule. So this adapter hand-rolls a minimal MCP
//! JSON-RPC client over the existing sync stack:
//!
//! - **M5.2a:** a newline-delimited JSON-RPC `tools/call` over a [`StdioTransport`]
//!   subprocess — no network, no TLS, CI-friendly.
//! - **M5.2b (this crate now):** an `ureq` HTTP [`HttpTransport`] + an
//!   application-layer **egress sandbox** ([`EgressPolicy`]): host-allowlist binding,
//!   an SSRF/DNS-rebind vetting resolver (refuses the `169.254.169.254` metadata IP +
//!   private/loopback targets reached via a public name), `redirects(0)` refusal, and
//!   a hard wall-clock watchdog. OS-level egress isolation (`bwrap`/nftables) stays a
//!   `kx-cloud` concern (D94) — see [`EgressPolicy`] for the honest boundary.
//!
//! The transport is a trait ([`McpTransport`]) so the HTTP impl drops in without
//! touching [`McpCapability`].
//!
//! ## Security posture (MCP is UNTRUSTED)
//!
//! - **Fail-closed inbound decode (IMP-5/IMP-16):** [`decode::decode_tool_result`]
//!   is total + panic-free over arbitrary / truncated bytes, size-capped, and
//!   refuses anything that is not a well-formed JSON-RPC `tools/call` result.
//! - **Effects are world-mutating by default → `StageThenCommit` (D66):**
//!   [`McpCapability`]'s `supported_patterns` is exactly `[StageThenCommit]`.
//! - **Provenance, not hard-integrity (D72):** MCP output is external data; the
//!   broker stages it with the capability identity as its provenance record.
//! - **Secrets out-of-band, never embedded (D81):** credentials are referenced by
//!   [`CredentialRef`] and injected into the transport at dispatch time — never
//!   placed in an `EffectRequest`, a `BrokerHandle`, the journal, a `MoteId`, or a
//!   `StepRecord`.
//!
//! The warrant gate (net_scope ⊆ warrant, tool_grants, pattern) is enforced by the
//! existing `kx_capability::LocalCapabilityBroker::precheck` — this crate adds a
//! `Capability` body, never a second gate.

mod capability;
mod credential;
mod decode;
mod egress;
mod errors;
mod http_transport;
mod jsonrpc;
mod secret_store;
mod transport;

pub use capability::McpCapability;
pub use credential::CredentialRef;
pub use decode::{decode_tool_result, MAX_TOOL_RESULT_BYTES_DEFAULT};
pub use egress::{classify_ip, vet_resolved_addr, EgressDenied, EgressPolicy, IpClass};
pub use errors::{DecodeError, TransportError};
pub use http_transport::HttpTransport;
pub use secret_store::{EnvSecretStore, SecretStore};
pub use transport::{McpTransport, StdioTransport};
// Re-export the warrant's `SecretRef` (the scope identifier) so callers can build
// a `CredentialRef::from_secret_ref` without a direct `kx-warrant` dependency.
pub use kx_warrant::SecretRef;
