//! The external MCP gateway admin seam (PR-6b-1 — `RegisterMcpServer` /
//! `ListMcpServers` / `DiscoverServerTools` / `TestMcpServer` /
//! `DeregisterMcpServer`).
//!
//! Spoken entirely in gateway-core's OWN wire vocabulary (`String` / `[u8; N]` /
//! `bool`) — no host type (no `kx_mcp_gateway::McpGateway`) crosses the seam, the
//! [`crate::tool_registry_admin::ToolRegistryAdmin`] pattern. The host
//! (`kx-gateway`) implements it over its `McpGateway` (which dials external MCP
//! servers + registers their tools into the SAME durable `tools.db`).
//!
//! # Boundaries (SN-8 / GR8 / GR19)
//!
//! - **The live untrusted-egress surface.** Registering a server DIALS it
//!   (`initialize` -> `tools/list`); the host enforces admission + dial-time
//!   SSRF vetting, a per-server rate-limit, and warrant-gated egress.
//! - **Server-derived ids.** `connection_id` + the discovered tool ids are
//!   derived server-side; the client never forges them.
//! - **Secret-less.** A connection references its credential by NAME only; the
//!   secret value never crosses this seam, the wire, or the journal (D81).
//! - **`None` seam ⇒ `unimplemented`.** A gateway without the MCP gateway wired
//!   degrades forward-compatibly. OAuth/device-flow + a credential marketplace
//!   are CLOUD (D159/GR19).

use crate::tool_registry_admin::RegisteredToolEntry;

/// A `RegisterMcpServer` request, in gateway-core vocabulary.
#[derive(Clone, Debug)]
pub struct McpServerRegistration {
    /// Unique operator handle; namespaces the discovered tool ids (`<name>/<remote>`).
    pub server_name: String,
    /// `"stdio"` | `"http"`.
    pub transport: String,
    /// stdio: the program path; http: the endpoint URL.
    pub endpoint: String,
    /// stdio command-line args (ignored for http).
    pub args: Vec<String>,
    /// http: refuse plaintext `http://` when `true`.
    pub tls_required: bool,
    /// OPTIONAL secret-less credential ref NAME (env var / vault key) — never the secret.
    pub credential_ref: Option<String>,
}

/// One registered external MCP server, in gateway-core vocabulary (the
/// `ListMcpServers` governance row).
#[derive(Clone, Debug)]
pub struct McpServerView {
    /// 16-byte server-derived connection id.
    pub connection_id: [u8; 16],
    /// The operator handle.
    pub server_name: String,
    /// `"stdio"` | `"http"`.
    pub transport: String,
    /// The command (stdio) or URL (http).
    pub endpoint: String,
    /// `"connected"` | `"unreachable"` | `"unknown"`.
    pub health: String,
    /// Tools discovered on the last successful dial.
    pub tool_count: u32,
    /// Whether a credential ref NAME is attached (never the value, D81).
    pub credential_ref_present: bool,
}

/// The outcome of registering an MCP server.
#[derive(Clone, Debug)]
pub struct RegisterServerOutcome {
    /// 16-byte server-derived connection id.
    pub connection_id: [u8; 16],
    /// Tools discovered + registered (0 when unreachable).
    pub discovered: u32,
    /// `"connected"` | `"unreachable"` | `"unknown"`.
    pub health: String,
}

/// Why an [`McpGatewayAdmin`] operation was refused.
#[derive(Debug, thiserror::Error)]
pub enum McpAdminError {
    /// The server host failed SSRF / allowlist vetting (deny-by-default). Maps to
    /// `permission_denied`.
    #[error("server host rejected: {0}")]
    HostRejected(String),
    /// A malformed / unsupported request field. Maps to `invalid_argument`.
    #[error("invalid MCP server spec: {0}")]
    InvalidArgument(String),
    /// A live dial failed (unreachable / refused / timeout / fail-closed decode).
    /// Maps to `failed_precondition` (the server is not reachable; not the client's
    /// malformed input).
    #[error("dial failed: {0}")]
    Dial(String),
    /// The per-server rate-limit was exceeded. Maps to `resource_exhausted`.
    #[error("rate limited: {0}")]
    RateLimited(String),
    /// No server with the given name is registered. Maps to `not_found`.
    #[error("no such MCP server: {0}")]
    NotFound(String),
    /// A durable-store (connections.db / tools.db) failure. Maps to `internal`.
    #[error("storage error: {0}")]
    Storage(String),
}

/// The external MCP gateway admin seam behind the 5 PR-6b-1 RPCs. The host
/// implements it over its `McpGateway`. A `None` seam ⇒ the RPCs return
/// `unimplemented`.
pub trait McpGatewayAdmin: Send + Sync {
    /// Register an external MCP server: vet the host (HTTP), dial + discover +
    /// register its tools, persist the connection, fold health. A host that fails
    /// admission vetting is REFUSED; a dial failure is NOT fatal (the connection
    /// persists with `unreachable` health — honest, never a fabricated success).
    ///
    /// # Errors
    /// [`McpAdminError`] on host rejection / invalid spec / storage failure.
    fn register_server(
        &self,
        reg: McpServerRegistration,
    ) -> Result<RegisterServerOutcome, McpAdminError>;

    /// List all registered servers (deterministic `(name)` order).
    ///
    /// # Errors
    /// [`McpAdminError::Storage`] on a read failure.
    fn list_servers(&self) -> Result<Vec<McpServerView>, McpAdminError>;

    /// Re-dial a registered server, re-discover its tools, and return its
    /// registered tool inventory rows + the count discovered.
    ///
    /// # Errors
    /// [`McpAdminError`] (`NotFound` / `Dial` / storage).
    fn discover_server(
        &self,
        server_name: &str,
    ) -> Result<(Vec<RegisteredToolEntry>, u32), McpAdminError>;

    /// Test a server's reachability (dial + `initialize` only). Returns
    /// `(reachable, detail)`; `detail` is a short non-sensitive diagnostic.
    ///
    /// # Errors
    /// [`McpAdminError::NotFound`] if no such server.
    fn test_server(&self, server_name: &str) -> Result<(bool, String), McpAdminError>;

    /// Deregister a server + its namespaced tools. Returns `true` iff removed.
    ///
    /// # Errors
    /// [`McpAdminError::Storage`] on a durable-store failure.
    fn deregister_server(&self, server_name: &str) -> Result<bool, McpAdminError>;
}
