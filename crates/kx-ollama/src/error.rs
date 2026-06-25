//! [`OllamaError`] — the client's failure classing, mirroring the transient vs
//! permanent split the T-CONN reachability work established for the MCP gateway.
//!
//! The classing is what the host's auto-detect uses: an [`OllamaError::Unreachable`]
//! at probe time means "Ollama is absent → print guidance"; a [`OllamaError::Status`]
//! or [`OllamaError::Protocol`] means the daemon answered but something is wrong.

/// A failure talking to the Ollama daemon.
#[derive(Debug, thiserror::Error)]
pub enum OllamaError {
    /// The endpoint URL is not a valid loopback `http(s)` URL, or it is a
    /// non-loopback host and the operator did not opt in (`allow_remote`).
    /// Construction-time refusal (SN-8): a mis-scoped client is never built.
    #[error("ollama endpoint refused: {0}")]
    Refused(String),

    /// The daemon could not be reached (connection refused, DNS failure, TLS
    /// failure). At probe time this is the "Ollama is not running" signal —
    /// transient / expected, the host degrades to guidance.
    #[error("ollama unreachable: {0}")]
    Unreachable(String),

    /// The wall-clock budget elapsed before the daemon responded.
    #[error("ollama timeout after {wall_clock_ms} ms")]
    Timeout {
        /// The wall-clock ceiling, in milliseconds.
        wall_clock_ms: u64,
    },

    /// The daemon returned a non-success HTTP status.
    #[error("ollama http status {0}")]
    Status(u16),

    /// The daemon answered but the body could not be decoded (malformed JSON,
    /// missing field, oversize). Fail-closed: never a silent partial result.
    #[error("ollama protocol error: {0}")]
    Protocol(String),
}

impl OllamaError {
    /// `true` iff this failure means the daemon is absent / unreachable (the
    /// probe-time "degrade to guidance" signal), as opposed to a daemon that
    /// answered with an error.
    #[must_use]
    pub fn is_absent(&self) -> bool {
        matches!(self, Self::Unreachable(_) | Self::Refused(_))
    }
}
