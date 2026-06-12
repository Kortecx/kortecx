//! The model-discovery read seam behind `ListModels` (Batch A).
//!
//! Display/discovery ONLY: model *selection* stays a recipe ENUM free-param
//! validated server-side at binding — nothing returned here authorizes
//! anything (the toolscout advisory precedent, SN-8). Spoken in gateway-core's
//! own wire vocabulary so no `kx-model-store` type crosses the seam; the host
//! builds its catalog from the descriptors it actually provisioned.

use crate::error::GatewayError;

/// One discoverable model, as display fields (mirrors `proto::ModelSummary`).
#[derive(Clone, Debug)]
pub struct ModelSummaryEntry {
    /// The model's id (the value a recipe `model` ENUM free-param accepts).
    pub model_id: String,
    /// Display modality strings: `"text"` | `"image"` | `"audio"` | `"video"`.
    pub modalities: Vec<String>,
    /// Host-synthesized display prose (GGUF name / file stem) — never identity.
    pub description: String,
    /// `true` iff this model backs the live serve loop right now.
    pub serving: bool,
    /// The served context window in tokens.
    pub context_len: u32,
}

/// The model-catalog read seam. The host implements it over the model registry
/// it provisioned at startup; an FFI-free build returns an EMPTY list (honest
/// discovery — "no models on this serve" is an answer, not an error).
pub trait ModelCatalogView: Send + Sync {
    /// Every discoverable model, in a stable display order.
    ///
    /// # Errors
    /// A host read failure ([`GatewayError::Internal`]).
    fn list(&self) -> Result<Vec<ModelSummaryEntry>, GatewayError>;
}
