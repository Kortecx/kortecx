//! `kx-ollama` — an out-of-process, **FFI-free** [`InferenceBackend`] over the
//! [Ollama](https://ollama.com) REST API.
//!
//! This crate is the "bring-your-own backend" path the [`InferenceBackend`] trait
//! was designed for (its doc-comment names Triton / vLLM / remote APIs): it rides
//! the SAME `kx-inference` seam as the in-process `LlamaInferenceBackend` but talks
//! to a running Ollama daemon over HTTP instead of linking llama.cpp. So a
//! toolchain-free install (no C++ build: no `CMake` / clang / vendored submodule)
//! can serve local models — Ollama ships a precompiled, GPU-accelerated runtime and
//! manages model
//! downloads; kortecx just dials it.
//!
//! ## What it is NOT
//! - It is **not** in the journal / digest path. Inference output is
//!   `ReadOnlyNondet` (commit-once; recovery reads the committed `result_ref`,
//!   never re-runs the model), and [`InferenceOutput::backend_name`] is an
//!   audit-only field, never journaled — so swapping this in for the llama backend
//!   leaves the canonical projection digest invariant by construction.
//! - It carries **no FFI**. The `tests/dep_wall.rs` pins the dependency tree
//!   FFI-free (no `kx-llamacpp`) and writer-free (no journal / gateway / frozen
//!   trio).
//!
//! ## Security (SN-8)
//! The Ollama base URL is an **operator-configured** value (the host reads it from
//! env and hands it to [`OllamaClient::new`]); it is NEVER model / client /
//! Mote-controlled — no warrant or bound `model` arg can redirect the engine. The
//! client defaults to **loopback only**: a non-loopback URL is refused unless the
//! operator explicitly opts in (`allow_remote`).
//!
//! [`InferenceBackend`]: kx_inference::InferenceBackend
//! [`InferenceOutput::backend_name`]: kx_inference::InferenceOutput

#![forbid(unsafe_code)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod backend;
mod client;
mod error;

pub use backend::{OllamaBackend, BACKEND_NAME};
pub use client::{GenOutcome, OllamaClient};
pub use error::OllamaError;
