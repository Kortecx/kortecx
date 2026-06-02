//! Typed refusal vocabulary for the MCP adapter: [`DecodeError`] (the fail-closed
//! inbound decoder's verdict) + [`TransportError`] (a transport round-trip failure).
//! Both map into [`kx_capability::CapabilityFailureReason`] so the broker surfaces a
//! typed failure — never a panic, never a silent accept.

use kx_capability::CapabilityFailureReason;
use thiserror::Error;

/// Why a JSON-RPC `tools/call` response was refused.
///
/// Every variant is a **fail-closed** outcome: an MCP server is untrusted, so a
/// response that is oversized, malformed, truncated, or an explicit protocol error
/// is refused rather than staged. The decoder is total over arbitrary bytes (it
/// never panics) — see [`crate::decode_tool_result`].
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DecodeError {
    /// The response exceeded the per-call size cap (IMP-16 — resource-exhaustion
    /// guard). Carries the observed and maximum byte counts.
    #[error("MCP response too large: {got} bytes > cap {max}")]
    Oversize {
        /// Observed response size in bytes.
        got: usize,
        /// The configured cap.
        max: usize,
    },
    /// The response was not a well-formed JSON-RPC `tools/call` result: not JSON,
    /// truncated, missing the `result` member, or a shape the fixed decoder does
    /// not accept. Untrusted input is never coerced — this is the refusal.
    #[error("malformed MCP response: {diagnostic}")]
    Malformed {
        /// A short, non-sensitive diagnostic (never echoes the raw payload).
        diagnostic: String,
    },
    /// The server returned a JSON-RPC `error` object (a well-formed protocol-level
    /// failure). Carries the server's code + message for diagnostics.
    #[error("MCP server error {code}: {message}")]
    ProtocolError {
        /// The JSON-RPC error code.
        code: i64,
        /// The JSON-RPC error message (server-supplied diagnostic).
        message: String,
    },
}

/// Why an MCP transport round-trip failed (before any response could be decoded):
/// the subprocess could not be spawned, stdin/stdout I/O failed, or the call
/// exceeded its wall-clock budget.
#[derive(Debug, Error)]
pub enum TransportError {
    /// The transport could not be established (subprocess spawn failed, endpoint
    /// unreachable). Carries a diagnostic; maps to `NetworkUnreachable`.
    #[error("MCP transport unreachable: {0}")]
    Unreachable(String),
    /// An I/O error writing the request or reading the response.
    #[error("MCP transport I/O error: {0}")]
    Io(String),
    /// The round-trip exceeded the per-call wall-clock budget. Maps to `Timeout`.
    #[error("MCP transport timed out after {wall_clock_ms} ms")]
    Timeout {
        /// The budget that was exceeded.
        wall_clock_ms: u64,
    },
}

impl From<DecodeError> for CapabilityFailureReason {
    fn from(e: DecodeError) -> Self {
        match e {
            // Both an oversized and a malformed body are "the response did not match
            // the expected shape" from the broker's vocabulary.
            DecodeError::Oversize { .. } | DecodeError::Malformed { .. } => {
                CapabilityFailureReason::InvalidResponse
            }
            // A server-side protocol error is a downstream failure; carry the detail.
            DecodeError::ProtocolError { code, message } => {
                CapabilityFailureReason::Other(format!("MCP error {code}: {message}"))
            }
        }
    }
}

impl From<TransportError> for CapabilityFailureReason {
    fn from(e: TransportError) -> Self {
        match e {
            TransportError::Unreachable(_) => CapabilityFailureReason::NetworkUnreachable,
            TransportError::Timeout { .. } => CapabilityFailureReason::Timeout,
            TransportError::Io(msg) => CapabilityFailureReason::Other(msg),
        }
    }
}
