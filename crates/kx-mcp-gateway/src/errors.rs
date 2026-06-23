//! The external MCP gateway's typed refusal vocabulary.

use thiserror::Error;

/// Why a gateway operation (register / discover / test / dial) failed. Every
/// variant is a fail-closed outcome — a server is untrusted and a dial is the
/// live egress surface, so anything unexpected is a typed refusal, never a
/// silent success.
#[derive(Debug, Error)]
pub enum GatewayError {
    /// The server's `server_host` was refused at admission (SSRF deny-by-default:
    /// internal / link-local / metadata / not in the operator allowlist).
    #[error("server host refused: {0}")]
    HostRejected(String),
    /// The endpoint / transport spec was malformed (bad URL, empty command, …).
    #[error("invalid connection spec: {0}")]
    InvalidSpec(String),
    /// A live dial (`initialize` / `tools/list` / `tools/call`) failed — transport
    /// unreachable, egress refusal, timeout, or a fail-closed decode of an untrusted
    /// reply. T-CONN: `transient` classifies the flavor so `add`/`test`/`discover`
    /// report a CONSISTENT reachability verdict (they all route through the one
    /// `probe`): a TRANSIENT failure (a network-level fault — connect/IO/timeout — the
    /// server may simply be down; retry-worthy) vs a PERMANENT one (the server SPOKE
    /// but its handshake/discovery reply was fail-closed-rejected — an incompatible /
    /// bad-spec server a retry can never fix).
    #[error("dial failed ({}): {reason}", if *transient { "transient" } else { "permanent" })]
    Dial {
        /// The diagnostic detail.
        reason: String,
        /// `true` = a retry-worthy network fault; `false` = a permanent protocol fault.
        transient: bool,
    },
    /// The per-server rate-limit / concurrency budget was exceeded.
    #[error("rate limit exceeded for server {0}")]
    RateLimited(String),
    /// A durable write to the connections sidecar or the tool registry failed.
    #[error("storage error: {0}")]
    Storage(String),
    /// No connection with the given name is registered.
    #[error("no such MCP server: {0}")]
    NotFound(String),
}
