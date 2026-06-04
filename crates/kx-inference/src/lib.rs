//! kortecx inference dispatcher (P1.8, D35).
//!
//! Per D35, the router is a DISPATCHER WITH CAPABILITY ENFORCEMENT — NOT a
//! model selector. The workflow author (or SDK) NAMES the model in
//! `warrant.model_route.model_id`; the dispatcher obeys, checks scope, and
//! times out per `warrant.resource_ceiling.wall_clock_ms`. The model id
//! participates in the idempotency key (kx-mote → `MoteDef.model_id`) so
//! model changes correctly bust the cache.
//!
//! PR 8 ships two forward-compat trait-shape hooks to prevent breaking
//! trait changes when multimodal + constrained-generation features land:
//! `InferenceInput::Multimodal` (reserved variant) and
//! `InferenceParams.grammar` (reserved Option field). The OSS v0.1
//! `LlamaInferenceBackend` returns `Err(InferenceError::Unsupported)` on
//! either path — see `roadmap-multimodal-synthesis-post-pr9` for the
//! future-PR sequencing commitment.
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
mod dispatcher;
// The llama.cpp-backed `LlamaInferenceBackend` + its loaded-model cache live
// behind the `llamacpp` feature (default-on). Gating the modules — not just the
// dep — is what lets the crate compile with `--no-default-features` (no native
// FFI). See Cargo.toml.
#[cfg(feature = "llamacpp")]
mod cache;
#[cfg(feature = "llamacpp")]
mod llama;
mod types;

pub use backend::{BatchItem, InferenceBackend};
pub use dispatcher::{DispatchOutcome, Dispatcher, DispatcherConfig};
#[cfg(feature = "llamacpp")]
pub use llama::{ContentFetcher, LlamaInferenceBackend};
pub use types::{
    inference_params_from_mote, Grammar, InferenceError, InferenceInput, InferenceOutput,
    InferenceParams, MEDIA_MARKER,
};
