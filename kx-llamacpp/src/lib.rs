#![warn(missing_docs)]

//! # kx-llamacpp — safe wrapper over llama.cpp's C API
//!
//! Per `03-ffi-and-inference.md` §1, **all unsafe is contained in this crate**
//! (and the [`kx_llamacpp_sys`] crate it sits atop). Public surface is
//! `unsafe`-free: callers see RAII Rust types, idiomatic `Result` returns, and
//! lifetimes that enforce llama.cpp's ownership rules.
//!
//! ## Layering
//!
//! ```text
//! caller (kx-inference, downstream)
//!    │   safe Rust API only
//!    ▼
//! kx-llamacpp (this crate)        ← safe public API; unsafe blocks contained
//!    │   `extern "C"` calls
//!    ▼
//! kx-llamacpp-sys (FFI)           ← bindgen-generated; no Rust above
//!    │   ABI boundary
//!    ▼
//! llama.cpp (C++; vendored submodule, pinned tag)
//! ```
//!
//! Nothing outside this crate imports `kx-llamacpp-sys` directly.
//!
//! ## API surface (P1.7-b)
//!
//! | Type | Purpose |
//! |---|---|
//! | [`LlamaBackend`] | RAII initialization of llama.cpp's global backend. |
//! | [`Model`] / [`ModelParams`] | Load a GGUF; query metadata (n_embd, n_layer, n_params, desc, size, …). |
//! | [`Vocab`] | Borrowed from [`Model`]: tokenize, detokenize, BOS/EOS/NL queries. |
//! | [`Batch`] | RAII over `llama_batch`: token + position + seq-id + logits-flag bundles. |
//! | [`Context`] / [`ContextParams`] | RAII inference context: decode, encode, logits/embedding readout, KV-cache management, perf counters. |
//! | [`Sampler`] / [`SamplerChainBuilder`] | Sampler chains: greedy, dist, temp, top_k, top_p, min_p, temp_ext. |
//! | [`PoolingType`] | Embedding pooling strategy. |
//! | [`PerfData`] | Snapshot of llama.cpp's internal performance counters. |
//! | [`LlamaError`] | Error type. |
//!
//! ## Lifetimes
//!
//! ```text
//! LlamaBackend  ──┐ borrowed by ──┐
//!                 │               ▼
//!                 │           Sampler<'b>
//!                 │
//!                 └─ borrowed by ─► Model<'b>
//!                                       │
//!                                       └─ borrowed by ─► Vocab<'m,'b>
//!                                                          Context<'m,'b>
//!                                                          │
//!                                                          └─ &mut by ─► decode(&Batch)
//! ```
//!
//! `Batch` and `LlamaBackend` have no inbound lifetime: they're free-standing.
//! Every other type encodes its dependency through a generic lifetime so the
//! borrow checker rejects use-after-free at compile time.

pub mod backend;
pub mod batch;
pub mod context;
pub mod error;
pub mod model;
pub mod sampler;
pub mod vocab;

pub use backend::LlamaBackend;
pub use batch::Batch;
pub use context::{Context, ContextParams, PerfData, PoolingType};
pub use error::LlamaError;
pub use model::{Model, ModelParams};
pub use sampler::{Sampler, SamplerChainBuilder};
pub use vocab::{Token, Vocab};

// ---------------------------------------------------------------------------
// Wrapper-invariant tests that don't need a GGUF model
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use kx_llamacpp_sys as sys;

    #[test]
    fn backend_init_and_drop() {
        let backend = LlamaBackend::new().expect("backend init must succeed");
        drop(backend);
    }

    #[test]
    fn two_backends_in_sequence_work() {
        let b1 = LlamaBackend::new().unwrap();
        drop(b1);
        let b2 = LlamaBackend::new().unwrap();
        drop(b2);
    }

    #[test]
    fn two_concurrent_backends_share_state() {
        let b1 = LlamaBackend::new().unwrap();
        let b2 = LlamaBackend::new().unwrap();
        drop(b1);
        drop(b2);
    }

    #[test]
    fn load_nonexistent_model_returns_load_failed() {
        let backend = LlamaBackend::new().unwrap();
        let result = Model::load(&backend, "/nonexistent/path/to.gguf");
        match result {
            Err(LlamaError::LoadFailed { path }) => {
                assert_eq!(path, std::path::PathBuf::from("/nonexistent/path/to.gguf"));
            }
            Err(other) => panic!("expected LoadFailed, got error: {other}"),
            Ok(_) => panic!("expected LoadFailed, got an unexpected Ok(Model)"),
        }
    }

    #[test]
    fn path_with_nul_byte_returns_path_invalid() {
        let backend = LlamaBackend::new().unwrap();
        let bad_path = std::ffi::OsString::from("bad\0path.gguf");
        let result = Model::load(&backend, std::path::PathBuf::from(bad_path));
        assert!(matches!(result, Err(LlamaError::PathInvalid(_))));
    }

    #[test]
    fn pinned_tag_is_b9000() {
        assert_eq!(sys::PINNED_LLAMACPP_TAG, "b9000");
    }

    #[test]
    fn sampler_chain_builder_constructs_without_model() {
        // Exercise the builder API surface against real FFI but without
        // sampling — only construction + Drop. This confirms the chain init
        // and child-sampler add paths link correctly.
        let backend = LlamaBackend::new().unwrap();
        let sampler = Sampler::chain(&backend)
            .add_top_k(40)
            .unwrap()
            .add_top_p(0.95, 1)
            .unwrap()
            .add_min_p(0.05, 1)
            .unwrap()
            .add_temp(0.8)
            .unwrap()
            .add_dist(42)
            .unwrap()
            .build()
            .unwrap();
        // Drop the chain (frees the chain + all stages).
        drop(sampler);
    }

    #[test]
    fn greedy_sampler_constructs() {
        let backend = LlamaBackend::new().unwrap();
        let _s = Sampler::greedy(&backend).unwrap();
    }

    #[test]
    fn typical_sampler_constructs() {
        let backend = LlamaBackend::new().unwrap();
        let _s = Sampler::typical(&backend, 0.7, 40, 0.9, 1234).unwrap();
    }

    #[test]
    fn batch_allocation_and_population() {
        // Allocate a token-mode batch, populate, clear, drop. No llama state
        // touched — pure batch buffer management.
        let mut batch = Batch::with_capacity(8, 1);
        assert_eq!(batch.n_tokens(), 0);
        assert_eq!(batch.capacity(), 8);
        batch.add(101, 0, &[0], true);
        batch.add(202, 1, &[0], false);
        assert_eq!(batch.n_tokens(), 2);
        batch.clear();
        assert_eq!(batch.n_tokens(), 0);
    }

    #[test]
    #[should_panic(expected = "Batch is full")]
    fn batch_overflow_panics() {
        let mut batch = Batch::with_capacity(2, 1);
        batch.add(1, 0, &[0], false);
        batch.add(2, 1, &[0], false);
        // This third add must panic.
        batch.add(3, 2, &[0], false);
    }

    #[test]
    fn model_params_builder_compiles() {
        // Exercise the model-params surface (no llama state needed).
        let _p = ModelParams::new()
            .with_n_gpu_layers(0)
            .with_vocab_only(false)
            .with_use_mmap(true)
            .with_use_mlock(false)
            .with_check_tensors(false);
    }
}
