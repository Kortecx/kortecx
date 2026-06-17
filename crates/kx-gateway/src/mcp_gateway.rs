//! PR-6b-1 host wiring for the EXTERNAL MCP gateway: the [`McpGatewayAdmin`] impl
//! over the [`kx_mcp_gateway::McpGateway`] + the [`BrokerCapabilitySink`] that
//! registers a dialed tool's firing capability on the serve broker.
//!
//! The runtime is a SECURE GATEWAY to external MCP servers (D132/D159/GR19): it
//! DIALS the server (`initialize` -> `tools/list`), registers the discovered
//! tools into the SAME durable `tools.db`, and governs the connection in an
//! off-journal `connections.db` sidecar — never an executor of arbitrary code.
//! The live untrusted-egress surface (GR8) is enforced by `kx-mcp-gateway`:
//! admission host vetting + dial-time SSRF/rebind vetting + per-server
//! rate-limit + warrant-gated egress + secret-less `CredentialRef` (D81).

use std::path::Path;
use std::sync::Arc;

use kx_capability::{Capability, LocalCapabilityBroker};
use kx_content::ContentStore;
use kx_gateway_core::{
    McpAdminError, McpGatewayAdmin, McpServerRegistration, McpServerView, RegisterServerOutcome,
    RegisteredToolEntry,
};
use kx_mcp_gateway::{
    CapabilitySink, GatewayError, McpGateway, SqliteConnectionStore, TransportSpec,
};
use kx_tool_registry::{RegistrationStatus, SqliteToolRegistry, ToolKind, ToolProvenance};

/// Build + wire the external MCP gateway: open the off-journal `connections.db`
/// beside the catalog ledgers, register the dialed-tool firing capability on the
/// serve broker, re-dial any persisted connections (fail-soft), and return the
/// `McpGatewayAdmin` seam for the 5 MCP-server RPCs. The generic content-store
/// type `S` stays local here (it never escapes into `kx-mcp-gateway`).
///
/// # Errors
/// [`GatewayError::Storage`] if `connections.db` cannot be opened.
pub(crate) fn wire_mcp_gateway<S: ContentStore + Send + Sync + 'static>(
    catalog_dir: &Path,
    registry: Arc<SqliteToolRegistry>,
    broker: Arc<LocalCapabilityBroker<S>>,
) -> Result<Arc<dyn McpGatewayAdmin>, GatewayError> {
    let store = SqliteConnectionStore::open(catalog_dir.join("connections.db"))?;
    let sink: Arc<dyn CapabilitySink> = Arc::new(BrokerCapabilitySink::new(broker));
    let allowlist = crate::tools::tool_host_allowlist();
    let gateway = Arc::new(McpGateway::new(store, registry.clone(), sink, allowlist));
    // Re-dial persisted servers so a restart re-registers their tools +
    // capabilities (fail-soft per server: a dead server is marked Unreachable).
    let _ = gateway.redial_persisted();
    Ok(Arc::new(HostMcpGateway::new(gateway, registry)))
}

/// Register a dialed tool's [`Capability`] on the serve broker at runtime. Wraps
/// the concrete `Arc<LocalCapabilityBroker<S>>` (whose `register_capability` is
/// `&self`/interior-mutable) so the generic content-store type never escapes into
/// `kx-mcp-gateway` (the dependency-inversion seam).
pub(crate) struct BrokerCapabilitySink<S: ContentStore + Send + Sync> {
    broker: Arc<LocalCapabilityBroker<S>>,
}

impl<S: ContentStore + Send + Sync> BrokerCapabilitySink<S> {
    pub(crate) fn new(broker: Arc<LocalCapabilityBroker<S>>) -> Self {
        Self { broker }
    }
}

impl<S: ContentStore + Send + Sync + 'static> CapabilitySink for BrokerCapabilitySink<S> {
    fn register_capability(&self, capability: Box<dyn Capability>) {
        self.broker.register_capability(capability);
    }
}

/// The [`McpGatewayAdmin`] host impl over the [`McpGateway`] + the durable
/// `tools.db` (for the `DiscoverServerTools` inventory projection).
pub(crate) struct HostMcpGateway {
    gateway: Arc<McpGateway>,
    registry: Arc<SqliteToolRegistry>,
}

impl HostMcpGateway {
    pub(crate) fn new(gateway: Arc<McpGateway>, registry: Arc<SqliteToolRegistry>) -> Self {
        Self { gateway, registry }
    }

    /// Project the registry's rows for one server (the `<name>/…` namespace) into
    /// the gateway-core inventory vocabulary.
    fn server_tool_rows(
        &self,
        server_name: &str,
    ) -> Result<Vec<RegisteredToolEntry>, McpAdminError> {
        let prefix = format!("{server_name}/");
        let rows = self
            .registry
            .discover(4096, None)
            .map_err(|e| McpAdminError::Storage(e.to_string()))?;
        Ok(rows
            .into_iter()
            .filter(|e| e.def.tool_id.0.starts_with(&prefix))
            .map(project_entry)
            .collect())
    }
}

impl McpGatewayAdmin for HostMcpGateway {
    fn register_server(
        &self,
        reg: McpServerRegistration,
    ) -> Result<RegisterServerOutcome, McpAdminError> {
        let transport = transport_from_wire(&reg)?;
        let outcome = self
            .gateway
            .register_server(&reg.server_name, transport, reg.credential_ref)
            .map_err(map_err)?;
        Ok(RegisterServerOutcome {
            connection_id: outcome.connection_id,
            discovered: outcome.discovered,
            health: outcome.health.tag().to_string(),
        })
    }

    fn list_servers(&self) -> Result<Vec<McpServerView>, McpAdminError> {
        let servers = self.gateway.list_servers().map_err(map_err)?;
        Ok(servers
            .into_iter()
            .map(|c| McpServerView {
                connection_id: c.id,
                server_name: c.name,
                transport: c.transport.kind().to_string(),
                endpoint: c.transport.endpoint().to_string(),
                health: c.health.tag().to_string(),
                tool_count: c.tool_count,
                credential_ref_present: c.credential_ref.is_some(),
            })
            .collect())
    }

    fn discover_server(
        &self,
        server_name: &str,
    ) -> Result<(Vec<RegisteredToolEntry>, u32), McpAdminError> {
        let discovered = self.gateway.discover(server_name).map_err(map_err)?;
        let rows = self.server_tool_rows(server_name)?;
        Ok((rows, discovered))
    }

    fn test_server(&self, server_name: &str) -> Result<(bool, String), McpAdminError> {
        let reachable = self.gateway.test(server_name).map_err(map_err)?;
        let detail = if reachable {
            "reachable".to_string()
        } else {
            "unreachable (dial/handshake failed)".to_string()
        };
        Ok((reachable, detail))
    }

    fn deregister_server(&self, server_name: &str) -> Result<bool, McpAdminError> {
        self.gateway.deregister_server(server_name).map_err(map_err)
    }
}

/// Build a `kx-mcp-gateway` transport spec from the gateway-core wire registration.
fn transport_from_wire(reg: &McpServerRegistration) -> Result<TransportSpec, McpAdminError> {
    match reg.transport.as_str() {
        "stdio" => Ok(TransportSpec::Stdio {
            command: reg.endpoint.clone(),
            args: reg.args.clone(),
        }),
        "http" => Ok(TransportSpec::Http {
            url: reg.endpoint.clone(),
            tls_required: reg.tls_required,
        }),
        other => Err(McpAdminError::InvalidArgument(format!(
            "unknown transport {other:?} (want \"stdio\" | \"http\")"
        ))),
    }
}

/// Map the gateway crate's error onto the gateway-core admin error vocabulary.
fn map_err(e: GatewayError) -> McpAdminError {
    match e {
        GatewayError::HostRejected(d) => McpAdminError::HostRejected(d),
        GatewayError::InvalidSpec(d) => McpAdminError::InvalidArgument(d),
        GatewayError::Dial(d) => McpAdminError::Dial(d),
        GatewayError::RateLimited(d) => McpAdminError::RateLimited(d),
        GatewayError::NotFound(d) => McpAdminError::NotFound(d),
        GatewayError::Storage(d) => McpAdminError::Storage(d),
    }
}

/// Project a durable registry entry into the gateway-core inventory row (mirrors
/// `crate::tools::project_entry`; kept local so the MCP-server discover surface
/// is self-contained).
fn project_entry(e: kx_tool_registry::RegisteredEntry) -> RegisteredToolEntry {
    let kind = match &e.def.kind {
        ToolKind::Builtin => "Builtin",
        ToolKind::Mcp { .. } => "Mcp",
        ToolKind::LocalScript { .. } => "LocalScript",
        ToolKind::External { .. } => "External",
        ToolKind::SelfGenerated { .. } => "SelfGenerated",
    }
    .to_string();
    let provenance = match &e.provenance {
        ToolProvenance::HumanAuthored { .. } => "HumanAuthored",
        ToolProvenance::SelfGenerated { .. } => "SelfGenerated",
    }
    .to_string();
    let registration_status = match e.status() {
        RegistrationStatus::Approved => "Approved",
        RegistrationStatus::PendingHumanReview => "PendingHumanReview",
    }
    .to_string();
    let net_scope_summary = match &e.def.required_capability.net_scope_required {
        kx_warrant::NetScope::None => "none".to_string(),
        kx_warrant::NetScope::EgressAllowlist(hosts) => {
            let joined = hosts
                .iter()
                .map(|h| h.0.as_str())
                .collect::<Vec<_>>()
                .join(",");
            format!("egress:{joined}")
        }
    };
    RegisteredToolEntry {
        tool_id: e.tool_id,
        tool_name: e.def.tool_id.0,
        tool_version: e.def.tool_version.0,
        kind,
        description: e.def.description,
        idempotency_class: format!("{:?}", e.def.idempotency_class),
        provenance,
        registration_status,
        server_host: e.server_host.unwrap_or_default(),
        net_scope_summary,
        is_builtin: e.is_builtin,
    }
}
