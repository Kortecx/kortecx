//! The model-lifecycle CONTROL seam behind `LoadModel`/`OffloadModel` (POC-3).
//!
//! Unlike [`ModelCatalogView`](crate::ModelCatalogView) (display-only discovery),
//! this seam MUTATES RAM residency: it warms/evicts a model in the owner-thread
//! LRU. It is scoped to the server's FIXED registered set — an unregistered
//! `model_id` is [`GatewayError::NotFound`] (fail-closed; the host NEVER warms an
//! arbitrary path). Off-journal / off-digest: residency is ephemeral RAM state
//! that rebuilds EMPTY on restart, so these controls write no journal fact.
//! Spoken in gateway-core's own vocabulary so no `kx-inference` type crosses the
//! seam.

use crate::error::GatewayError;

/// The outcome of a load/offload control op (mirrors the proto responses).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelLifecycleOutcome {
    /// The model the op targeted (echoed back).
    pub model_id: String,
    /// Residency AFTER the op (`true` after a successful load, `false` after
    /// a successful offload).
    pub loaded: bool,
    /// Residency BEFORE the op. On load, `false` ⇒ a cold load happened; on
    /// offload, `false` ⇒ an idempotent no-op (it was not resident).
    pub was_resident: bool,
}

/// The model-lifecycle control seam. The host implements it over the concrete
/// inference backend it provisioned at startup; a build with no backend leaves it
/// unwired (the RPC then returns `Unimplemented`, the `GetServerInfo` precedent).
pub trait ModelLifecycleControl: Send + Sync {
    /// Warm a REGISTERED model into RAM (a real cold load on the owner thread).
    /// Over-capacity ⇒ honest LRU-evict-oldest (sequential swap).
    ///
    /// # Errors
    /// [`GatewayError::NotFound`] if `model_id` is not in the registered set
    /// (fail-closed); [`GatewayError::Internal`] on a backend/load failure.
    fn load(&self, model_id: &str) -> Result<ModelLifecycleOutcome, GatewayError>;

    /// Evict a REGISTERED model from RAM (a real `llama_model_free`). Idempotent:
    /// a not-resident model offloads to `was_resident = false`.
    ///
    /// # Errors
    /// [`GatewayError::NotFound`] if `model_id` is not in the registered set
    /// (fail-closed); [`GatewayError::Internal`] on a backend failure.
    fn offload(&self, model_id: &str) -> Result<ModelLifecycleOutcome, GatewayError>;
}
