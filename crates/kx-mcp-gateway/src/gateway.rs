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

use kx_mcp::{
    EnvSecretStore, HttpTransport, McpSessionCapability, McpTransport, RemoteToolDecl, SecretStore,
    StdioTransport,
};
use kx_mote::{ToolName, ToolVersion};
use kx_tool_registry::{
    IdempotencyClass, InputSchema, McpEndpointId, ParamSpec, ParamType, SqliteToolRegistry,
    ToolDef, ToolKind, ToolProvenance,
};
use kx_warrant::ToolRequirement;

use crate::connection::{
    connection_id_of, Connection, ConnectionHealth, SessionMode, TransportSpec,
};
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
    /// The resolver that turns an authorized `credential_ref` NAME into its secret
    /// value, transiently, at transport setup (D110.2). Defaults to the env-var
    /// passthrough ([`EnvSecretStore`]); the host injects a keychain-backed
    /// [`kx_mcp::ChainedSecretStore`] (MM-3) so a connection credential resolves
    /// from the OS keychain first, then the host environment (back-compat).
    secret_store: Arc<dyn SecretStore>,
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
            secret_store: Arc::new(EnvSecretStore),
        }
    }

    /// Inject the resolver used to turn a connection's `credential_ref` NAME into
    /// its secret value at transport setup. The host wires a keychain-backed
    /// [`kx_mcp::ChainedSecretStore`] here (MM-3); the default is the env-var
    /// passthrough, so existing env-var credentials keep resolving unchanged.
    #[must_use]
    pub fn with_secret_store(mut self, secret_store: Arc<dyn SecretStore>) -> Self {
        self.secret_store = secret_store;
        self
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
        session_mode: SessionMode,
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
                session_mode: SessionMode::Stateless,
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
            session_mode,
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
        // T-CONN: reachable iff the FULL handshake the gateway needs to USE the
        // server succeeds (`initialize` + `tools/list`) — the SAME `probe` that
        // `register`/`dial_and_register` uses, so `test` and `add` can never
        // disagree. Discards the decls (test only checks reachability + folds
        // health; it never registers tools, so `conn.tool_count` is preserved).
        let reachable = Self::probe(&conn, DIAL_WALL_CLOCK_MS, &self.secret_store).is_ok();
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

    /// T-CONN: the ONE reachability probe — open a session, complete the MCP
    /// handshake (`initialize`), and `tools/list`. This is the single definition of
    /// "reachable": the full handshake the gateway needs to actually USE the server.
    /// Both callers route through it — `dial_and_register` registers the returned
    /// decls, `test` discards them — so the two paths can NEVER disagree. Before
    /// this, `test` stopped at `initialize` while register went on to `tools/list`,
    /// so a server that handshakes but fails `tools/list` reported reachable via
    /// `test` yet unreachable via `add` (register). Does NOT rate-limit — each
    /// caller keeps its own single `try_acquire` (no double-acquire); a pure dial
    /// helper (no `self`), so the rate-limit + store fold stay with the callers.
    fn probe(
        conn: &Connection,
        wall_clock_ms: u64,
        secret_store: &Arc<dyn SecretStore>,
    ) -> Result<Vec<RemoteToolDecl>, GatewayError> {
        let transport = build_transport(conn, secret_store)?;
        // T-CONN: open is a TRANSPORT round-trip (connect / spawn / I/O) ⇒ TRANSIENT —
        // the server may simply be down; a retry can recover.
        let mut session = transport.open_session().map_err(|e| GatewayError::Dial {
            reason: e.to_string(),
            transient: true,
        })?;
        // PR-6b-3: capture the server's negotiated protocol version (recorded for
        // diagnostics — never a hard gate, so old `2025-06-18` + new `2026-07-28`
        // servers both dial successfully).
        let negotiated = session
            .initialize(wall_clock_ms)
            .map_err(|e| dial_error_of_session(&e))?;
        tracing::info!(
            server = %conn.name,
            negotiated_version = %if negotiated.is_empty() { "unspecified" } else { &negotiated },
            session_mode = conn.session_mode.tag(),
            "dialed external MCP server"
        );
        session
            .list_tools(DISCOVERY_MAX_BYTES, wall_clock_ms)
            .map_err(|e| dial_error_of_session(&e))
    }

    /// Dial a server, `tools/list`, and register each discovered tool into the
    /// registry + the broker. Returns the count actually registered. A per-tool
    /// registration failure is logged + skipped (best-effort) rather than failing
    /// the whole dial, so `count` (and the folded health) reflect what truly
    /// registered — never a hard zero while some tools are live (review #7).
    ///
    /// Re-vets the egress host against the LIVE allowlist on EVERY (re-)dial — the
    /// shared path behind `discover` and `redial_persisted` — so an operator's
    /// allowlist tightening is retroactive on the next dial/restart, not just at
    /// first admission. A redial is a fresh admission, not an inherited grant.
    fn dial_and_register(
        &self,
        conn: &Connection,
        wall_clock_ms: u64,
    ) -> Result<u32, GatewayError> {
        // Re-vet BEFORE acquiring a rate-limit token or opening a session, so a
        // now-disallowed host is refused without consuming either. Stdio has no
        // egress (`egress_host() == None`) and is skipped, as at admission. The
        // register_server admission re-vet is idempotent (same host, same list).
        if let Some(host) = conn.egress_host() {
            vet_registration_host(&host, &self.allowlist).map_err(GatewayError::HostRejected)?;
        }
        if !self.rate_limiter.try_acquire(&conn.name) {
            return Err(GatewayError::RateLimited(conn.name.clone()));
        }
        let decls = Self::probe(conn, wall_clock_ms, &self.secret_store)?;

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
            let cap_transport = build_transport(conn, &self.secret_store)?;
            self.sink.register_capability(Box::new(
                McpSessionCapability::new(
                        tool_id,
                        tool_version,
                        McpEndpointId(conn.transport.endpoint().to_string()),
                        remote.to_string(),
                        cap_transport,
                    )
                    // PR-6b-3: honor the connection's firing posture (stateless
                    // single-shot by default; reuse one session when stateful).
                    .with_stateful(conn.session_mode.is_stateful()),
            ));
            count += 1;
        }
        Ok(count)
    }
}

/// T-CONN: classify a `SessionError` from `initialize`/`tools/list` into a dial
/// failure with the right reachability flavor. A `Transport` fault (connect / I/O /
/// timeout / egress refusal) is TRANSIENT (retry-worthy); a `Decode` fault (the server
/// SPOKE but its reply was fail-closed-rejected — an incompatible / bad-spec server)
/// is PERMANENT (a retry can never fix it). Keeps `add`/`test`/`discover` — all routed
/// through `probe` — reporting the SAME flavor for the SAME failure.
fn dial_error_of_session(e: &kx_mcp::SessionError) -> GatewayError {
    GatewayError::Dial {
        reason: e.to_string(),
        transient: matches!(e, kx_mcp::SessionError::Transport(_)),
    }
}

/// Build a `kx-mcp` transport from a connection's spec + optional credential.
///
/// `secret_store` is the host-injected resolver (MM-3 keychain-then-env chain by
/// default the bare env passthrough) that the transport uses to turn the
/// connection's `credential_ref` NAME into the secret value transiently at dispatch
/// — the value is read inside the transport, injected into the child env / the
/// `Authorization` header, and dropped (D81; never journaled or stored).
fn build_transport(
    conn: &Connection,
    secret_store: &Arc<dyn SecretStore>,
) -> Result<Box<dyn McpTransport>, GatewayError> {
    match &conn.transport {
        TransportSpec::Stdio { command, args } => {
            let mut t = StdioTransport::new(command.as_str());
            for a in args {
                t = t.arg(a.as_str());
            }
            if let Some(secret_ref) = &conn.credential_ref {
                t = t
                    .credential(kx_mcp::CredentialRef::from_env_var(secret_ref.clone()))
                    .with_secret_store(secret_store.clone());
            }
            Ok(Box::new(t))
        }
        TransportSpec::Http { url, tls_required } => {
            let net_scope = conn.net_scope();
            let mut t = HttpTransport::new(url, &net_scope, *tls_required)
                .map_err(|e| GatewayError::InvalidSpec(e.to_string()))?;
            if let Some(secret_ref) = &conn.credential_ref {
                // The credential holds the FULL header value convention (e.g. "Bearer x").
                t = t
                    .header_credential(
                        "Authorization",
                        kx_mcp::CredentialRef::from_env_var(secret_ref.clone()),
                    )
                    .with_secret_store(secret_store.clone());
            }
            Ok(Box::new(t))
        }
    }
}

/// Maximum JSON-Schema nesting depth the mapper will accept (PR-6b-3 / RC
/// SEP-2106 security constraint). A schema deeper than this — or one carrying an
/// EXTERNAL `$ref` — is refused (mapped to `None`); the arg still passes through
/// verbatim for the SERVER to validate. Generous for real tool schemas, tight
/// enough to deny a pathological nested-schema DoS.
const MAX_SCHEMA_DEPTH: usize = 8;

/// Best-effort map of a remote tool's JSON-Schema `inputSchema` (the 2020-12 RC
/// shape, SEP-2106) into the typed registry [`InputSchema`] — the OPTIONAL
/// client-side arg gate. Maps the property kinds the runtime can validate exactly:
/// `string` (honouring `maxLength`, clamped ≤ 8192), `integer` (honouring
/// `minimum`/`maximum`), `boolean`, and a pure-string `enum` (→ exact-match
/// `Enum`). `number`/`array`/`object` and the `oneOf`/`anyOf`/`allOf` combinators
/// are NOT mapped — there is **no float in `ParamType`** (SN-8), and a structured
/// type has no typed gate — so those args pass through VERBATIM for the server to
/// validate authoritatively (`deny_unknown = false`, never a fabricated gate).
///
/// Security (the RC constraint): the schema is pre-scanned and REFUSED entirely
/// (→ `None`) if it nests deeper than [`MAX_SCHEMA_DEPTH`] or carries an external
/// `$ref` (a non-`#` pointer — defense against schema-driven SSRF/DoS). Returns
/// `None` when the schema is absent, not an object schema, or refused.
fn json_schema_to_input_schema(schema_json: &[u8]) -> Option<InputSchema> {
    if schema_json.is_empty() {
        return None;
    }
    let value: serde_json::Value = serde_json::from_slice(schema_json).ok()?;
    // RC security pre-scan: bound depth + refuse external `$ref` before mapping.
    if !schema_is_safe(&value, 0) {
        return None;
    }
    let obj = value.as_object()?;
    let properties = obj.get("properties")?.as_object()?;
    let required: std::collections::BTreeSet<&str> = obj
        .get("required")
        .and_then(|r| r.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    let mut params = Vec::new();
    for (pname, pspec) in properties {
        let Some(ty) = param_type_of(pspec) else {
            // number/array/object/combinator/absent → no typed gate; the arg
            // passes through verbatim (the server validates it).
            continue;
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

/// Map a single JSON-Schema property spec to a typed [`ParamType`], or `None` when
/// it has no exact typed gate (number/array/object/combinator → verbatim pass-through).
fn param_type_of(pspec: &serde_json::Value) -> Option<ParamType> {
    // A pure-string `enum` → exact-match Enum (SN-8: exact equality, no fuzzy
    // match). A mixed/numeric enum is left to the server (no partial allow-list).
    if let Some(values) = pspec.get("enum").and_then(|e| e.as_array()) {
        if !values.is_empty() && values.iter().all(serde_json::Value::is_string) {
            let allowed = values
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect::<std::collections::BTreeSet<String>>();
            return Some(ParamType::Enum { allowed });
        }
    }
    match pspec.get("type").and_then(|t| t.as_str()) {
        Some("string") => {
            let max_len = pspec
                .get("maxLength")
                .and_then(serde_json::Value::as_u64)
                .and_then(|n| usize::try_from(n.min(8192)).ok())
                .unwrap_or(8192);
            Some(ParamType::Str { max_len })
        }
        Some("integer") => Some(ParamType::Int {
            min: pspec.get("minimum").and_then(serde_json::Value::as_i64),
            max: pspec.get("maximum").and_then(serde_json::Value::as_i64),
        }),
        Some("boolean") => Some(ParamType::Bool),
        _ => None,
    }
}

/// Pre-scan a parsed schema for the RC security constraints: REFUSE an external
/// `$ref` (a non-`#`-prefixed pointer — schema-driven SSRF/fetch defense) and bound
/// nesting depth (DoS). Returns `true` iff the schema is SAFE to map. The walk is
/// itself depth-capped (bails at the limit without descending), so the scan can
/// never be turned into a stack-overflow vector.
fn schema_is_safe(value: &serde_json::Value, depth: usize) -> bool {
    if depth > MAX_SCHEMA_DEPTH {
        return false;
    }
    match value {
        serde_json::Value::Object(map) => {
            if let Some(r) = map.get("$ref").and_then(|v| v.as_str()) {
                // Internal same-document pointers (`#/...`) are tolerated (that prop
                // simply gets no typed gate); an EXTERNAL ref refuses the schema.
                if !r.starts_with('#') {
                    return false;
                }
            }
            map.values().all(|v| schema_is_safe(v, depth + 1))
        }
        serde_json::Value::Array(items) => items.iter().all(|v| schema_is_safe(v, depth + 1)),
        _ => true,
    }
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
    fn dial_error_classifies_transient_vs_permanent() {
        // T-CONN: a TRANSPORT fault (timeout / I/O / connect) is TRANSIENT — the
        // server may be down; a retry can recover.
        let timeout = dial_error_of_session(&kx_mcp::SessionError::Transport(
            kx_mcp::TransportError::Timeout { wall_clock_ms: 500 },
        ));
        assert!(
            matches!(
                timeout,
                GatewayError::Dial {
                    transient: true,
                    ..
                }
            ),
            "a transport timeout is a transient dial failure"
        );
        let io = dial_error_of_session(&kx_mcp::SessionError::Transport(
            kx_mcp::TransportError::Io("broken pipe".into()),
        ));
        assert!(matches!(
            io,
            GatewayError::Dial {
                transient: true,
                ..
            }
        ));
        // A DECODE fault (the server SPOKE but its reply was fail-closed-rejected — an
        // incompatible / bad-spec server) is PERMANENT — a retry can never fix it.
        let proto = dial_error_of_session(&kx_mcp::SessionError::Decode(
            kx_mcp::DecodeError::ProtocolError {
                code: -32601,
                message: "method not found".into(),
            },
        ));
        assert!(
            matches!(
                proto,
                GatewayError::Dial {
                    transient: false,
                    ..
                }
            ),
            "a protocol-error decode is a permanent dial failure"
        );
        // The flavor surfaces in the Display (the operator-facing detail).
        assert!(proto.to_string().contains("permanent"));
        assert!(timeout.to_string().contains("transient"));
    }

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
    fn json_schema_2020_12_enum_and_bounds() {
        // PR-6b-3: a pure-string enum maps to exact-match Enum; string maxLength
        // clamps; integer minimum/maximum become bounds.
        let schema = br#"{"type":"object","properties":{
            "mode":{"enum":["a","b","c"]},
            "name":{"type":"string","maxLength":32},
            "huge":{"type":"string","maxLength":99999},
            "count":{"type":"integer","minimum":1,"maximum":10}
        }}"#;
        let mapped = json_schema_to_input_schema(schema).unwrap();
        let by = |n: &str| {
            mapped
                .params
                .iter()
                .find(|p| p.name == n)
                .unwrap()
                .ty
                .clone()
        };
        match by("mode") {
            ParamType::Enum { allowed } => {
                assert_eq!(allowed.len(), 3);
                assert!(allowed.contains("b"));
            }
            other => panic!("expected Enum, got {other:?}"),
        }
        assert_eq!(by("name"), ParamType::Str { max_len: 32 });
        assert_eq!(
            by("huge"),
            ParamType::Str { max_len: 8192 },
            "clamped to the ceiling"
        );
        assert_eq!(
            by("count"),
            ParamType::Int {
                min: Some(1),
                max: Some(10)
            }
        );
    }

    #[test]
    fn json_schema_passes_through_combinators_verbatim() {
        // oneOf/anyOf/allOf + number have no typed gate → skipped (server validates).
        let schema = br#"{"type":"object","properties":{
            "either":{"oneOf":[{"type":"string"},{"type":"integer"}]},
            "amount":{"type":"number"},
            "ok":{"type":"boolean"}
        }}"#;
        let mapped = json_schema_to_input_schema(schema).unwrap();
        // only `ok` is client-validatable.
        assert_eq!(mapped.params.len(), 1);
        assert_eq!(mapped.params[0].name, "ok");
    }

    #[test]
    fn json_schema_refuses_external_ref() {
        // An external `$ref` (non-`#` pointer) refuses the WHOLE schema (SSRF/DoS
        // defense) — the arg still passes through verbatim for the server.
        let schema = br#"{"type":"object","properties":{
            "x":{"$ref":"https://evil.example/schema.json"}
        }}"#;
        assert!(json_schema_to_input_schema(schema).is_none());
        // An internal `#/...` pointer is tolerated (that prop just gets no gate).
        let internal = br##"{"type":"object","properties":{
            "x":{"$ref":"#/$defs/T"},
            "ok":{"type":"boolean"}
        }}"##;
        let mapped = json_schema_to_input_schema(internal).unwrap();
        assert_eq!(mapped.params.len(), 1);
        assert_eq!(mapped.params[0].name, "ok");
    }

    #[test]
    fn json_schema_refuses_overdeep_schema() {
        // Build a schema nested past MAX_SCHEMA_DEPTH → refused (DoS bound).
        let mut inner = String::from(r#"{"type":"string"}"#);
        for _ in 0..(MAX_SCHEMA_DEPTH + 3) {
            inner = format!(r#"{{"type":"object","properties":{{"n":{inner}}}}}"#);
        }
        assert!(
            json_schema_to_input_schema(inner.as_bytes()).is_none(),
            "a schema deeper than MAX_SCHEMA_DEPTH must be refused"
        );
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
