//! Typed gateway errors and their mapping to [`tonic::Status`]. The load-bearing
//! property: **[`GatewayError::NotAuthorized`] always maps to the identical
//! `permission_denied("not authorized")`** regardless of cause (wrong run, ref
//! not owned, run unregistered) — no existence oracle (D102.1 / D120.1).

use tonic::Status;

/// An error surfaced by a gateway RPC.
#[derive(Debug, thiserror::Error)]
pub enum GatewayError {
    /// The caller is not authorized for this request. ONE uniform variant for
    /// "wrong instance_id" / "ref not in this run" / "run unregistered" so the
    /// error reveals nothing about what exists (no existence oracle).
    #[error("not authorized")]
    NotAuthorized,

    /// A malformed request field (wrong-length hash / instance_id, missing
    /// required sub-message).
    #[error("invalid request: {0}")]
    InvalidArgument(&'static str),

    /// An internal read/fold failure (journal or projection error). Only
    /// reachable AFTER an ownership check passes, so it is never an oracle.
    #[error("internal gateway error: {0}")]
    Internal(String),

    /// A request exceeded a fail-closed server resource cap (the Batch A
    /// `PutContent` payload cap). Checked BEFORE any store/journal touch.
    #[error("resource exhausted: {0}")]
    ResourceExhausted(&'static str),

    /// A named entity does not exist WITHIN a scope the caller already owns
    /// (Batch B: an unknown `mote_id` in an owned run). Only reachable AFTER
    /// an ownership check passes — the owner can already enumerate the scope
    /// (`GetProjection`), so this is honest, never an existence oracle.
    #[error("not found: {0}")]
    NotFound(&'static str),
}

impl From<GatewayError> for Status {
    fn from(err: GatewayError) -> Self {
        match err {
            // Uniform message — never branch on the cause.
            GatewayError::NotAuthorized => Status::permission_denied("not authorized"),
            GatewayError::InvalidArgument(msg) => Status::invalid_argument(msg),
            GatewayError::Internal(msg) => Status::internal(msg),
            GatewayError::ResourceExhausted(msg) => Status::resource_exhausted(msg),
            GatewayError::NotFound(msg) => Status::not_found(msg),
        }
    }
}

/// Build an [`GatewayError::Internal`] from any displayable error.
pub(crate) fn internal<E: std::fmt::Display>(err: E) -> GatewayError {
    GatewayError::Internal(err.to_string())
}

/// Parse a 16-byte `instance_id` from a request field.
pub(crate) fn instance_id_16(bytes: &[u8]) -> Result<[u8; 16], GatewayError> {
    bytes
        .try_into()
        .map_err(|_| GatewayError::InvalidArgument("instance_id must be 16 bytes"))
}

/// Parse a 32-byte hash from a request field, naming the field on failure.
pub(crate) fn hash_32(bytes: &[u8], what: &'static str) -> Result<[u8; 32], GatewayError> {
    bytes
        .try_into()
        .map_err(|_| GatewayError::InvalidArgument(what))
}
