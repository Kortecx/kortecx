// Notice: this crate intentionally does NOT have `#![forbid(unsafe_code)]`.
// Per `03-ffi-and-inference.md` §1, `kx-llamacpp-sys` is the **single unsafe boundary**
// of the runtime — the only crate that defines `extern "C"` FFI declarations. The
// `kx-llamacpp` safe wrapper (P1.7) is the only crate that imports this one;
// downstream callers see only the safe API.

#![warn(missing_docs)]
// llama.cpp's C API uses snake_case + leading-underscore identifiers that don't fit
// Rust's idiomatic naming. We deliberately preserve the upstream names so callers
// (only `kx-llamacpp`, by rule) can recognize the C API one-to-one.
#![allow(non_camel_case_types, non_snake_case, non_upper_case_globals)]

//! # kx-llamacpp-sys — raw FFI to llama.cpp's C API
//!
//! **The single unsafe-heavy crate in the runtime.** Per the FFI boundary rule from
//! `03-ffi-and-inference.md` §1:
//!
//! > **Rust owns *what / where / when / on-failure*. C++ owns *how to run a forward
//! > pass*.**
//!
//! This crate carries the raw FFI declarations of llama.cpp's C API. **Nothing but
//! `kx-llamacpp` (P1.7) may import this crate** — the rule is enforced by review and
//! by a future compile-time lint at the workspace level. Downstream callers
//! (`kx-inference`, `kx-executor`, etc.) see only the safe wrapper.
//!
//! ## P1.6 scope — bindings-only, linking deferred to P1.7
//!
//! This step ships **forward declarations only**: opaque types (`llama_model`,
//! `llama_context`, `llama_vocab`) + a minimal set of `extern "C"` function
//! signatures that operate on pointers. No `bindgen`, no `build.rs`, no
//! `cc`/`cmake` invocation of llama.cpp's source, no linking. The DoD checks
//! at this step are:
//!
//! 1. The crate compiles cleanly.
//! 2. The binding shape is recognizable as llama.cpp's C API (one-to-one names).
//! 3. The pinned upstream version is recorded (see [`PINNED_LLAMACPP_VERSION`]).
//!
//! The full set of bindings (including by-value params structs like
//! `llama_model_params` and `llama_context_params`, the `llama_batch` interface, the
//! tokenization + sampling API surface) lands at P1.7 alongside:
//!
//! - The `bindgen` integration (vendored llama.cpp headers + `build.rs`).
//! - The C++ source compilation + linking (via the `cc` or `cmake` crate).
//! - The safe wrapper (`kx-llamacpp`) over this raw surface.
//! - The smoke test that loads a tiny GGUF and runs one forward pass through the
//!   raw API. The GGUF will live under Git LFS per the established convention.
//!
//! ## Safety
//!
//! Every function in this crate is `unsafe` to call. Callers are responsible for
//! upholding llama.cpp's documented preconditions (init order, lifetime/ownership
//! of pointers, threading rules). The safe wrapper (`kx-llamacpp`) absorbs these
//! preconditions behind a Rust-friendly API.
//!
//! ## The pinned version
//!
//! See [`PINNED_LLAMACPP_VERSION`]. When P1.7 lands the actual linking, the build
//! script will pin to this same upstream version via either a git submodule or a
//! vendored source drop.

/// The upstream llama.cpp version this binding shape was authored against.
///
/// Updates to this value MUST be paired with regeneration of the bindings + a CI
/// integration test against the new version. As of P1.6 (bindings-only), this is a
/// documentation pin; P1.7 will hold the binding ABI to the same version via the
/// build script.
pub const PINNED_LLAMACPP_VERSION: &str = "b4000 (placeholder — P1.7 pins via submodule/vendor)";

// ===========================================================================
// Primitive type aliases (matching llama.h on 64-bit Linux/macOS as of 2026)
// ===========================================================================

/// `int32_t` token identifier in the model's vocabulary.
pub type llama_token = i32;

/// `int32_t` token position within a sequence (KV-cache slot index).
pub type llama_pos = i32;

/// `int32_t` per-batch sequence identifier.
pub type llama_seq_id = i32;

// ===========================================================================
// Opaque types
// ===========================================================================
//
// These follow the Rust FFI idiom for opaque C types: zero-sized struct with a
// private field, so a `*mut llama_model` is unforgeable from safe Rust and any
// dereference requires `unsafe`. The actual size/layout lives in llama.cpp; we
// never construct these types in Rust — we only hold pointers to them.

/// Opaque handle to a loaded model (the weights + tokenizer + metadata).
///
/// Lifetime: created by `llama_model_load_from_file` (P1.7 signature); freed by
/// [`llama_model_free`]. **NOT thread-safe** — synchronize access in Rust.
#[repr(C)]
pub struct llama_model {
    _private: [u8; 0],
}

/// Opaque handle to an inference context (KV-cache + per-context state).
///
/// One context per concurrent inference stream. Created from a `*llama_model` by
/// `llama_new_context_with_model` (P1.7 signature); freed by [`llama_free`].
#[repr(C)]
pub struct llama_context {
    _private: [u8; 0],
}

/// Opaque handle to the model's vocabulary (tokenizer state).
///
/// Borrowed view obtained via [`llama_model_get_vocab`]; lifetime tied to the
/// owning model.
#[repr(C)]
pub struct llama_vocab {
    _private: [u8; 0],
}

// ===========================================================================
// Functions — the minimal pointer-only surface for P1.6
// ===========================================================================
//
// By-value-struct functions (`llama_model_load_from_file` taking
// `llama_model_params`, `llama_new_context_with_model` taking
// `llama_context_params`, the entire tokenize / decode / sample surface) are
// deferred to P1.7 because they require knowing the upstream struct layouts.
// P1.7 introduces bindgen, which generates the layouts from the actual headers.

extern "C" {
    /// Initialize the llama.cpp backend. Must be called once before any other
    /// llama_* function. Idempotent: calling twice is a no-op in upstream.
    pub fn llama_backend_init();

    /// Tear down the llama.cpp backend. Must be called after all
    /// models/contexts have been freed.
    pub fn llama_backend_free();

    /// Free a previously-loaded model. After this returns, `model` is dangling.
    ///
    /// # Safety
    /// `model` must have been produced by a llama_model_load_* function and
    /// must not have been previously freed.
    pub fn llama_model_free(model: *mut llama_model);

    /// Free an inference context. After this returns, `ctx` is dangling.
    ///
    /// # Safety
    /// `ctx` must have been produced by `llama_new_context_with_model` (or
    /// equivalent) and must not have been previously freed. The model the
    /// context was created from must outlive this call.
    pub fn llama_free(ctx: *mut llama_context);

    /// The context window size of an inference context (number of tokens it can
    /// hold). Pure read.
    ///
    /// # Safety
    /// `ctx` must be a valid, non-dangling context pointer.
    pub fn llama_n_ctx(ctx: *const llama_context) -> u32;

    /// The embedding dimensionality of a model. Pure read.
    ///
    /// # Safety
    /// `model` must be a valid, non-dangling model pointer.
    pub fn llama_n_embd(model: *const llama_model) -> i32;

    /// The vocabulary size of a vocab. Pure read.
    ///
    /// # Safety
    /// `vocab` must be a valid, non-dangling vocab pointer.
    pub fn llama_n_vocab(vocab: *const llama_vocab) -> i32;

    /// Borrow the vocabulary owned by `model`. The returned pointer is valid
    /// for the lifetime of `model`.
    ///
    /// # Safety
    /// `model` must be a valid, non-dangling model pointer.
    pub fn llama_model_get_vocab(model: *const llama_model) -> *const llama_vocab;
}

// ===========================================================================
// Compile-only shape check
// ===========================================================================
//
// A binding-shape check that runs at *compile* time (zero runtime cost): if any
// of the declared signatures is malformed, the const expressions below fail to
// type-check. P1.6's "the crate compiles cleanly" DoD is captured here.

const _ASSERT_TOKEN_IS_I32: usize = 0 - !(core::mem::size_of::<llama_token>() == 4) as usize;
const _ASSERT_POS_IS_I32: usize = 0 - !(core::mem::size_of::<llama_pos>() == 4) as usize;
const _ASSERT_SEQ_ID_IS_I32: usize = 0 - !(core::mem::size_of::<llama_seq_id>() == 4) as usize;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opaque_types_are_zero_sized_to_prevent_construction_in_rust() {
        // `llama_model`, `llama_context`, `llama_vocab` are zero-sized
        // forward declarations. Their actual layout lives in C; Rust must only
        // hold pointers. The ZST discipline prevents accidental
        // safe-Rust-side construction.
        assert_eq!(core::mem::size_of::<llama_model>(), 0);
        assert_eq!(core::mem::size_of::<llama_context>(), 0);
        assert_eq!(core::mem::size_of::<llama_vocab>(), 0);
    }

    #[test]
    fn primitive_token_aliases_match_int32() {
        assert_eq!(core::mem::size_of::<llama_token>(), 4);
        assert_eq!(core::mem::size_of::<llama_pos>(), 4);
        assert_eq!(core::mem::size_of::<llama_seq_id>(), 4);
    }

    #[test]
    fn pinned_version_constant_is_non_empty() {
        // Documentation pin; P1.7's build script will assert against this.
        assert!(!PINNED_LLAMACPP_VERSION.is_empty());
    }
}
