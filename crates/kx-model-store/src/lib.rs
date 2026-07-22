// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! # kx-model-store — model lifecycle (M4, D108.2)
//!
//! The model store is the runtime's single source of truth for **which model
//! files exist, what modalities they serve, and where their projector lives** —
//! the vocabulary every multi-modal dispatch needs. Before this crate the OSS
//! the `InferenceBackend` registered models as a frozen
//! `HashMap<ModelId, PathBuf>` with no capability metadata; a caller could not
//! ask "does this model accept images?" and so could not reject a text-only
//! model handed an image, or a vision projector handed audio.
//!
//! ## What it owns
//!
//! - [`ModelDescriptor`] — a model's identity: its `ModelId`, GGUF path, optional
//!   multi-modal projector (`mmproj`) path, declared [`Modality`] set, default
//!   context window, and a stable [`identity_digest`](ModelDescriptor::identity_digest).
//! - [`ModelRegistry`] — a `BTreeMap`-backed set of descriptors (deterministic
//!   iteration) implementing [`ModelResolver`], the seam a backend resolves through.
//! - [`gguf`] — a **fail-closed** GGUF-header validator: the model file is *new
//!   untrusted input*, so the header is parsed with bounded reads and rejected on
//!   bad magic / unknown version / absurd counts ([`ModelStoreError::InvalidGguf`]).
//!
//! ## FFI-free by construction (the install-barrier guarantee, D127.3)
//!
//! This crate depends on **no** `kx-llamacpp` / C++ FFI — only `kx-mote` and
//! `kx-content`. It therefore stays in the dependency closure that builds with no
//! C++ toolchain, preserving `cargo install kx-runtime`. The **live** loaded-model
//! handle (the heavyweight `llama_model`) is cached in the backend
//! (`kx-inference`), keyed by [`ModelDescriptor::identity_digest`]; this crate
//! holds only paths + metadata.
//!
//! ## `identity_digest` is a CACHE identity, not a weight hash
//!
//! Hashing multi-GB weights on every registration would be ruinous, and the
//! backend's laziness (a model file need not exist until first dispatch) forbids
//! reading the file at registration. So [`identity_digest`](ModelDescriptor::identity_digest)
//! is a domain-tagged blake3 over the **path + declared modalities** — stable
//! (same file ⇒ same key ⇒ one cached load shared across `ModelId`s that point at
//! it) without touching the weights. It is the loaded-model cache key, never a
//! cryptographic commitment to the weights and never journaled.
//!
//! ## Off the trust path (SN-8)
//!
//! The store never gates selection, commitment, eviction, or the audit path, and
//! carries no floats. The guarantee-path crates (`kx-scheduler` / `kx-executor` /
//! `kx-projection` / `kx-journal`) do not depend on it; the one inbound edge is
//! `kx-inference → kx-model-store` (behind the `llamacpp` feature), a capability
//! dependency, not a trust dependency.

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::doc_markdown
)]
// Tests assert on known-good values where `unwrap`/`expect` document the
// invariant being checked; the workspace policy denies them in library code.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod descriptor;
pub mod errors;
pub mod gguf;
pub mod registry;

pub use descriptor::{Modality, ModelDescriptor};
pub use errors::ModelStoreError;
pub use gguf::{read_context_length, read_model_name, GgufHeader};
pub use registry::{ModelRegistry, ModelResolver, MutableRegistry};
