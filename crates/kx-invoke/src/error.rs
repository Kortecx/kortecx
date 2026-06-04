//! Typed inbound-execution errors.
//!
//! **No-oracle note for the M8 inbound server:** when surfacing these to an
//! UNTRUSTED inbound caller (an external MCP client), the host MUST collapse
//! [`InvokeError::Unauthorized`] / [`InvokeError::NotFound`] /
//! [`InvokeError::NotAWorkflow`] / [`InvokeError::BodyUnavailable`] into ONE
//! uniform "not authorized" (no existence oracle — a caller must not learn what
//! recipes exist or are published). The distinct variants exist for trusted
//! hosts (logging / operator surfaces).

use kx_catalog::AdvertiseError;
use kx_gateway_core::SubmitterError;
use kx_warrant::NarrowingError;

/// A failure binding or executing an inbound snapshot invocation.
#[derive(Debug, thiserror::Error)]
pub enum InvokeError {
    /// The caller is not authorized to `Use` (or `Read`) this recipe.
    #[error("not authorized")]
    Unauthorized,
    /// The handle resolves to no published version (caller IS Read-authorized).
    #[error("recipe not found")]
    NotFound,
    /// The published content is not an executable workflow recipe.
    #[error("handle does not resolve to a workflow recipe")]
    NotAWorkflow,
    /// No executable body is stored for the resolved recipe identity.
    #[error("recipe body unavailable")]
    BodyUnavailable,
    /// Building the input schema from the free-param contract failed.
    #[error("recipe schema error: {0}")]
    Schema(#[from] AdvertiseError),
    /// The supplied arguments failed validation (type / range / unknown field).
    /// Carries the `kx_tool_registry::SchemaError` rendered (that type is not an
    /// `std::error::Error`, so it is captured as a string here).
    #[error("argument validation failed: {0}")]
    ArgValidation(String),
    /// The supplied arguments were not a decodable JSON object.
    #[error("arguments are not a JSON object: {0}")]
    ArgParse(String),
    /// A declared variable slot maps to no step in the recipe body — fail-closed
    /// (a silent drop could run the recipe with an unbound parameter).
    #[error("variable slot '{0}' is declared by no recipe step")]
    SlotUnbound(String),
    /// A declared variable slot had no supplied value (defensive; validation
    /// should have caught it).
    #[error("variable slot '{0}' has no supplied value")]
    SlotMissing(String),
    /// The recipe body does not compile.
    #[error("recipe body does not compile: {0}")]
    Uncompilable(String),
    /// The recipe compiled to zero Motes — nothing to run.
    #[error("recipe is empty")]
    EmptyRecipe,
    /// Narrowing the caller's effective warrant against a step's declared warrant
    /// failed — the caller cannot run a recipe that needs more than they hold.
    #[error("warrant narrowing failed: {0}")]
    WarrantNarrowing(#[from] NarrowingError),
    /// Proxying the run to the coordinator failed.
    #[error("submit failed: {0}")]
    Submit(#[from] SubmitterError),
}
