//! Connection domain types for the external MCP gateway.
//!
//! A [`Connection`] is an operator-registered external MCP server: a transport
//! spec (stdio command, or an HTTP endpoint), an OPTIONAL secret-less credential
//! reference (the env-var / vault key NAME, never the secret — D81), and a
//! folded health status. Its `connection_id` is SERVER-DERIVED from the operator
//! name (SN-8: the client never forges it) and is NEVER a `MoteId` / journal /
//! digest input — connections live in an off-journal, rebuildable sidecar.

use kx_warrant::{Host, NetScope};

/// How the gateway reaches a server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportSpec {
    /// A local subprocess MCP server (`command` + `args`), spoken to over its
    /// stdin/stdout. Credentials inject into the child env out-of-band (D81).
    Stdio {
        /// The server program to spawn.
        command: String,
        /// Command-line arguments for the server.
        args: Vec<String>,
    },
    /// A remote HTTP MCP server (JSON-RPC over POST to `url`). `tls_required`
    /// drives `https_only` (a plaintext `http://` dial is refused when `true`).
    Http {
        /// The server endpoint URL (`http(s)://host[:port]/path`).
        url: String,
        /// Whether plaintext `http://` is refused (the warrant's `tls_required`).
        tls_required: bool,
    },
}

impl TransportSpec {
    /// A short, stable kind tag for storage + display (`"stdio"` / `"http"`).
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            TransportSpec::Stdio { .. } => "stdio",
            TransportSpec::Http { .. } => "http",
        }
    }

    /// The human endpoint string (the command for stdio, the URL for HTTP) —
    /// for display + the connections sidecar's `endpoint` column.
    #[must_use]
    pub fn endpoint(&self) -> &str {
        match self {
            TransportSpec::Stdio { command, .. } => command,
            TransportSpec::Http { url, .. } => url,
        }
    }
}

/// A folded per-server health status (off-digest, advisory — never authority).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionHealth {
    /// Never dialed (just registered, or restored from the sidecar on restart).
    Unknown,
    /// The last dial completed the handshake.
    Connected,
    /// The last dial failed (unreachable / refused / timeout).
    Unreachable,
}

impl ConnectionHealth {
    /// A short, stable tag for storage + display.
    #[must_use]
    pub fn tag(self) -> &'static str {
        match self {
            ConnectionHealth::Unknown => "unknown",
            ConnectionHealth::Connected => "connected",
            ConnectionHealth::Unreachable => "unreachable",
        }
    }

    /// Parse a stored tag back into a health value (fail-soft → `Unknown`).
    #[must_use]
    pub fn from_tag(s: &str) -> Self {
        match s {
            "connected" => ConnectionHealth::Connected,
            "unreachable" => ConnectionHealth::Unreachable,
            _ => ConnectionHealth::Unknown,
        }
    }
}

/// An operator-registered external MCP server.
#[derive(Debug, Clone)]
pub struct Connection {
    /// The server-derived id (a stable handle; NEVER a digest input).
    pub id: [u8; 16],
    /// The operator-chosen unique name (the primary key).
    pub name: String,
    /// How the gateway reaches the server.
    pub transport: TransportSpec,
    /// The OPTIONAL credential reference NAME (never the secret value).
    pub credential_ref: Option<String>,
    /// The last-folded health.
    pub health: ConnectionHealth,
    /// The number of tools discovered on the last successful `tools/list`.
    pub tool_count: u32,
}

impl Connection {
    /// The egress host this server dials — the host of the HTTP URL, or empty for
    /// a stdio server (no egress). Used to derive the per-tool `net_scope`.
    #[must_use]
    pub fn egress_host(&self) -> Option<String> {
        match &self.transport {
            TransportSpec::Stdio { .. } => None,
            TransportSpec::Http { url, .. } => {
                url::host_of_url(url).map(std::string::ToString::to_string)
            }
        }
    }

    /// The per-tool egress requirement: an allowlist of exactly this server's host
    /// (HTTP), or `None` (stdio — no egress). The broker's `precheck` enforces a
    /// fired tool's `net_scope ⊆ warrant.net_scope`, so this binds the registered
    /// tool to its origin server.
    #[must_use]
    pub fn net_scope(&self) -> NetScope {
        match self.egress_host() {
            Some(host) => {
                let mut set = std::collections::BTreeSet::new();
                set.insert(Host(host));
                NetScope::EgressAllowlist(set)
            }
            None => NetScope::None,
        }
    }
}

/// Derive a server's stable 16-byte id from its operator name (server-derived,
/// SN-8). Deterministic ⇒ a sidecar rebuild re-materializes the same id set.
#[must_use]
pub fn connection_id_of(name: &str) -> [u8; 16] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"kx-mcp-connection\0");
    hasher.update(name.as_bytes());
    let full = hasher.finalize();
    let mut id = [0u8; 16];
    id.copy_from_slice(&full.as_bytes()[..16]);
    id
}

/// A tiny, dependency-light URL host extractor (no `url` crate needed here —
/// `kx-mcp` validates the full URL at dial time; this is only for the egress
/// host of an already-vetted HTTP endpoint).
mod url {
    /// Extract the bare host from an `http(s)://host[:port]/...` URL, or `None`.
    pub(super) fn host_of_url(u: &str) -> Option<&str> {
        let after_scheme = u.split_once("://").map(|(_, rest)| rest)?;
        // authority ends at the first `/`, `?`, or `#`.
        let authority = after_scheme
            .split(['/', '?', '#'])
            .next()
            .unwrap_or(after_scheme);
        // strip optional userinfo@.
        let hostport = authority.rsplit_once('@').map_or(authority, |(_, hp)| hp);
        if let Some(rest) = hostport.strip_prefix('[') {
            // [ipv6]:port
            return rest.split(']').next().filter(|h| !h.is_empty());
        }
        // host:port (single colon) or bare host.
        let host = if hostport.matches(':').count() == 1 {
            hostport.split(':').next().unwrap_or("")
        } else {
            hostport
        };
        if host.is_empty() {
            None
        } else {
            Some(host)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_id_is_stable_and_name_derived() {
        let a = connection_id_of("github");
        let b = connection_id_of("github");
        let c = connection_id_of("gitlab");
        assert_eq!(a, b, "same name ⇒ same id (rebuild-stable)");
        assert_ne!(a, c, "different name ⇒ different id");
        assert_ne!(a, [0u8; 16]);
    }

    #[test]
    fn http_net_scope_is_the_server_host() {
        let conn = Connection {
            id: connection_id_of("c"),
            name: "c".into(),
            transport: TransportSpec::Http {
                url: "https://mcp.example.com:8443/rpc".into(),
                tls_required: true,
            },
            credential_ref: None,
            health: ConnectionHealth::Unknown,
            tool_count: 0,
        };
        assert_eq!(conn.egress_host().as_deref(), Some("mcp.example.com"));
        match conn.net_scope() {
            NetScope::EgressAllowlist(hosts) => {
                assert!(hosts.contains(&Host("mcp.example.com".into())));
            }
            NetScope::None => panic!("http server must require egress"),
        }
    }

    #[test]
    fn stdio_has_no_egress() {
        let conn = Connection {
            id: connection_id_of("local"),
            name: "local".into(),
            transport: TransportSpec::Stdio {
                command: "my-mcp-server".into(),
                args: vec!["--stdio".into()],
            },
            credential_ref: None,
            health: ConnectionHealth::Unknown,
            tool_count: 0,
        };
        assert_eq!(conn.egress_host(), None);
        assert_eq!(conn.net_scope(), NetScope::None);
    }

    #[test]
    fn host_of_url_handles_forms() {
        use super::url::host_of_url;
        assert_eq!(host_of_url("https://a.com/rpc"), Some("a.com"));
        assert_eq!(host_of_url("http://a.com:8080"), Some("a.com"));
        assert_eq!(host_of_url("https://u:p@a.com/x"), Some("a.com"));
        assert_eq!(host_of_url("https://[::1]:9000/x"), Some("::1"));
        assert_eq!(host_of_url("not-a-url"), None);
    }
}
