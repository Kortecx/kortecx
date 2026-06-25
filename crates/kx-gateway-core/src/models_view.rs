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
    /// `true` iff this model is the PRIMARY/default serve route.
    pub serving: bool,
    /// The served context window in tokens.
    pub context_len: u32,
    /// POC-3: `true` iff the model is RESIDENT in RAM right now (live LRU
    /// residency). The host recomputes this per `list()` call from the backend's
    /// `resident()` snapshot; an FFI-free / model-less serve reports `false`.
    pub loaded: bool,
    /// POC-3: the recipe handle a client invokes to chat with THIS model (the
    /// binder-free routing key — primary = `kx/recipes/chat`, secondary =
    /// `kx/recipes/m-<id>`). Empty when no model is served.
    pub chat_handle: String,
    /// The serving engine that backs this model — `"kx-llamacpp"` (in-process
    /// llama.cpp) or `"kx-ollama"` (a local Ollama daemon). A display/audit field
    /// (never identity); empty on an old host. Additive (proto tag 8).
    pub engine: String,
    /// PR-B: `true` iff this model is the server's CONFIGURED dataset embedder (the
    /// `KX_SERVE_EMBED_MODEL` model, else the primary). NOT a per-model embedding-
    /// capability claim (the engines expose none) — it marks WHICH model the
    /// server-embed path uses. Display/audit only (never identity); `false` on an old
    /// host or a model-less serve. Additive (proto tag 9).
    pub can_embed: bool,
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
