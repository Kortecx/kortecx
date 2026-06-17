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
    /// unreachable, egress refusal, timeout, or a fail-closed decode of an
    /// untrusted reply.
    #[error("dial failed: {0}")]
    Dial(String),
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
