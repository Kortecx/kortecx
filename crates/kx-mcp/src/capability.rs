//! [`McpCapability`] — the concrete `ToolKind::Mcp` dispatch and the first
//! production [`kx_capability::Capability`] impl.
//!
//! `invoke` frames a JSON-RPC `tools/call` from the (already warrant-validated)
//! `EffectRequest.payload`, runs it over the configured [`McpTransport`], and
//! decodes the response fail-closed + size-capped. The broker
//! (`LocalCapabilityBroker`) is what enforces the warrant (net_scope ⊆ warrant,
//! tool_grants, pattern) before calling `invoke` and what stages the returned bytes
//! — this type only produces the effect's result bytes.

use kx_capability::{Capability, CapabilityFailureReason, EffectRequest};
use kx_mote::{EffectPattern, ToolName, ToolVersion};
use kx_tool_registry::McpEndpointId;
use serde_json::value::RawValue;

use crate::decode::{decode_tool_result, MAX_TOOL_RESULT_BYTES_DEFAULT};
use crate::jsonrpc::ToolsCallRequest;
use crate::transport::McpTransport;

/// MCP effects are world-mutating by default → `StageThenCommit` (D66). A server
/// that documents an idempotency key can be modelled with a tighter
/// `IdempotencyClass` later, but the *pattern* surface stays staged-then-commit.
const SUPPORTED_PATTERNS: [EffectPattern; 1] = [EffectPattern::StageThenCommit];

/// The concrete `ToolKind::Mcp` capability: a thin MCP client behind the
/// [`Capability`] trait.
pub struct McpCapability {
    name: ToolName,
    version: ToolVersion,
    #[allow(dead_code)] // Carried for identity + the M5.2b net_scope host derivation.
    endpoint: McpEndpointId,
    remote_name: String,
    transport: Box<dyn McpTransport>,
    max_response_bytes: usize,
    wall_clock_ms: u64,
}

impl std::fmt::Debug for McpCapability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `dyn McpTransport` is not `Debug`; elide it.
        f.debug_struct("McpCapability")
            .field("name", &self.name)
            .field("version", &self.version)
            .field("endpoint", &self.endpoint)
            .field("remote_name", &self.remote_name)
            .field("max_response_bytes", &self.max_response_bytes)
            .field("wall_clock_ms", &self.wall_clock_ms)
            .finish_non_exhaustive()
    }
}

impl McpCapability {
    /// Build an MCP capability named `name`@`version` that calls the remote tool
    /// `remote_name` at `endpoint` over `transport`.
    ///
    /// The response size cap defaults to [`MAX_TOOL_RESULT_BYTES_DEFAULT`] and the
    /// per-call wall-clock budget defaults to the transport's own fallback; use
    /// [`with_max_response_bytes`](Self::with_max_response_bytes) /
    /// [`with_wall_clock_ms`](Self::with_wall_clock_ms) to bound them from the
    /// warrant (IMP-16).
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

    /// Bound the response size (IMP-16). A response larger than this is refused as
    /// `InvalidResponse` and nothing is staged. A `0` argument is treated as the
    /// default cap (never "unbounded").
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

impl Capability for McpCapability {
    fn name(&self) -> &ToolName {
        &self.name
    }

    fn version(&self) -> &ToolVersion {
        &self.version
    }

    fn supported_patterns(&self) -> &[EffectPattern] {
        &SUPPORTED_PATTERNS
    }

    fn invoke(&self, request: &EffectRequest) -> Result<Vec<u8>, CapabilityFailureReason> {
        // The args are the validated `EffectRequest.payload`, carried VERBATIM as an
        // opaque RawValue (never decoded into a dynamic Value / floats). An empty
        // payload means "no arguments" → `{}`.
        let args: &RawValue = if request.payload.is_empty() {
            serde_json::from_str("{}").map_err(|e| {
                CapabilityFailureReason::Other(format!("internal: empty-args encode: {e}"))
            })?
        } else {
            serde_json::from_slice(&request.payload).map_err(|e| {
                CapabilityFailureReason::Other(format!("tool args are not valid JSON: {e}"))
            })?
        };

        let rpc = ToolsCallRequest::new(1, &self.remote_name, args);
        let request_bytes = serde_json::to_vec(&rpc).map_err(|e| {
            CapabilityFailureReason::Other(format!("internal: request encode: {e}"))
        })?;

        tracing::debug!(remote = %self.remote_name, "mcp tools/call dispatch");

        // Untrusted round-trip: bounded read + wall-clock watchdog in the transport.
        let response = self.transport.round_trip(
            &request_bytes,
            self.max_response_bytes,
            self.wall_clock_ms,
        )?;

        // Fail-closed inbound decode (IMP-5 / IMP-16). Returns the result object's
        // verbatim bytes on success; any malformed / oversize / error response is a
        // typed failure, never silently accepted.
        let result = decode_tool_result(&response, self.max_response_bytes)?;
        Ok(result)
    }
}
