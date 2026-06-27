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

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;

use kx_capability::{Capability, CapabilityBroker, EffectRequest, LocalCapabilityBroker};
use kx_content::ContentStore;
use kx_gateway_core::{
    CallToolOutcome, McpAdminError, McpGatewayAdmin, McpServerRegistration, McpServerView,
    RegisterServerOutcome, RegisteredToolEntry,
};
use kx_mcp_gateway::{
    CapabilitySink, GatewayError, McpGateway, SessionMode, SqliteConnectionStore, TransportSpec,
};
use kx_mote::{
    EffectPattern, GraphPosition, InferenceParams, InputDataId, LogicRef, ModelId, Mote, MoteDef,
    NdClass, PromptTemplateHash, ToolName, ToolVersion, MOTE_DEF_SCHEMA_VERSION,
};
use kx_tool_registry::{RegistrationStatus, SqliteToolRegistry, ToolKind, ToolProvenance};
use kx_warrant::{MoteClass, SecretRef, SecretScope, ToolGrant, ToolRequirement, WarrantSpec};

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
    content: Arc<S>,
) -> Result<Arc<dyn McpGatewayAdmin>, GatewayError> {
    let store = SqliteConnectionStore::open(catalog_dir.join("connections.db"))?;
    // The SAME broker backs both the dialed-tool capability sink AND the operator
    // diagnostic live-fire (`CallMcpTool`) — one fire path, SN-8 re-enforced there.
    let sink: Arc<dyn CapabilitySink> = Arc::new(BrokerCapabilitySink::new(broker.clone()));
    let allowlist = crate::tools::tool_host_allowlist();
    let gateway = Arc::new(McpGateway::new(store, registry.clone(), sink, allowlist));
    // Re-dial persisted servers so a restart re-registers their tools +
    // capabilities — but OFF the serve-bind path: a hung/dead persisted server
    // must NOT delay the listeners coming up (review #2). The dials are synchronous
    // (std threads), so run them on the blocking pool; each dial is budget-bounded
    // (REDIAL_WALL_CLOCK_MS). Fail-soft per server; capability registration is
    // interior-mutable + dialed tools aren't auto-granted in 6b-1, so deferring
    // re-registration past bind loses nothing. Health is off-digest, so this is
    // safe w.r.t. the digest/frozen-trio invariants.
    let redial_handle = gateway.clone();
    tokio::task::spawn_blocking(move || {
        if let Err(error) = redial_handle.redial_persisted() {
            tracing::warn!(%error, "MCP gateway: persisted-connection re-dial failed");
        }
    });
    Ok(Arc::new(HostMcpGateway::new(
        gateway, registry, broker, content,
    )))
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
/// `tools.db` (for the `DiscoverServerTools` inventory projection) + the serve
/// broker & content store (for the `CallMcpTool` operator diagnostic fire). Generic
/// over the content store `S` so the concrete broker/content types stay local.
pub(crate) struct HostMcpGateway<S: ContentStore + Send + Sync + 'static> {
    gateway: Arc<McpGateway>,
    registry: Arc<SqliteToolRegistry>,
    broker: Arc<LocalCapabilityBroker<S>>,
    content: Arc<S>,
}

impl<S: ContentStore + Send + Sync + 'static> HostMcpGateway<S> {
    pub(crate) fn new(
        gateway: Arc<McpGateway>,
        registry: Arc<SqliteToolRegistry>,
        broker: Arc<LocalCapabilityBroker<S>>,
        content: Arc<S>,
    ) -> Self {
        Self {
            gateway,
            registry,
            broker,
            content,
        }
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

impl<S: ContentStore + Send + Sync + 'static> McpGatewayAdmin for HostMcpGateway<S> {
    fn register_server(
        &self,
        reg: McpServerRegistration,
    ) -> Result<RegisterServerOutcome, McpAdminError> {
        let transport = transport_from_wire(&reg)?;
        let session_mode = SessionMode::from_tag(&reg.session_mode);
        let outcome = self
            .gateway
            .register_server(
                &reg.server_name,
                transport,
                reg.credential_ref,
                session_mode,
            )
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
                session_mode: c.session_mode.tag().to_string(),
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
        // T-CONN: `test` now runs the SAME probe as `register` (initialize +
        // tools/list), so the detail names what was actually checked — `test` and
        // `add` can no longer disagree on reachability.
        let detail = if reachable {
            "reachable (initialize + tools/list)".to_string()
        } else {
            "unreachable (initialize/tools-list handshake failed)".to_string()
        };
        Ok((reachable, detail))
    }

    fn deregister_server(&self, server_name: &str) -> Result<bool, McpAdminError> {
        self.gateway.deregister_server(server_name).map_err(map_err)
    }

    fn call_tool(
        &self,
        server_name: &str,
        remote_name: &str,
        args_json: &str,
    ) -> Result<CallToolOutcome, McpAdminError> {
        let tool_id = ToolName(format!("{server_name}/{remote_name}"));
        // Resolve the REGISTERED def — its version, declared scopes, and typed schema
        // are the source of truth (the client supplies none of them; SN-8).
        let def = self
            .registry
            .defs()
            .into_iter()
            .find(|d| d.tool_id == tool_id)
            .ok_or_else(|| {
                McpAdminError::NotFound(format!(
                    "no registered tool `{server_name}/{remote_name}` (dial the server first)"
                ))
            })?;
        // Validate the args against the tool's typed inputSchema FAIL-CLOSED (the same
        // gate the agentic settle applies) so a bad fire never reaches the connector.
        if let Some(schema) = def.input_schema.as_ref() {
            kx_tool_registry::validate_args(schema, args_json.as_bytes()).map_err(|e| {
                McpAdminError::InvalidArgument(format!(
                    "args do not match the tool inputSchema: {e}"
                ))
            })?;
        }
        // The connection's credential ref NAME (never the value, D81) → the warrant's
        // secret scope, so the broker admits the transport's out-of-band resolution.
        let secret_scope = self
            .gateway
            .list_servers()
            .ok()
            .and_then(|servers| servers.into_iter().find(|c| c.name == server_name))
            .and_then(|c| c.credential_ref)
            .map_or(SecretScope::None, |cred| {
                SecretScope::AllowList(BTreeSet::from([SecretRef(cred)]))
            });
        let cap = def.required_capability.clone();
        let mote = diagnostic_fire_mote(&tool_id, &def.tool_version);
        // The single-grant warrant is built from the tool's OWN declared scopes — the
        // broker re-verifies tool ∈ grants + request scopes ⊆ warrant (SN-8 at the gate).
        let warrant =
            diagnostic_fire_warrant(&tool_id, &def.tool_version, &cap, secret_scope.clone());
        let request = EffectRequest {
            payload: args_json.as_bytes().to_vec(),
            pattern: EffectPattern::StageThenCommit,
            idempotency_key: None,
            net_scope: cap.net_scope_required.clone(),
            fs_scope: cap.fs_scope_required.clone(),
            secret_scope,
        };
        let handle = self
            .broker
            .dispatch(&mote, &warrant, &tool_id, request)
            .map_err(|e| McpAdminError::Dial(format!("tool fire failed: {e}")))?;
        let payload = self.content.get(&handle.staged_ref).map_err(|e| {
            McpAdminError::Storage(format!("the staged tool result was unreadable: {e}"))
        })?;
        Ok(CallToolOutcome {
            result: (*payload).to_vec(),
        })
    }
}

/// A one-off `WorldMutating` / `StageThenCommit` Mote declaring `(tool, version)` in
/// its `tool_contract` so the broker admits the diagnostic fire (it never journals —
/// the Mote is discarded after dispatch). Mirrors the conformance harness `probe_mote`.
fn diagnostic_fire_mote(tool_id: &ToolName, version: &ToolVersion) -> Mote {
    let mut tool_contract = BTreeMap::new();
    tool_contract.insert(tool_id.clone(), version.clone());
    let def = MoteDef {
        critic_check: None,
        logic_ref: LogicRef::from_bytes([0; 32]),
        model_id: ModelId("kx-connector-diagnostic".into()),
        prompt_template_hash: PromptTemplateHash::from_bytes([0; 32]),
        tool_contract,
        nd_class: NdClass::WorldMutating,
        config_subset: BTreeMap::new(),
        effect_pattern: EffectPattern::StageThenCommit,
        critic_for: None,
        is_topology_shaper: false,
        inference_params: InferenceParams::default(),
        schema_version: MOTE_DEF_SCHEMA_VERSION,
    };
    Mote::new(
        def,
        InputDataId::from_bytes([0; 32]),
        GraphPosition(vec![0]),
        smallvec::SmallVec::new(),
    )
}

/// A single-grant warrant carrying EXACTLY the fired tool + the tool's OWN declared
/// net/fs/secret scopes — the broker re-verifies the request scopes are a subset
/// (SN-8). The client never supplies grants; this is server-built from the registry.
fn diagnostic_fire_warrant(
    tool_id: &ToolName,
    version: &ToolVersion,
    cap: &ToolRequirement,
    secret_scope: SecretScope,
) -> WarrantSpec {
    WarrantSpec {
        mote_class: MoteClass::WorldMutating,
        nd_class: MoteClass::WorldMutating,
        fs_scope: cap.fs_scope_required.clone(),
        net_scope: cap.net_scope_required.clone(),
        syscall_profile_ref: cap.syscall_profile_ref,
        tool_grants: BTreeSet::from([ToolGrant {
            tool_id: tool_id.clone(),
            tool_version: version.clone(),
        }]),
        secret_scope,
        ..Default::default()
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
        // T-CONN: surface the reachability FLAVOR in the detail so a client gets a
        // consistent transient-vs-permanent verdict across add/test/discover.
        GatewayError::Dial { reason, transient } => McpAdminError::Dial(format!(
            "{reason} ({})",
            if transient {
                "transient — the server may be down; retry"
            } else {
                "permanent — the server is incompatible or misconfigured; check the spec"
            }
        )),
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
