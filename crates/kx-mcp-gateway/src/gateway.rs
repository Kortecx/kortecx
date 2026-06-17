//! [`McpGateway`] — the multi-server orchestrator.
//!
//! Manages N external MCP servers: dial (over `kx-mcp`'s session seam), discover
//! (`tools/list`), register each discovered tool into the durable registry +
//! the broker (so it is fireable), govern (the connections sidecar), and enforce
//! the live untrusted-egress surface (admission host vetting + per-server
//! rate-limit; dial-time SSRF/rebind is already enforced inside the `kx-mcp`
//! transports). The runtime is a SECURE GATEWAY (D132/GR19): no arbitrary code
//! runs in-runtime; the discovered tool's `tool_id` + the `connection_id` are
//! server-derived (SN-8).
//!
//! ## Firing scope (PR-6b-1 vs PR-6b-2)
//!
//! PR-6b-1 dials + discovers + registers (the tool's [`McpSessionCapability`] is
//! registered on the broker, so the call path is live and directly testable). The
//! bind-time react-warrant derivation that makes the autonomous ReAct loop GRANT
//! and auto-fire a dialed tool ships in PR-6b-2 (it shares the coordinator
//! args-from-params machinery with the authored `tool()` node). A registered tool
//! is fired ONLY through a warrant that grants it (SN-8) — never by mere presence.

use std::sync::Arc;

use kx_mcp::{HttpTransport, McpSessionCapability, McpTransport, StdioTransport};
use kx_mote::{ToolName, ToolVersion};
use kx_tool_registry::{
    IdempotencyClass, InputSchema, McpEndpointId, ParamSpec, ParamType, SqliteToolRegistry,
    ToolDef, ToolKind, ToolProvenance,
};
use kx_warrant::ToolRequirement;

use crate::connection::{connection_id_of, Connection, ConnectionHealth, TransportSpec};
use crate::errors::GatewayError;
use crate::ratelimit::RateLimiter;
use crate::store::SqliteConnectionStore;

/// Default per-call response-size cap for discovery + dials (1 MiB; the decoder
/// floor). Tool firing uses the warrant ceiling via the capability.
const DISCOVERY_MAX_BYTES: usize = 1 << 20;

/// Default per-dial wall-clock budget (ms) for operator-initiated dials
/// (register / discover / test): `0` ⇒ the transport's own default (30 s). The
/// operator is interactively waiting, so a generous ceiling is fine.
const DIAL_WALL_CLOCK_MS: u64 = 0;

/// Per-dial budget for the STARTUP re-dial of persisted servers (8 s). Tighter
/// than the interactive budget so a dead/hung persisted server is abandoned
/// quickly — and re-dial runs OFF the serve-bind path (a background task), so it
/// never delays the listeners coming up regardless.
const REDIAL_WALL_CLOCK_MS: u64 = 8_000;

/// MCP tool versions are not surfaced by `tools/list`; the gateway pins every
/// discovered tool to version `1` (the registry keys on `(name, version)`).
const REMOTE_TOOL_VERSION: &str = "1";

/// The seam by which the gateway registers a discovered tool's [`Capability`] on
/// the live serve broker, WITHOUT this crate depending on the concrete
/// content-store-generic `LocalCapabilityBroker<S>`. The host (`kx-gateway`)
/// implements it over its broker (`register_capability` is `&self`).
///
/// [`Capability`]: kx_capability::Capability
pub trait CapabilitySink: Send + Sync {
    /// Register (or replace) a capability on the broker at runtime.
    fn register_capability(&self, capability: Box<dyn kx_capability::Capability>);
}

/// The outcome of registering/discovering an MCP server.
#[derive(Debug, Clone)]
pub struct RegisterOutcome {
    /// The server-derived connection id.
    pub connection_id: [u8; 16],
    /// The number of tools discovered + registered.
    pub discovered: u32,
    /// The folded health after the dial.
    pub health: ConnectionHealth,
}

/// The multi-server external MCP gateway.
pub struct McpGateway {
    store: SqliteConnectionStore,
    registry: Arc<SqliteToolRegistry>,
    sink: Arc<dyn CapabilitySink>,
    rate_limiter: RateLimiter,
    /// Admission host allowlist (`KX_SERVE_TOOL_HOST_ALLOWLIST`), deny-by-default
    /// when non-empty; empty ⇒ any non-internal host (the SSRF classifier still
    /// refuses internal/metadata literals).
    allowlist: Vec<String>,
}

impl std::fmt::Debug for McpGateway {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpGateway")
            .field("allowlist", &self.allowlist)
            .finish_non_exhaustive()
    }
}

impl McpGateway {
    /// Build the gateway over a connections sidecar, the durable tool registry,
    /// the broker sink, and the admission allowlist. Default rate limit: 20-token
    /// burst, 10 dials/sec per server.
    #[must_use]
    pub fn new(
        store: SqliteConnectionStore,
        registry: Arc<SqliteToolRegistry>,
        sink: Arc<dyn CapabilitySink>,
        allowlist: Vec<String>,
    ) -> Self {
        Self {
            store,
            registry,
            sink,
            rate_limiter: RateLimiter::new(20, 10),
            allowlist,
        }
    }

    /// Register an external MCP server: vet its host (HTTP), dial + discover +
    /// register its tools, persist the connection, and fold health.
    ///
    /// A host that fails admission vetting is REFUSED (never stored). A dial
    /// failure is NOT fatal: the connection is stored with `Unreachable` health
    /// (the operator can fix the server and `test`/`discover` later) and the
    /// outcome reports `health = Unreachable`, `discovered = 0` — honest, never a
    /// fabricated success (GR15).
    ///
    /// # Errors
    /// [`GatewayError`] on an empty name, host rejection, an invalid spec, or a
    /// storage failure.
    pub fn register_server(
        &self,
        name: &str,
        transport: TransportSpec,
        credential_ref: Option<String>,
    ) -> Result<RegisterOutcome, GatewayError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(GatewayError::InvalidSpec(
                "server name is empty".to_string(),
            ));
        }
        if name.contains('/') {
            return Err(GatewayError::InvalidSpec(
                "server name must not contain '/' (it namespaces tool ids)".to_string(),
            ));
        }
        // Admission host vetting for HTTP (deny-by-default; stdio has no egress).
        if let TransportSpec::Http { url, .. } = &transport {
            // Refuse credentials embedded in the URL userinfo (`user:pass@host`):
            // they would persist in connections.db + ride the wire/CLI/UI/logs,
            // defeating the secret-less-credential invariant (D81, review #4). All
            // secrets must go through the by-name `credential_ref` header path.
            if url_authority(url).is_some_and(|a| a.contains('@')) {
                return Err(GatewayError::InvalidSpec(
                    "endpoint must not embed credentials in the URL (user:pass@host); use credential_ref"
                        .to_string(),
                ));
            }
            let host = crate::connection::Connection {
                id: [0; 16],
                name: name.to_string(),
                transport: TransportSpec::Http {
                    url: url.clone(),
                    tls_required: false,
                },
                credential_ref: None,
                health: ConnectionHealth::Unknown,
                tool_count: 0,
            }
            .egress_host()
            .ok_or_else(|| GatewayError::InvalidSpec(format!("endpoint has no host: {url}")))?;
            vet_registration_host(&host, &self.allowlist).map_err(GatewayError::HostRejected)?;
        }

        let conn = Connection {
            id: connection_id_of(name),
            name: name.to_string(),
            transport,
            credential_ref,
            health: ConnectionHealth::Unknown,
            tool_count: 0,
        };
        // Persist FIRST (so a dial failure still leaves a re-testable record).
        self.store.upsert(&conn)?;

        match self.dial_and_register(&conn, DIAL_WALL_CLOCK_MS) {
            Ok(count) => {
                self.store
                    .set_health(name, ConnectionHealth::Connected, count)?;
                Ok(RegisterOutcome {
                    connection_id: conn.id,
                    discovered: count,
                    health: ConnectionHealth::Connected,
                })
            }
            Err(e) => {
                tracing::warn!(server = %name, error = %e, "MCP server registered but unreachable");
                self.store
                    .set_health(name, ConnectionHealth::Unreachable, 0)?;
                Ok(RegisterOutcome {
                    connection_id: conn.id,
                    discovered: 0,
                    health: ConnectionHealth::Unreachable,
                })
            }
        }
    }

    /// Re-dial a registered server and re-discover/re-register its tools.
    ///
    /// # Errors
    /// [`GatewayError::NotFound`] if no such server; [`GatewayError`] on a dial /
    /// storage failure.
    pub fn discover(&self, name: &str) -> Result<u32, GatewayError> {
        let conn = self
            .store
            .get(name)?
            .ok_or_else(|| GatewayError::NotFound(name.to_string()))?;
        match self.dial_and_register(&conn, DIAL_WALL_CLOCK_MS) {
            Ok(count) => {
                self.store
                    .set_health(name, ConnectionHealth::Connected, count)?;
                Ok(count)
            }
            Err(e) => {
                self.store
                    .set_health(name, ConnectionHealth::Unreachable, 0)?;
                Err(e)
            }
        }
    }

    /// Test a server's reachability (dial + `initialize` only — no tool registration).
    ///
    /// # Errors
    /// [`GatewayError::NotFound`] if no such server.
    pub fn test(&self, name: &str) -> Result<bool, GatewayError> {
        let conn = self
            .store
            .get(name)?
            .ok_or_else(|| GatewayError::NotFound(name.to_string()))?;
        if !self.rate_limiter.try_acquire(&conn.name) {
            return Err(GatewayError::RateLimited(conn.name));
        }
        let transport = build_transport(&conn)?;
        let reachable = match transport.open_session() {
            Ok(mut session) => session.initialize(DIAL_WALL_CLOCK_MS).is_ok(),
            Err(_) => false,
        };
        let health = if reachable {
            ConnectionHealth::Connected
        } else {
            ConnectionHealth::Unreachable
        };
        self.store.set_health(name, health, conn.tool_count)?;
        Ok(reachable)
    }

    /// List all registered servers (ordered by name).
    ///
    /// # Errors
    /// [`GatewayError::Storage`] on a SQLite failure.
    pub fn list_servers(&self) -> Result<Vec<Connection>, GatewayError> {
        self.store.list()
    }

    /// Deregister a server: remove its connection record + deregister every tool
    /// it contributed (the `<name>/…` namespace). An orphaned broker capability is
    /// inert (never granted ⇒ never fires, SN-8) and clears on the next restart.
    ///
    /// # Errors
    /// [`GatewayError::Storage`] on a SQLite failure.
    pub fn deregister_server(&self, name: &str) -> Result<bool, GatewayError> {
        let removed = self.store.remove(name)?;
        if removed {
            // Deregister the server's namespaced tools from the durable registry.
            let prefix = format!("{name}/");
            let rows = self
                .registry
                .discover(4096, None)
                .map_err(|e| GatewayError::Storage(e.to_string()))?;
            for entry in rows {
                let tool_name = &entry.def.tool_id.0;
                if tool_name.starts_with(&prefix) {
                    let _ = self
                        .registry
                        .deregister(&entry.def.tool_id, &entry.def.tool_version);
                }
            }
        }
        Ok(removed)
    }

    /// On startup, re-dial every persisted connection (fail-soft per server) so a
    /// restart re-registers the discovered tools + their capabilities. A dead
    /// server is marked `Unreachable` and never aborts serve.
    ///
    /// # Errors
    /// [`GatewayError::Storage`] on a SQLite failure listing the connections.
    pub fn redial_persisted(&self) -> Result<(), GatewayError> {
        for conn in self.store.list()? {
            match self.dial_and_register(&conn, REDIAL_WALL_CLOCK_MS) {
                Ok(count) => {
                    let _ = self
                        .store
                        .set_health(&conn.name, ConnectionHealth::Connected, count);
                    tracing::info!(server = %conn.name, tools = count, "re-dialed persisted MCP server");
                }
                Err(e) => {
                    let _ = self
                        .store
                        .set_health(&conn.name, ConnectionHealth::Unreachable, 0);
                    tracing::warn!(server = %conn.name, error = %e, "persisted MCP server unreachable on restart");
                }
            }
        }
        Ok(())
    }

    /// Dial a server, `tools/list`, and register each discovered tool into the
    /// registry + the broker. Returns the count actually registered. A per-tool
    /// registration failure is logged + skipped (best-effort) rather than failing
    /// the whole dial, so `count` (and the folded health) reflect what truly
    /// registered — never a hard zero while some tools are live (review #7).
    fn dial_and_register(
        &self,
        conn: &Connection,
        wall_clock_ms: u64,
    ) -> Result<u32, GatewayError> {
        if !self.rate_limiter.try_acquire(&conn.name) {
            return Err(GatewayError::RateLimited(conn.name.clone()));
        }
        let transport = build_transport(conn)?;
        let mut session = transport
            .open_session()
            .map_err(|e| GatewayError::Dial(e.to_string()))?;
        session
            .initialize(wall_clock_ms)
            .map_err(|e| GatewayError::Dial(e.to_string()))?;
        let decls = session
            .list_tools(DISCOVERY_MAX_BYTES, wall_clock_ms)
            .map_err(|e| GatewayError::Dial(e.to_string()))?;

        let mut count = 0u32;
        for decl in decls {
            let remote = decl.name.trim();
            if remote.is_empty() || remote.contains('/') {
                tracing::warn!(server = %conn.name, "skipping tool with empty/invalid remote name");
                continue;
            }
            // Namespace the registered id by the connection name so tools from
            // different servers never collide and deregistration is scoped.
            let tool_id = ToolName(format!("{}/{}", conn.name, remote));
            let tool_version = ToolVersion(REMOTE_TOOL_VERSION.to_string());
            let input_schema = json_schema_to_input_schema(&decl.input_schema_json);
            let def = ToolDef {
                tool_id: tool_id.clone(),
                tool_version: tool_version.clone(),
                kind: ToolKind::Mcp {
                    endpoint: McpEndpointId(conn.transport.endpoint().to_string()),
                    remote_name: remote.to_string(),
                },
                required_capability: ToolRequirement {
                    net_scope_required: conn.net_scope(),
                    fs_scope_required: kx_warrant::FsScope::empty(),
                    syscall_profile_ref: kx_content::ContentRef::from_bytes([0; 32]),
                    min_resource_ceiling: kx_warrant::ResourceCeiling {
                        cpu_milli: 0,
                        mem_bytes: 0,
                        wall_clock_ms: 0,
                        fd_count: 0,
                        disk_bytes: 0,
                    },
                },
                description: if decl.description.is_empty() {
                    format!("MCP tool {remote} on server {}", conn.name)
                } else {
                    decl.description.clone()
                },
                // MCP effects are world-mutating by default → Staged (D66/D58 §7).
                idempotency_class: IdempotencyClass::Staged,
                input_schema,
            };
            if let Err(error) = self.registry.register_durable(
                def,
                ToolProvenance::HumanAuthored {
                    author: format!("mcp-gateway:{}", conn.name),
                },
                conn.egress_host(),
            ) {
                // Best-effort: a single tool's durable-write failure does not fail
                // the whole dial (or hard-zero the count) — log it + skip, so the
                // capability is NOT registered for an unpersisted tool either.
                tracing::warn!(server = %conn.name, tool = %remote, %error, "skipping tool (durable register failed)");
                continue;
            }

            // Register the firing capability on the broker (per-invoke session) —
            // only after the durable write succeeded.
            let cap_transport = build_transport(conn)?;
            self.sink
                .register_capability(Box::new(McpSessionCapability::new(
                    tool_id,
                    tool_version,
                    McpEndpointId(conn.transport.endpoint().to_string()),
                    remote.to_string(),
                    cap_transport,
                )));
            count += 1;
        }
        Ok(count)
    }
}

/// Build a `kx-mcp` transport from a connection's spec + optional credential.
fn build_transport(conn: &Connection) -> Result<Box<dyn McpTransport>, GatewayError> {
    match &conn.transport {
        TransportSpec::Stdio { command, args } => {
            let mut t = StdioTransport::new(command.as_str());
            for a in args {
                t = t.arg(a.as_str());
            }
            if let Some(secret_ref) = &conn.credential_ref {
                t = t.credential(kx_mcp::CredentialRef::from_env_var(secret_ref.clone()));
            }
            Ok(Box::new(t))
        }
        TransportSpec::Http { url, tls_required } => {
            let net_scope = conn.net_scope();
            let mut t = HttpTransport::new(url, &net_scope, *tls_required)
                .map_err(|e| GatewayError::InvalidSpec(e.to_string()))?;
            if let Some(secret_ref) = &conn.credential_ref {
                // The env var holds the FULL header value convention (e.g. "Bearer x").
                t = t.header_credential(
                    "Authorization",
                    kx_mcp::CredentialRef::from_env_var(secret_ref.clone()),
                );
            }
            Ok(Box::new(t))
        }
    }
}

/// Best-effort map of a remote tool's JSON-Schema `inputSchema` into the typed
/// registry [`InputSchema`]. Maps `string`/`integer`/`boolean` object properties
/// (the args the runtime can validate); skips unmappable types (number/array/
/// object) and sets `deny_unknown = false` (external servers may accept extra
/// fields — being lenient avoids false rejections, and the SERVER still validates
/// authoritatively). Returns `None` when the schema is absent or not an object
/// schema (⇒ no client-side arg gate; the remote validates).
fn json_schema_to_input_schema(schema_json: &[u8]) -> Option<InputSchema> {
    if schema_json.is_empty() {
        return None;
    }
    let value: serde_json::Value = serde_json::from_slice(schema_json).ok()?;
    let obj = value.as_object()?;
    let properties = obj.get("properties")?.as_object()?;
    let required: std::collections::BTreeSet<&str> = obj
        .get("required")
        .and_then(|r| r.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    let mut params = Vec::new();
    for (pname, pspec) in properties {
        let ty_str = pspec.get("type").and_then(|t| t.as_str());
        let ty = match ty_str {
            Some("string") => ParamType::Str { max_len: 8192 },
            Some("integer") => ParamType::Int {
                min: None,
                max: None,
            },
            Some("boolean") => ParamType::Bool,
            // number/array/object/null/absent → not client-validatable; skip the
            // param (the server validates it). The arg still passes through verbatim.
            _ => continue,
        };
        params.push(ParamSpec {
            name: pname.clone(),
            ty,
            required: required.contains(pname.as_str()),
        });
    }
    if params.is_empty() {
        return None;
    }
    // Deterministic param order (the registry keys schema by content).
    params.sort_by(|a, b| a.name.cmp(&b.name));
    Some(InputSchema {
        params,
        deny_unknown: false,
    })
}

/// Admission-time SSRF vetting of a server host (`host[:port]`), deny-by-default.
/// REPLICATES the focused std-only classifier in `kx-gateway::tools` (the same
/// `kx_mcp::egress::classify_ip` policy the dial path then re-checks on the
/// RESOLVED address — the two-gate egress split). Internal / link-local /
/// metadata / CGNAT / IPv6-ULA / mapped literals are refused; a DNS name is
/// accepted unless an operator allowlist excludes it.
fn vet_registration_host(host: &str, allowlist: &[String]) -> Result<(), String> {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    fn is_public_v4(v4: Ipv4Addr) -> bool {
        let o = v4.octets();
        let cgnat = o[0] == 100 && (o[1] & 0xc0) == 64;
        !(v4.is_loopback()
            || v4.is_private()
            || v4.is_link_local()
            || v4.is_unspecified()
            || v4.is_broadcast()
            || v4.is_documentation()
            || v4.is_multicast()
            || cgnat)
    }
    fn is_public_v6(v6: &Ipv6Addr) -> bool {
        let seg = v6.segments();
        let link_local = (seg[0] & 0xffc0) == 0xfe80;
        let unique_local = (seg[0] & 0xfe00) == 0xfc00;
        let mapped_v4 = v6.to_ipv4_mapped().is_some();
        !(v6.is_loopback()
            || v6.is_unspecified()
            || v6.is_multicast()
            || link_local
            || unique_local
            || mapped_v4)
    }

    let host = host.trim();
    if host.is_empty() {
        return Err("server host is empty".to_string());
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        let public = match ip {
            IpAddr::V4(v4) => is_public_v4(v4),
            IpAddr::V6(v6) => is_public_v6(&v6),
        };
        if !public {
            return Err(format!(
                "host {host:?} is a non-public address (internal/link-local/metadata refused)"
            ));
        }
    }
    // Normalize both sides (lowercase + strip a trailing FQDN dot) so the
    // case/dot-insensitive allowlist match is consistent with how a host is
    // compared elsewhere (review #6).
    if !allowlist.is_empty() {
        let want = normalize_host(host);
        if !allowlist.iter().any(|h| normalize_host(h) == want) {
            return Err(format!(
                "host {host:?} is not in KX_SERVE_TOOL_HOST_ALLOWLIST"
            ));
        }
    }
    Ok(())
}

/// Normalize a host for an allowlist comparison: lowercase + strip one trailing
/// FQDN dot. (Host-only; the caller already stripped any port.)
fn normalize_host(h: &str) -> String {
    h.trim().trim_end_matches('.').to_ascii_lowercase()
}

/// Extract the authority (`[userinfo@]host[:port]`) of an `http(s)://…` URL — the
/// substring between `://` and the first `/`, `?`, or `#`. Used to refuse
/// userinfo-embedded credentials at admission (dependency-light; no `url` crate).
fn url_authority(u: &str) -> Option<&str> {
    let after = u.split_once("://").map(|(_, rest)| rest)?;
    Some(after.split(['/', '?', '#']).next().unwrap_or(after))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_schema_maps_known_types_and_required() {
        let schema = br#"{"type":"object","properties":{
            "q":{"type":"string"},
            "n":{"type":"integer"},
            "flag":{"type":"boolean"},
            "weird":{"type":"number"}
        },"required":["q"]}"#;
        let mapped = json_schema_to_input_schema(schema).unwrap();
        // number is skipped; 3 mappable params, sorted by name.
        assert_eq!(mapped.params.len(), 3);
        assert_eq!(mapped.params[0].name, "flag");
        assert_eq!(mapped.params[1].name, "n");
        assert_eq!(mapped.params[2].name, "q");
        assert!(mapped.params[2].required);
        assert!(!mapped.params[1].required);
        assert!(!mapped.deny_unknown, "external schemas stay lenient");
    }

    #[test]
    fn json_schema_absent_or_empty_is_none() {
        assert!(json_schema_to_input_schema(b"").is_none());
        assert!(json_schema_to_input_schema(b"{}").is_none());
        assert!(json_schema_to_input_schema(b"not json").is_none());
        // object schema with only unmappable props → None (server validates).
        assert!(json_schema_to_input_schema(br#"{"properties":{"x":{"type":"array"}}}"#).is_none());
    }

    #[test]
    fn vet_refuses_internal_and_metadata_hosts() {
        for h in [
            "127.0.0.1",
            "10.0.0.1",
            "169.254.169.254",
            "[::1]",
            "[fc00::1]",
        ] {
            // strip brackets for the parse (host_part is done by the caller; here
            // we pass the bare host the connection extracts).
            let bare = h.trim_start_matches('[').trim_end_matches(']');
            assert!(
                vet_registration_host(bare, &[]).is_err(),
                "{h} must be refused"
            );
        }
    }

    #[test]
    fn vet_accepts_public_and_honors_allowlist() {
        assert!(vet_registration_host("mcp.example.com", &[]).is_ok());
        assert!(vet_registration_host("8.8.8.8", &[]).is_ok());
        let allow = vec!["mcp.example.com".to_string()];
        assert!(vet_registration_host("mcp.example.com", &allow).is_ok());
        assert!(vet_registration_host("evil.com", &allow).is_err());
    }

    #[test]
    fn allowlist_match_is_case_and_trailing_dot_insensitive() {
        // review #6: normalize both sides of the allowlist comparison.
        let allow = vec!["MCP.Example.COM".to_string()];
        assert!(vet_registration_host("mcp.example.com", &allow).is_ok());
        assert!(vet_registration_host("mcp.example.com.", &allow).is_ok());
    }

    #[test]
    fn url_authority_extracts_userinfo_and_host() {
        // review #4: the authority carries userinfo when present.
        assert_eq!(url_authority("https://u:p@host/rpc"), Some("u:p@host"));
        assert!(url_authority("https://u:p@host/rpc").unwrap().contains('@'));
        assert_eq!(url_authority("https://host:443/rpc?x#y"), Some("host:443"));
        assert!(!url_authority("https://host/rpc").unwrap().contains('@'));
    }
}
