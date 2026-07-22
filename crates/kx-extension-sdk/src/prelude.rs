// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! The curated connector-author surface: `use kx_extension_sdk::prelude::*;`.
//!
//! This is the **supported, semver-pinned** set of seams a connector author needs.
//! It is the ONLY re-export module; `tests/api_surface.rs` pins it exactly (a
//! rename upstream fails the build here; an add/remove forces a reviewed edit to
//! the test's `EXPECTED` set). Each symbol is sourced from ONE crate (e.g.
//! `SecretRef`/`NetScope`/`FsScope`/`ResourceCeiling` ship from `kx-warrant`, their
//! origin, even though `kx-mcp`/`kx-tool-registry` also re-export them) so there is
//! never a duplicate name. The HOST admin seams are NOT here — they live behind the
//! opt-in `gateway-admin` feature (see the crate root).

// -- The connector dial path (kx-mcp-gateway) ----------------------------------
// Register / discover / govern an external MCP server. `McpGateway::register_server`
// is the one entry the conformance harness drives; `CapabilitySink` is how a host
// wires a discovered tool onto its live broker.
pub use kx_mcp_gateway::{
    connection_id_of, CapabilitySink, Connection, ConnectionHealth, GatewayError, McpGateway,
    RegisterOutcome, SessionMode, SqliteConnectionStore, TransportSpec,
};

// -- Transports, secrets, egress, the capability body (kx-mcp) ------------------
// The stdio/HTTP transports an author dials over, the secret-by-ref types (D81),
// the egress-vetting helpers (SSRF/DNS-rebind), and the live capability impls.
pub use kx_mcp::{
    classify_ip, vet_resolved_addr, CredentialRef, DecodeError, EgressDenied, EgressPolicy,
    EnvSecretStore, HttpTransport, IpClass, McpCapability, McpSession, McpSessionCapability,
    McpTransport, RemoteToolDecl, SecretStore, SessionError, StdioTransport, TransportError,
};

// -- The tool vocabulary + the durable registry (kx-tool-registry) --------------
// A tool's full spec (`ToolDef`), its dispatch discriminant (`ToolKind`), the typed
// input-schema vocabulary, the registry seam, and registration lifecycle types.
pub use kx_tool_registry::{
    IdempotencyClass, InMemoryToolRegistry, InputSchema, McpEndpointId, ParamSpec, ParamType,
    RegisteredEntry, RegistrationError, RegistrationStatus, ResolutionError, SqliteToolRegistry,
    ToolDef, ToolKind, ToolProvenance, ToolRegistry,
};

// -- The capability boundary a connector's tools run under (kx-warrant) ----------
// The warrant a tool fires under, its narrowable scopes + ceilings, the per-tool
// requirement, and `check_tool_requirement` (tool.required_capability ⊆ warrant).
pub use kx_warrant::{
    check_tool_requirement, CostCeiling, FsScope, Host, ModelRoute, NetScope, ResourceCeiling,
    SecretRef, SecretScope, ToolGrant, ToolRequirement, WarrantSpec,
};

// -- Tool identity (kx-mote) ----------------------------------------------------
// The `(ToolName, ToolVersion)` a warrant grant references.
pub use kx_mote::{ToolName, ToolVersion};
