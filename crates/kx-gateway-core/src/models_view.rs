//! The model-discovery read seam behind `ListModels` (Batch A).
//!
//! Display/discovery ONLY: model *selection* stays a recipe ENUM free-param
//! validated server-side at binding — nothing returned here authorizes
//! anything (the toolscout advisory precedent, SN-8). Spoken in gateway-core's
//! own wire vocabulary so no `kx-model-store` type crosses the seam; the host
//! builds its catalog from the descriptors it actually provisioned.

use crate::error::GatewayError;

/// One discoverable model, as display fields (mirrors `proto::ModelSummary`).
///
/// The boolean fields (`serving`/`loaded`/`can_embed`/`active`) are independent display
/// flags mirroring the wire `ModelSummary` one-to-one, NOT a state machine — so the
/// `struct_excessive_bools` enum/state-machine refactor does not apply.
#[derive(Clone, Debug)]
#[allow(clippy::struct_excessive_bools)]
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
    /// Model Control v2: the model's PROVENANCE — `"local"` (a startup GGUF),
    /// `"ollama"` (a daemon-discovered tag), `"pulled-ollama"` / `"pulled-url"` (a
    /// runtime `kx models pull`). Display/audit ONLY (SN-8); empty on an old host.
    /// Additive (proto tag 10).
    pub source: String,
    /// Model Control v2: `true` iff this model is the server's ACTIVE default (the
    /// `SetActiveModel` choice — an off-journal advisory hint a client uses to pick
    /// the per-model chat handle; the server never silently re-routes `kx/recipes/chat`).
    /// Recomputed per `list()` from the live active-model selection. Additive (proto tag 11).
    pub active: bool,
    /// Model Control v2: the RAG-grounded chat recipe handle for THIS model (the
    /// per-model `kx/recipes/chat-rag` / `kx/recipes/chat-rag-m-<id>`), so New Chat can
    /// ground a switched model. Empty when no dataset/embedder is configured for it.
    /// Additive (proto tag 12).
    pub chat_rag_handle: String,
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
