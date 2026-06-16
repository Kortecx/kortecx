//! The declarative-tools registry admin seam (PR-6a — `RegisterTool` /
//! `DeregisterTool` / `DiscoverTools`).
//!
//! Spoken entirely in gateway-core's OWN wire vocabulary (`String` / `[u8; N]` /
//! `bool`) — no host type (no `kx_tool_registry::ToolDef`) crosses the seam, the
//! [`crate::AlertView`] / [`crate::ToolScoutView`] pattern. The host
//! (`kx-gateway`) implements it over its durable `Arc<SqliteToolRegistry>`
//! (`tools.db`) + the admission-time SSRF vetting of `server_host`.
//!
//! # Boundaries (load-bearing — SN-8 / GR8 / GR19)
//!
//! - **Off the truth path.** `tools.db` is off-journal, off-digest: a server-
//!   derived `tool_id` (`registration_token_of(def, prov)[..16]`), never a
//!   `MoteId`/digest input, never a journal write. Built-ins re-seed on open ⇒
//!   the canonical digest is invariant by construction.
//! - **Registration grants NO authority.** A registered tool fires only when a
//!   SERVER-issued warrant grants the exact `(name, version)` AND the broker
//!   precheck passes (the live ReAct loop). The client never names/forges
//!   `tool_id`, and client `tool_grants` stay refused at admission (BLOCKER #5).
//! - **OSS = the registry + view + admission-time SSRF vetting.** DIALING the
//!   external MCP server (the live remote tool round), credentialed Connections,
//!   and parallel fan-out are the PR-6b / Cloud surface (D159/D132/GR19) — the
//!   vetted `server_host` is stored here, never dialed.
//! - **`None` seam ⇒ `unimplemented`.** A gateway without the registry wired
//!   degrades forward-compatibly.

/// One registered tool, projected into gateway-core's own wire vocabulary (the
/// `DiscoverTools` inventory / governance row). `net_scope_summary` is a display
/// string; authority never rides this seam (SN-8).
#[derive(Clone, Debug)]
pub struct RegisteredToolEntry {
    /// 16-byte server-derived id (`registration_token_of(def, provenance)[..16]`).
    pub tool_id: [u8; 16],
    /// Identity half — the grant-set key.
    pub tool_name: String,
    /// Identity half — pinned version.
    pub tool_version: String,
    /// `"Builtin"` | `"Mcp"` (display).
    pub kind: String,
    /// Free-form description (NEVER parsed for enforcement).
    pub description: String,
    /// `"Token"` | `"Readback"` | `"Staged"` | `"AtLeastOnce"`.
    pub idempotency_class: String,
    /// `"HumanAuthored"` | `"SelfGenerated"`.
    pub provenance: String,
    /// `"Approved"` | `"PendingHumanReview"`.
    pub registration_status: String,
    /// The vetted egress endpoint the PR-6b gateway will dial (empty = no egress).
    pub server_host: String,
    /// Display summary: `"none"` | `"egress:host[,host]"`.
    pub net_scope_summary: String,
    /// `true` for server-built tools (re-seeded on open; NOT deregisterable).
    pub is_builtin: bool,
}

/// One declared, typed tool parameter (the MCP `inputSchema` analogue — CLOSED
/// set, no float, SN-8). `ty`: `"str"` | `"bytes"` | `"int"` | `"bool"` | `"enum"`.
#[derive(Clone, Debug)]
pub struct ToolParamWire {
    /// The argument key.
    pub name: String,
    /// The declared type discriminant (closed vocabulary).
    pub ty: String,
    /// Byte cap for `str`/`bytes` (ignored otherwise).
    pub max_len: u32,
    /// Whether the argument must be present.
    pub required: bool,
    /// The permitted exact values for `enum`.
    pub allowed: Vec<String>,
}

/// A registration's declared typed parameter contract (validated fail-closed
/// before any dispatch). `None` ⇒ no validation.
#[derive(Clone, Debug)]
pub struct ToolSchemaWire {
    /// The declared parameters (canonical order — part of the tool identity).
    pub params: Vec<ToolParamWire>,
    /// Refuse keys not in `params` (fail-closed against smuggled fields).
    pub deny_unknown: bool,
}

/// A `RegisterTool` request, in gateway-core vocabulary. The server derives
/// identity + capability from these (the client never supplies a warrant /
/// tool_id — SN-8).
#[derive(Clone, Debug)]
pub struct ToolRegistration {
    /// Identity half.
    pub tool_name: String,
    /// Identity half.
    pub tool_version: String,
    /// Free-form description.
    pub description: String,
    /// The declared idempotency class token (validated against the closed set).
    pub idempotency_class: String,
    /// Optional typed parameter schema.
    pub input_schema: Option<ToolSchemaWire>,
    /// The `host[:port]` the PR-6b gateway will dial — REQUIRED, SSRF-vetted at
    /// admission (deny-by-default; internal/link-local/metadata literals refused).
    pub server_host: String,
    /// The tool's name on the remote MCP server (default = `tool_name`).
    pub remote_name: String,
}

/// Why a [`ToolRegistryAdmin::register`] was refused.
#[derive(Debug, thiserror::Error)]
pub enum ToolAdminError {
    /// `server_host` failed SSRF / allowlist vetting (deny-by-default). Maps to
    /// `permission_denied` — the host is not a permitted egress target.
    #[error("server_host rejected: {0}")]
    HostRejected(String),
    /// A malformed / unsupported request field (unknown idempotency class, empty
    /// host, unsupported param type). Maps to `invalid_argument`.
    #[error("invalid registration: {0}")]
    InvalidArgument(String),
    /// A durable-store (tools.db) failure. Maps to `internal`.
    #[error("tool registry storage error: {0}")]
    Storage(String),
}

/// The declarative-tools registry admin seam behind `RegisterTool` /
/// `DeregisterTool` / `DiscoverTools`. The host implements it over its durable
/// `tools.db`. A `None` seam ⇒ the RPCs return `unimplemented`.
pub trait ToolRegistryAdmin: Send + Sync {
    /// Register a declarative external MCP tool. The host SSRF-vets `server_host`,
    /// derives the tool's identity + capability server-side (`HumanAuthored`,
    /// `net_scope` = egress to `server_host`), and durably stores it. Returns the
    /// 16-byte SERVER-DERIVED `tool_id`.
    ///
    /// # Errors
    /// [`ToolAdminError`] on a rejected host / invalid field / storage failure.
    fn register(&self, reg: ToolRegistration) -> Result<[u8; 16], ToolAdminError>;

    /// Deregister an operator-registered tool by exact `(name, version)`.
    /// Built-ins are refused (returns `false`). Returns `true` iff a row was
    /// removed.
    ///
    /// # Errors
    /// [`crate::error::GatewayError`] on a durable-store failure.
    fn deregister(
        &self,
        tool_name: &str,
        tool_version: &str,
    ) -> Result<bool, crate::error::GatewayError>;

    /// One deterministic `(name, version)`-ordered page of the registry, after an
    /// exclusive `(after_name, after_version)` cursor. `limit` is pre-clamped by
    /// the service. Returns `(rows, has_more)`.
    ///
    /// # Errors
    /// [`crate::error::GatewayError`] on a read / decode failure.
    fn discover(
        &self,
        limit: usize,
        after: Option<(String, String)>,
    ) -> Result<(Vec<RegisteredToolEntry>, bool), crate::error::GatewayError>;
}
