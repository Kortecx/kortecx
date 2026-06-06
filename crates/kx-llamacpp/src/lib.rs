#![warn(missing_docs)]
// SN-4 v2: tighten the lint surface. `pedantic` catches subtle issues
// (needless clones, manual let-else, by-value when &-ref would do).
// The allowed lints below are noise for an FFI-heavy crate where C/Rust
// integer width juggling and panic-doc bloat would be a treadmill.
#![warn(clippy::pedantic)]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_lossless,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::doc_markdown,
    clippy::elidable_lifetime_names,
    clippy::items_after_statements,
    clippy::pub_underscore_fields,
    clippy::needless_pass_by_value,
    clippy::range_plus_one,
    clippy::return_self_not_must_use
)]
// TODO(workspace.lints cleanup — undocumented_unsafe_blocks): the H-3 FFI
// hardening sweep (PR 4.7) added module-level safety documentation +
// alloc/free pairing tables. A follow-up audit will add per-block
// `// SAFETY:` comments to each individual `unsafe { ... }` block (~80
// sites in this crate), at which point this allow can be removed. Until
// then, the per-block SAFETY documentation is in the crate-level docs +
// the lifetime parameter chain enforces the load-bearing invariants at
// compile time. Allowed at the crate level so the workspace policy
// doesn't block kx-llamacpp.
#![allow(clippy::undocumented_unsafe_blocks)]
// Same TODO: kx-llamacpp's FFI wrapper uses `.expect()` on construction-
// time pointer NonNull checks where the FFI contract guarantees non-null
// in the success path. Documented inline per call site. A follow-up
// migrates these to typed errors / Option chains. The unconditional
// `#![allow]` covers both production and test code; no `cfg_attr(test)`
// needed (the cleanup PR will swap this for `cfg_attr(test, ...)` once
// production sites migrate).
#![allow(clippy::unwrap_used, clippy::expect_used)]

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
//!
//! ## Safety invariants (the FFI audit trail)
//!
//! Every `unsafe` block in this crate upholds one or more of the invariants
//! below. These are the load-bearing safety facts a reviewer should verify
//! when auditing the crate or advancing the `llama.cpp` submodule pin.
//!
//! 1. **Ownership chain enforced by lifetimes.** The type system rejects
//!    using a handle after its parent is dropped:
//!    - `Sampler<'b>` borrows from `LlamaBackend<'b>`
//!    - `Model<'b>` borrows from `LlamaBackend<'b>`
//!    - `Vocab<'m, 'b>` borrows from `Model<'b>` (with `'b: 'm`)
//!    - `Context<'m, 'b: 'm>` borrows from `Model<'b>` and is bounded by
//!      `'b: 'm` so the backend outlives both model and context.
//!
//!    Use-after-free of a llama.cpp resource is therefore a **compile
//!    error**, not a runtime UAF. If a future submodule advance changes
//!    ownership semantics (a function that "borrows from" becoming "owns")
//!    these bounds must be re-audited.
//!
//! 2. **Send is implemented where the handle is exclusively owned; Sync is
//!    NOT implemented anywhere.** `Model`, `Context`, `Sampler`, and
//!    `Batch` impl `Send` (a handle can be moved across threads) but do
//!    NOT impl `Sync` (the underlying llama.cpp resource is not safe for
//!    concurrent calls from multiple threads — pinning this in the type
//!    system means the borrow checker rejects accidental sharing). If a
//!    future caller wants concurrent inference, the answer is a
//!    `Mutex<Context>` or a per-thread `Context`, never a `&Context` from
//!    two threads.
//!
//! 3. **Each FFI handle has exactly one matching `Drop` impl calling the
//!    correct `*_free` function.** The audit table:
//!
//!    | Type | FFI alloc | FFI free | Drop location |
//!    |---|---|---|---|
//!    | `LlamaBackend` | `llama_backend_init` | `llama_backend_free` | [`backend`] |
//!    | `Model` | `llama_model_load_from_file` | `llama_model_free` | [`model`] |
//!    | `Context` | `llama_init_from_model` | `llama_free` | [`context`] |
//!    | `Sampler` | `llama_sampler_chain_init` | `llama_sampler_free` | [`sampler`] |
//!    | `Batch` | `llama_batch_init` | `llama_batch_free` | [`batch`] |
//!    | `Mtmd` | `mtmd_init_from_file` | `mtmd_free` | [`mtmd`] |
//!    | `Bitmap` | `mtmd_helper_bitmap_init_from_buf` | `mtmd_bitmap_free` | [`mtmd`] |
//!    | `InputChunks` | `mtmd_input_chunks_init` | `mtmd_input_chunks_free` | [`mtmd`] |
//!
//!    `Drop` runs exactly once. Each `*_free` is called with a `ptr` that
//!    is non-null (enforced by `NonNull` storage) and that the FFI assigned
//!    at construction time.
//!
//! 4. **Pointers from llama.cpp are wrapped in `NonNull<T>` immediately and
//!    checked at construction.** Construction returns a `Result` whose
//!    `Err` variant carries the load-failure context; the `Ok` variant
//!    holds a `NonNull`. No `*mut T` flows further than the construction
//!    site.
//!
//! 5. **One sanctioned self-reference: [`ModelWithProjector`].** A cached
//!    multi-modal projector must coexist with the [`Model`] it borrows in a
//!    single owned value (so a model + its projector can be cached together).
//!    `ModelWithProjector` is the *only* place that holds such a
//!    self-referential borrow. Its soundness rests on two facts the compiler
//!    cannot check — the model is `Box`-pinned (stable address) and
//!    field-declaration order guarantees the projector is dropped before the
//!    model — both documented at its definition and guarded by the
//!    `struct_field_drop_order_is_declaration_order` test. No new FFI handle is
//!    allocated (it composes `Model` + `Mtmd`, each of which still frees itself
//!    exactly once per invariant 3).
//!
//! ## ABI pin
//!
//! The `llama.cpp` source is a **pinned git submodule** at a specific
//! commit. The pin SHA + upgrade procedure are documented in
//! [`kx-llamacpp-sys/PIN.md`](../kx-llamacpp-sys/PIN.md). The build script
//! emits a `cargo:warning=` line containing the current pin SHA on every
//! build so drift is detectable from CI logs and developer terminals.
//! Advancing the pin without running the documented audit is **not safe**
//! — llama.cpp's FFI surface is unstable; an unaudited bump may change
//! function signatures, struct layouts, or pointer-ownership semantics in
//! ways that compile but corrupt memory under specific call patterns.

pub mod backend;
pub mod batch;
pub mod chat;
pub mod context;
pub mod error;
pub mod generator;
pub mod model;
pub mod mtmd;
pub mod sampler;
pub mod vocab;

// Env-driven inference tuning (CPU + Apple-Metal). Crate-private: read inside
// `ModelParams::new` / `ContextParams::new` so the frozen `kx-inference` call
// sites pick up GPU offload / flash-attn / KV-quant / threads transparently.
mod env;

pub use backend::LlamaBackend;
pub use batch::Batch;
pub use chat::ChatMessage;
pub use context::{Context, ContextParams, FlashAttn, KvCacheType, PerfData, PoolingType};
pub use error::LlamaError;
pub use generator::Generator;
pub use model::{Model, ModelParams};
pub use mtmd::{Bitmap, InputChunks, ModelWithProjector, Mtmd};
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
        batch.add(Token(101), 0, &[0], true);
        batch.add(Token(202), 1, &[0], false);
        assert_eq!(batch.n_tokens(), 2);
        batch.clear();
        assert_eq!(batch.n_tokens(), 0);
    }

    #[test]
    #[should_panic(expected = "Batch is full")]
    fn batch_overflow_panics() {
        let mut batch = Batch::with_capacity(2, 1);
        batch.add(Token(1), 0, &[0], false);
        batch.add(Token(2), 1, &[0], false);
        // This third add must panic.
        batch.add(Token(3), 2, &[0], false);
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

    /// SN-4 reachability: `Sampler::accept` is a no-op for stateless samplers
    /// (greedy / dist) but must not crash. Wrapper-level proof that the FFI
    /// call links and is safe to invoke unconditionally.
    #[test]
    fn sampler_accept_does_not_crash() {
        let backend = LlamaBackend::new().unwrap();
        let mut sampler = Sampler::greedy(&backend).unwrap();
        sampler.accept(Token(42));
        sampler.accept(Token(100));
        sampler.accept(Token(0));
    }

    /// SN-4 reachability: `Sampler::reset` is meaningful for stateful samplers
    /// (penalties, mirostat) but valid for stateless chains too. Proves the
    /// FFI call links.
    #[test]
    fn sampler_reset_does_not_crash() {
        let backend = LlamaBackend::new().unwrap();
        let mut sampler = Sampler::typical(&backend, 0.7, 40, 0.95, 1234).unwrap();
        sampler.reset();
        sampler.accept(Token(5));
        sampler.reset();
    }
}
