//! kortecx inference dispatcher (P1.8, D35).
//!
//! Per D35, the router is a DISPATCHER WITH CAPABILITY ENFORCEMENT — NOT a
//! model selector. The workflow author (or SDK) NAMES the model in
//! `warrant.model_route.model_id`; the dispatcher obeys, checks scope, and
//! times out per `warrant.resource_ceiling.wall_clock_ms`. The model id
//! participates in the idempotency key (kx-mote → `MoteDef.model_id`) so
//! model changes correctly bust the cache.
//!
//! PR 8 shipped two forward-compat trait-shape hooks; both are now IMPLEMENTED.
//! `InferenceInput::Multimodal` serves image dispatch on a vision model, and
//! `InferenceParams.grammar` (RC2) constrains tool-call decoding: when set, the
//! `LlamaInferenceBackend` renders the carried `kx_grammar::ToolEnvelopeSpec` to
//! GBNF and prepends a lazy/triggered sampler stage (`build_sampler`) — never in
//! the frozen `dispatcher.rs`, and only on a real model load (a malformed grammar
//! carrier fails closed). A backend that cannot honor a variant still returns
//! `Err(InferenceError::Unsupported)`.
//!
//! # Quick example
//!
//! ```
//! use kx_inference::{Dispatcher, DispatcherConfig, LlamaInferenceBackend};
//! use kx_model_validator::InMemoryModelRegistry;
//! use std::sync::Arc;
//!
//! // Construct an empty dispatcher with an empty registry. Register
//! // backends with `register_backend` and `Arc<dyn ModelRegistry>`
//! // with a populated registry when wiring real inference.
//! let dispatcher = Dispatcher::new(DispatcherConfig {
//!     model_registry: Arc::new(InMemoryModelRegistry::new()),
//! });
//! assert_eq!(dispatcher.backend_count(), 0);
//! ```

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]
#![cfg_attr(
    test,
    allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)
)]

mod backend;
// The object-safe `ContentFetcher` content-fetch seam is UNGATED — both the
// llama.cpp backend (`llama.rs`) and the FFI-free Ollama backend (`kx-ollama`)
// fetch image bytes for a `Multimodal` dispatch through it, and the Ollama path
// must build under `--features serve-engine` WITHOUT `llamacpp`. Deps: only
// `kx_content` (FFI-free leaf).
mod content;
mod dispatcher;
// The llama.cpp-backed `LlamaInferenceBackend` + its loaded-model cache live
// behind the `llamacpp` feature (default-on). Gating the modules — not just the
// dep — is what lets the crate compile with `--no-default-features` (no native
// FFI). See Cargo.toml.
#[cfg(feature = "llamacpp")]
mod cache;
#[cfg(feature = "llamacpp")]
mod llama;
// Built-in chat-template fallbacks (Gemma / ChatML) for models whose embedded
// GGUF template llama.cpp's `minja` cannot render. Uses `kx_llamacpp` types, so
// it rides the same `llamacpp` gate as `cache`/`llama`.
#[cfg(feature = "llamacpp")]
mod templates;
mod types;

pub use backend::{BatchItem, EmbeddingBackend, InferenceBackend, TokenSink};
pub use content::ContentFetcher;
pub use dispatcher::{DispatchOutcome, Dispatcher, DispatcherConfig};
#[cfg(feature = "llamacpp")]
pub use llama::LlamaInferenceBackend;
pub use types::{
    inference_params_from_mote, EmbeddingOutput, EmbeddingPooling, Grammar, InferenceError,
    InferenceInput, InferenceOutput, InferenceParams, MEDIA_MARKER,
};
