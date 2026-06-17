//! PR-6b-1 — the stateful MCP session seam + the session-firing capability.
//!
//! The M5.2a [`crate::McpTransport`] is **single-shot**: one request, one
//! response (a fresh subprocess per call for stdio). That is exactly right for
//! the bundled handshake-free `mcp-echo` tool, but a real EXTERNAL MCP server
//! expects the lifecycle handshake `initialize → tools/list → tools/call` over
//! ONE live connection. [`McpSession`] adds that, behind an additive
//! [`crate::McpTransport::open_session`] seam — the existing
//! [`crate::McpCapability`] path (over `round_trip`) is BYTE-IDENTICAL because it
//! never opens a session.
//!
//! Two consumers:
//! - **Discovery** (the `kx-mcp-gateway` crate): open a session, `initialize`,
//!   `tools/list`, close — map each declaration into the durable tool registry.
//! - **Firing** ([`McpSessionCapability`]): a [`kx_capability::Capability`] that,
//!   per `invoke`, opens a SHORT-LIVED session (`initialize` → `tools/call` →
//!   drop). Per-invoke statelessness keeps the runtime's exactly-once contract
//!   (each Mote dispatch is independent + idempotent via the run-scoped key);
//!   the handshake makes it correct against session-requiring servers.
//!
//! MCP stays UNTRUSTED: every inbound payload is decoded fail-closed + size-capped
//! through [`crate::decode`]; secrets ride out-of-band (D81); effects are
//! `StageThenCommit` (D66). The warrant gate (net_scope ⊆ warrant, tool_grants,
//! pattern) is enforced by the broker's `precheck` before `invoke` — this type
//! adds a body, never a second authority gate.

use kx_capability::{Capability, CapabilityFailureReason, EffectRequest};
use kx_mote::{EffectPattern, ToolName, ToolVersion};
use kx_tool_registry::McpEndpointId;

use crate::decode::{RemoteToolDecl, MAX_TOOL_RESULT_BYTES_DEFAULT};
use crate::errors::{DecodeError, TransportError};
use crate::transport::McpTransport;

/// MCP effects are world-mutating by default → `StageThenCommit` (D66) — the same
/// pattern surface the single-shot [`crate::McpCapability`] exposes.
const SUPPORTED_PATTERNS: [EffectPattern; 1] = [EffectPattern::StageThenCommit];

/// A session-level failure: either the transport round-trip failed, or the
/// untrusted response decoded fail-closed. Both map into the broker's typed
/// [`CapabilityFailureReason`] vocabulary (never a panic, never a silent accept).
#[derive(Debug)]
pub enum SessionError {
    /// A transport round-trip failure (spawn/connect, I/O, timeout, egress refusal).
    Transport(TransportError),
    /// The untrusted response was refused by the fail-closed decoder.
    Decode(DecodeError),
}

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionError::Transport(e) => write!(f, "{e}"),
            SessionError::Decode(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for SessionError {}

impl From<TransportError> for SessionError {
    fn from(e: TransportError) -> Self {
        SessionError::Transport(e)
    }
}

impl From<DecodeError> for SessionError {
    fn from(e: DecodeError) -> Self {
        SessionError::Decode(e)
    }
}

impl From<SessionError> for CapabilityFailureReason {
    fn from(e: SessionError) -> Self {
        match e {
            SessionError::Transport(t) => t.into(),
            SessionError::Decode(d) => d.into(),
        }
    }
}

/// A stateful MCP session: the lifecycle handshake + discovery + tool calls over
/// ONE live connection. Built via [`McpTransport::open_session`]; dropping it
/// closes the connection (reaps the stdio child / releases the HTTP session).
///
/// `Send` (not `Sync`): a session carries per-connection mutable state (the live
/// child / the request-id counter), used single-threaded by one caller at a time.
pub trait McpSession: Send {
    /// Send the MCP `initialize` handshake and verify the server's reply is a
    /// well-formed result (fail-closed). Sent once, right after open.
    fn initialize(&mut self, wall_clock_ms: u64) -> Result<(), SessionError>;

    /// Send `tools/list` and decode the server's tool declarations fail-closed.
    fn list_tools(
        &mut self,
        max_response_bytes: usize,
        wall_clock_ms: u64,
    ) -> Result<Vec<RemoteToolDecl>, SessionError>;

    /// Send `tools/call` for `remote_name` with verbatim `arguments` JSON bytes
    /// and decode the result object fail-closed. An empty `arguments` slice means
    /// "no arguments" (`{}`).
    fn call(
        &mut self,
        remote_name: &str,
        arguments: &[u8],
        max_response_bytes: usize,
        wall_clock_ms: u64,
        idempotency_key: Option<&[u8; 32]>,
    ) -> Result<Vec<u8>, SessionError>;
}

/// A [`Capability`] that fires an external MCP tool over a SHORT-LIVED stateful
/// session — opened per `invoke`, handshaken, called, then dropped.
///
/// Distinct from [`crate::McpCapability`] (which is single-shot `round_trip`, for
/// the handshake-free bundled echo tool): the gateway registers THIS for
/// dialed external tools so a session-requiring server is handled correctly,
/// while the bundled-tool path stays byte-identical.
pub struct McpSessionCapability {
    name: ToolName,
    version: ToolVersion,
    #[allow(dead_code)] // Carried for identity + diagnostics.
    endpoint: McpEndpointId,
    remote_name: String,
    transport: Box<dyn McpTransport>,
    max_response_bytes: usize,
    wall_clock_ms: u64,
}

impl std::fmt::Debug for McpSessionCapability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpSessionCapability")
            .field("name", &self.name)
            .field("version", &self.version)
            .field("endpoint", &self.endpoint)
            .field("remote_name", &self.remote_name)
            .field("max_response_bytes", &self.max_response_bytes)
            .field("wall_clock_ms", &self.wall_clock_ms)
            .finish_non_exhaustive()
    }
}

impl McpSessionCapability {
    /// Build a session-firing capability `name`@`version` that calls the remote
    /// tool `remote_name` at `endpoint` over `transport` (opening a fresh session
    /// per invoke).
    #[must_use]
    pub fn new(
        name: ToolName,
        version: ToolVersion,
        endpoint: McpEndpointId,
        remote_name: impl Into<String>,
        transport: Box<dyn McpTransport>,
    ) -> Self {
        Self {
            name,
            version,
            endpoint,
            remote_name: remote_name.into(),
            transport,
            max_response_bytes: MAX_TOOL_RESULT_BYTES_DEFAULT,
            wall_clock_ms: 0,
        }
    }

    /// Bound the response size (IMP-16). `0` ⇒ the default cap (never "unbounded").
    #[must_use]
    pub fn with_max_response_bytes(mut self, max_bytes: usize) -> Self {
        self.max_response_bytes = if max_bytes == 0 {
            MAX_TOOL_RESULT_BYTES_DEFAULT
        } else {
            max_bytes
        };
        self
    }

    /// Set the per-call wall-clock budget (ms) — typically the warrant's
    /// `resource_ceiling.wall_clock_ms`. `0` lets the transport apply its default.
    #[must_use]
    pub fn with_wall_clock_ms(mut self, wall_clock_ms: u64) -> Self {
        self.wall_clock_ms = wall_clock_ms;
        self
    }
}

impl Capability for McpSessionCapability {
    fn name(&self) -> &ToolName {
        &self.name
    }

    fn version(&self) -> &ToolVersion {
        &self.version
    }

    fn supported_patterns(&self) -> &[EffectPattern] {
        &SUPPORTED_PATTERNS
    }

    fn required_secret_scope(&self) -> kx_warrant::SecretScope {
        self.transport.declared_secret_scope()
    }

    fn invoke(&self, request: &EffectRequest) -> Result<Vec<u8>, CapabilityFailureReason> {
        // Open a fresh short-lived session, handshake, fire, drop. The args are the
        // already-warrant-validated `EffectRequest.payload`, carried verbatim.
        tracing::debug!(remote = %self.remote_name, "mcp session tools/call dispatch");
        let mut session = self.transport.open_session().map_err(|e| {
            // A transport that does not support sessions (or a failed open/spawn)
            // is a typed transport failure — never a silent success.
            CapabilityFailureReason::from(e)
        })?;
        session
            .initialize(self.wall_clock_ms)
            .map_err(CapabilityFailureReason::from)?;
        let result = session
            .call(
                &self.remote_name,
                &request.payload,
                self.max_response_bytes,
                self.wall_clock_ms,
                request.idempotency_key.as_ref(),
            )
            .map_err(CapabilityFailureReason::from)?;
        Ok(result)
    }
}
