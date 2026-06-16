//! PR-6a host wiring for the declarative tools registry: the [`ToolRegistryAdmin`]
//! impl over the durable [`SqliteToolRegistry`] (`tools.db`) + the admission-time
//! SSRF vetting of a `RegisterTool`'s `server_host`.
//!
//! The runtime is a SECURE GATEWAY to external MCP servers, never an executor of
//! arbitrary code (D132/D159/GR19). PR-6a *registers + vets + stores* an external
//! MCP tool's endpoint; DIALING it (the live remote tool round), credentialed
//! Connections, and parallel fan-out are PR-6b/Cloud. So the SSRF check here is
//! the FIRST gate (reject obviously-internal hosts at admission); the behavioural
//! dial-time rebind defense (`kx_mcp::egress::vet_resolved_addr`) is PR-6b's
//! second gate. We replicate a focused, std-only host classifier here (rather than
//! taking the optional `kx-mcp` dep) so the registry RPCs compile in every serve
//! feature config; `kx_mcp::egress::classify_ip` remains the canonical policy the
//! dial path reuses.

use std::collections::BTreeSet;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::Arc;

use kx_content::ContentRef;
use kx_gateway_core::{
    GatewayError, RegisteredToolEntry, ToolAdminError, ToolRegistration, ToolRegistryAdmin,
};
use kx_mote::{ToolName, ToolVersion};
use kx_tool_registry::{
    tool_id_of, IdempotencyClass, InputSchema, McpEndpointId, ParamSpec, ParamType, RegisteredEntry,
    RegistrationStatus, SqliteToolRegistry, ToolDef, ToolKind, ToolProvenance,
};
use kx_warrant::{FsScope, Host, NetScope, ResourceCeiling, ToolRequirement};

/// The audit-only author stamped on an operator registration. SN-8: the author
/// is NOT enforcement-bearing (authority comes only from the server-issued
/// warrant); it is an audit/display field. A single-party OSS serve stamps a
/// fixed principal; the per-party principal is a Cloud concern (multi-tenant).
const REGISTER_AUTHOR: &str = "operator";

/// The [`ToolRegistryAdmin`] host impl over the durable `tools.db`. Holds the same
/// `Arc<SqliteToolRegistry>` the serve path seeds the built-ins + bundled tools
/// into, so `DiscoverTools` reflects the real registry.
pub(crate) struct HostToolRegistry {
    registry: Arc<SqliteToolRegistry>,
    /// Optional deny-by-default host allowlist (`KX_SERVE_TOOL_HOST_ALLOWLIST`).
    /// Empty ⇒ any non-internal host may be registered (nothing is dialed in 6a);
    /// non-empty ⇒ only listed hosts may be registered.
    allowlist: Vec<String>,
}

impl HostToolRegistry {
    pub(crate) fn new(registry: Arc<SqliteToolRegistry>, allowlist: Vec<String>) -> Self {
        Self {
            registry,
            allowlist,
        }
    }
}

/// The optional operator host allowlist for `RegisterTool` — `KX_SERVE_TOOL_HOST_ALLOWLIST`
/// (comma-separated `host[:port]`). Empty/unset ⇒ any non-internal host may be
/// registered (nothing is dialed in PR-6a; the real egress gate is PR-6b's
/// dial-time `vet_resolved_addr` + the warrant `net_scope`).
pub(crate) fn tool_host_allowlist() -> Vec<String> {
    std::env::var("KX_SERVE_TOOL_HOST_ALLOWLIST")
        .ok()
        .map(|s| {
            s.split(',')
                .map(str::trim)
                .filter(|h| !h.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

impl ToolRegistryAdmin for HostToolRegistry {
    fn register(&self, reg: ToolRegistration) -> Result<[u8; 16], ToolAdminError> {
        // (1) SSRF admission gate — reject internal/link-local/metadata hosts
        // deny-by-default; honor the optional operator allowlist.
        vet_registration_host(&reg.server_host, &self.allowlist)
            .map_err(ToolAdminError::HostRejected)?;

        // (2) Map the declared idempotency class fail-closed.
        let idempotency_class = parse_idempotency_class(&reg.idempotency_class)
            .ok_or_else(|| ToolAdminError::InvalidArgument(format!(
                "unknown idempotency_class {:?} (want Token|Readback|Staged|AtLeastOnce)",
                reg.idempotency_class
            )))?;

        // (3) Map the optional typed schema fail-closed (no float — SN-8).
        let input_schema = match reg.input_schema {
            None => None,
            Some(s) => Some(map_input_schema(s).map_err(ToolAdminError::InvalidArgument)?),
        };

        // (4) Server-derive identity + capability. kind = Mcp{endpoint = host};
        // net_scope_required = egress to the (vetted) host — the MCP-as-egress
        // monotonic rule (a warrant must permit that host to ever resolve it).
        let remote_name = if reg.remote_name.trim().is_empty() {
            reg.tool_name.clone()
        } else {
            reg.remote_name.clone()
        };
        let def = ToolDef {
            tool_id: ToolName(reg.tool_name),
            tool_version: ToolVersion(reg.tool_version),
            kind: ToolKind::Mcp {
                endpoint: McpEndpointId(reg.server_host.clone()),
                remote_name,
            },
            required_capability: ToolRequirement {
                net_scope_required: NetScope::EgressAllowlist(BTreeSet::from([Host(
                    reg.server_host.clone(),
                )])),
                fs_scope_required: FsScope::empty(),
                syscall_profile_ref: ContentRef::from_bytes([0; 32]),
                min_resource_ceiling: ResourceCeiling {
                    cpu_milli: 0,
                    mem_bytes: 0,
                    wall_clock_ms: 0,
                    fd_count: 0,
                    disk_bytes: 0,
                },
            },
            description: reg.description,
            idempotency_class,
            input_schema,
        };

        // (5) Durable write — always HumanAuthored (Approved). SN-8: the client
        // cannot self-assert SelfGenerated to launder lineage.
        let token = self
            .registry
            .register_durable(
                def,
                ToolProvenance::HumanAuthored {
                    author: REGISTER_AUTHOR.to_string(),
                },
                Some(reg.server_host),
            )
            .map_err(|e| ToolAdminError::Storage(e.to_string()))?;
        Ok(tool_id_of(&token))
    }

    fn deregister(&self, tool_name: &str, tool_version: &str) -> Result<bool, GatewayError> {
        self.registry
            .deregister(&ToolName(tool_name.to_string()), &ToolVersion(tool_version.to_string()))
            .map_err(|e| GatewayError::Internal(e.to_string()))
    }

    fn discover(
        &self,
        limit: usize,
        after: Option<(String, String)>,
    ) -> Result<(Vec<RegisteredToolEntry>, bool), GatewayError> {
        // Over-fetch by one to compute has_more without a second query.
        let after_ref = after.as_ref().map(|(n, v)| (n.as_str(), v.as_str()));
        let mut rows = self
            .registry
            .discover(limit + 1, after_ref)
            .map_err(|e| GatewayError::Internal(e.to_string()))?;
        let has_more = rows.len() > limit;
        rows.truncate(limit);
        Ok((rows.into_iter().map(project_entry).collect(), has_more))
    }
}

/// Project a durable registry entry into the gateway-core wire vocabulary.
fn project_entry(e: RegisteredEntry) -> RegisteredToolEntry {
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
        NetScope::None => "none".to_string(),
        NetScope::EgressAllowlist(hosts) => {
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

fn parse_idempotency_class(s: &str) -> Option<IdempotencyClass> {
    match s {
        "Token" => Some(IdempotencyClass::Token),
        "Readback" => Some(IdempotencyClass::Readback),
        "Staged" => Some(IdempotencyClass::Staged),
        "AtLeastOnce" => Some(IdempotencyClass::AtLeastOnce),
        _ => None,
    }
}

fn map_input_schema(s: kx_gateway_core::ToolSchemaWire) -> Result<InputSchema, String> {
    let mut params = Vec::with_capacity(s.params.len());
    for p in s.params {
        let max_len = if p.max_len == 0 { 4096 } else { p.max_len as usize };
        let ty = match p.ty.as_str() {
            "str" => ParamType::Str { max_len },
            "bytes" => ParamType::Bytes { max_len },
            "int" => ParamType::Int {
                min: None,
                max: None,
            },
            "bool" => ParamType::Bool,
            "enum" => ParamType::Enum {
                allowed: p.allowed.into_iter().collect(),
            },
            other => {
                return Err(format!(
                    "unsupported param type {other:?} (want str|bytes|int|bool|enum)"
                ))
            }
        };
        params.push(ParamSpec {
            name: p.name,
            ty,
            required: p.required,
        });
    }
    Ok(InputSchema {
        params,
        deny_unknown: s.deny_unknown,
    })
}

/// Admission-time SSRF vetting of a `RegisterTool` `server_host` (`host[:port]`),
/// deny-by-default. An IP literal is rejected unless it is a public address
/// (loopback / private / link-local / metadata 169.254.169.254 / unspecified /
/// multicast / broadcast / documentation / CGNAT / IPv6 ULA all refused); a DNS
/// name is accepted unless an operator allowlist is set and excludes it. PR-6a
/// does not DIAL the host — the behavioural rebind defense is PR-6b's dial-time
/// `kx_mcp::egress::vet_resolved_addr`.
///
/// # Errors
/// A human-readable rejection reason (no internal detail; the host is echoed).
pub(crate) fn vet_registration_host(host_port: &str, allowlist: &[String]) -> Result<(), String> {
    let host = host_part(host_port.trim());
    if host.is_empty() {
        return Err("server_host is empty".to_string());
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        if !is_public_ip(&ip) {
            return Err(format!(
                "server_host {host:?} is a non-public address (internal/link-local/metadata refused)"
            ));
        }
    }
    if !allowlist.is_empty() && !allowlist.iter().any(|h| h == host) {
        return Err(format!(
            "server_host {host:?} is not in KX_SERVE_TOOL_HOST_ALLOWLIST"
        ));
    }
    Ok(())
}

/// Extract the host part of a `host[:port]` (handles bracketed + bare IPv6).
fn host_part(s: &str) -> &str {
    if let Some(rest) = s.strip_prefix('[') {
        // [ipv6]:port — host is up to the closing bracket.
        return rest.split(']').next().unwrap_or("");
    }
    // host:port (exactly one colon) ⇒ strip the port; a bare hostname / IPv4 or a
    // bare IPv6 (≥2 colons) ⇒ the whole string is the host.
    if s.matches(':').count() == 1 {
        s.split(':').next().unwrap_or("")
    } else {
        s
    }
}

/// Deny-by-default IP classification (std-only). Mirrors the intent of
/// `kx_mcp::egress::classify_ip`: only a clearly-global address is public.
fn is_public_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_v4(*v4),
        IpAddr::V6(v6) => is_public_v6(v6),
    }
}

fn is_public_v4(v4: Ipv4Addr) -> bool {
    let o = v4.octets();
    // CGNAT 100.64.0.0/10 (shared address space; not stable in std as is_shared).
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
    let link_local = (seg[0] & 0xffc0) == 0xfe80; // fe80::/10
    let unique_local = (seg[0] & 0xfe00) == 0xfc00; // fc00::/7
    let mapped_v4 = v6.to_ipv4_mapped().is_some(); // ::ffff:0:0/96 — refuse (deny-by-default)
    !(v6.is_loopback()
        || v6.is_unspecified()
        || v6.is_multicast()
        || link_local
        || unique_local
        || mapped_v4)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_internal_ip_literals() {
        for h in [
            "127.0.0.1",
            "127.0.0.1:8080",
            "10.0.0.5",
            "192.168.1.1:443",
            "172.16.0.1",
            "169.254.169.254", // cloud metadata endpoint
            "0.0.0.0",
            "100.64.0.1", // CGNAT
            "[::1]:9000",
            "[fe80::1]",
            "[fc00::1]:443",
        ] {
            assert!(
                vet_registration_host(h, &[]).is_err(),
                "expected {h} to be rejected"
            );
        }
    }

    #[test]
    fn accepts_public_hosts_when_no_allowlist() {
        for h in [
            "mcp.example.com",
            "mcp.example.com:8443",
            "8.8.8.8",
            "1.1.1.1:443",
        ] {
            assert!(
                vet_registration_host(h, &[]).is_ok(),
                "expected {h} to be accepted"
            );
        }
    }

    #[test]
    fn allowlist_excludes_unlisted_dns() {
        let allow = vec!["mcp.example.com".to_string()];
        assert!(vet_registration_host("mcp.example.com", &allow).is_ok());
        assert!(vet_registration_host("mcp.evil.com", &allow).is_err());
        // An internal host is still refused even if "allowlisted".
        assert!(vet_registration_host("127.0.0.1", &allow).is_err());
    }

    fn registration(name: &str, host: &str) -> ToolRegistration {
        ToolRegistration {
            tool_name: name.to_string(),
            tool_version: "1".to_string(),
            description: "a remote search tool".to_string(),
            idempotency_class: "Readback".to_string(),
            input_schema: Some(kx_gateway_core::ToolSchemaWire {
                params: vec![kx_gateway_core::ToolParamWire {
                    name: "q".to_string(),
                    ty: "str".to_string(),
                    max_len: 256,
                    required: true,
                    allowed: vec![],
                }],
                deny_unknown: true,
            }),
            server_host: host.to_string(),
            remote_name: String::new(),
        }
    }

    fn host_registry() -> HostToolRegistry {
        HostToolRegistry::new(
            Arc::new(SqliteToolRegistry::open_in_memory().unwrap()),
            vec![],
        )
    }

    #[test]
    fn register_then_discover_roundtrip() {
        let admin = host_registry();
        let id = admin.register(registration("web-search", "mcp.example.com")).unwrap();
        assert_ne!(id, [0u8; 16]);
        let (rows, _) = admin.discover(64, None).unwrap();
        let row = rows.iter().find(|r| r.tool_name == "web-search").unwrap();
        assert_eq!(row.kind, "Mcp");
        assert_eq!(row.server_host, "mcp.example.com");
        assert_eq!(row.net_scope_summary, "egress:mcp.example.com");
        assert_eq!(row.provenance, "HumanAuthored");
        assert_eq!(row.registration_status, "Approved");
        assert!(!row.is_builtin);
        // built-ins are visible too (re-seeded on open)
        assert!(rows.iter().any(|r| r.tool_name == "fs-read" && r.is_builtin));
    }

    #[test]
    fn register_internal_host_is_refused() {
        let admin = host_registry();
        let err = admin
            .register(registration("evil", "169.254.169.254"))
            .unwrap_err();
        assert!(matches!(err, ToolAdminError::HostRejected(_)));
    }

    #[test]
    fn bad_idempotency_class_is_invalid_argument() {
        let admin = host_registry();
        let mut reg = registration("bad", "mcp.example.com");
        reg.idempotency_class = "Whenever".to_string();
        let err = admin.register(reg).unwrap_err();
        assert!(matches!(err, ToolAdminError::InvalidArgument(_)));
    }

    #[test]
    fn deregister_removes_a_registered_tool() {
        let admin = host_registry();
        admin.register(registration("temp", "mcp.example.com")).unwrap();
        assert!(admin.deregister("temp", "1").unwrap());
        assert!(!admin.deregister("temp", "1").unwrap()); // gone now
        // a built-in cannot be deregistered
        assert!(!admin.deregister("fs-read", "1").unwrap());
    }
}
