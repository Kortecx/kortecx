//! [`ResolutionError`] + [`RegistrationError`] — typed refusal vocabularies
//! for [`crate::ToolRegistry::resolve`] and the register / approve pair.

use kx_mote::{ToolName, ToolVersion};
use kx_warrant::WarrantField;

use crate::ids::{McpEndpointId, RegistrationToken};

/// Reason [`crate::ToolRegistry::resolve`] refused.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum ResolutionError {
    /// No tool registered with this `(tool_id, tool_version)`.
    #[error("tool not found in registry: {tool_id:?}@{tool_version:?}")]
    NotFound {
        /// The requested tool id.
        tool_id: ToolName,
        /// The requested version.
        tool_version: ToolVersion,
    },
    /// The tool's required capability exceeds the warrant on this axis.
    #[error("tool requirement exceeds warrant on field {axis:?}")]
    CapabilityExceedsWarrant {
        /// The axis that was exceeded.
        axis: WarrantField,
    },
    /// MCP endpoint unreachable. Reserved for future use (the broker performs
    /// the actual reachability check; the registry surfaces this when
    /// short-circuiting at resolution).
    #[error("MCP endpoint unreachable: {endpoint:?}")]
    McpUnreachable {
        /// The unreachable endpoint.
        endpoint: McpEndpointId,
    },
    /// The tool exists but is `PendingHumanReview` — INERT until reviewed.
    #[error("registration pending human review: token={token:?}")]
    PendingHumanReview {
        /// The pending registration's token.
        token: RegistrationToken,
    },
}

/// Reason a registration operation failed.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum RegistrationError {
    /// At approve time, `def.required_capability` is not a subset of the
    /// `generating_lineage_warrant` on this axis. Anti-privilege-laundering
    /// guard.
    #[error("self-generated tool's required capability exceeds the generating lineage's warrant on field {axis:?}")]
    InvalidLineageSubset {
        /// The axis where the subset check failed.
        axis: WarrantField,
    },
    /// The token doesn't match any registration in the registry.
    #[error("registration token unknown: {token:?}")]
    UnknownToken {
        /// The unknown token.
        token: RegistrationToken,
    },
    /// Approve was called on a registration that is already approved.
    #[error("registration already approved: {token:?}")]
    AlreadyApproved {
        /// The token whose registration was already approved.
        token: RegistrationToken,
    },
    /// A `HumanAuthored` registration cannot be approved separately — it is
    /// approved at registration. Surfaces if a caller tries to call
    /// `approve_registration` for a HumanAuthored token.
    #[error("HumanAuthored registrations are approved at register-time; nothing to do")]
    NotPendingReview {
        /// The token whose registration is not in PendingHumanReview state.
        token: RegistrationToken,
    },
    /// A durable-store (SQLite) open / I/O / SQL / corrupt-row failure. Only the
    /// durable [`crate::SqliteToolRegistry`] raises this; the in-memory registry
    /// never touches a store. Fail-closed: a write that cannot durably commit is
    /// surfaced rather than silently lost.
    #[error("tool registry storage error: {0}")]
    Storage(String),
}
