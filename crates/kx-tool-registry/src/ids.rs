//! Identifier newtypes used throughout the registry.

use kx_content::ContentRef;
use serde::{Deserialize, Serialize};

/// Identifier for an MCP endpoint registered with this registry.
///
/// Opaque string; the registry treats it as a handle. The actual MCP protocol
/// dispatch is the broker's responsibility (P1.8.5); this crate only carries
/// the identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct McpEndpointId(pub String);

/// Identifier for a human reviewer authorized to approve self-generated tools.
///
/// Opaque string (likely an org email or user id in real deployments).
/// Tracked in the registry's audit log; not enforcement-bearing in v0.1.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ReviewerId(pub String);

/// Content-addressed token returned by [`crate::ToolRegistry::register`].
///
/// `RegistrationToken = blake3(canonical_bincode((ToolDef, ToolProvenance)))`.
/// Deterministic: re-submitting the same `(def, provenance)` produces the same
/// token. The token is the registry's primary key for the pending registration
/// (used by [`crate::ToolRegistry::approve_registration`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RegistrationToken(pub ContentRef);
