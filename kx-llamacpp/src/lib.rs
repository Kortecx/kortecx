#![warn(missing_docs)]

//! # kx-llamacpp — safe wrapper over llama.cpp's C API
//!
//! Per `03-ffi-and-inference.md` §1, **all unsafe is contained in this crate** (and the
//! `-sys` crate it sits atop). Public surface is `unsafe`-free: callers see RAII Rust
//! types, idiomatic `Result` returns, and lifetimes that enforce llama.cpp's ownership
//! rules.
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
//! ## P1.7-a scope
//!
//! This step ships the **safe wrapper API shape** + the build infrastructure that
//! compiles + links llama.cpp. The smoke test that loads a real model is at P1.7-b
//! (with a tiny GGUF downloaded in CI via a build script + SHA-256 verification).
//!
//! At P1.7-a:
//! - [`LlamaBackend`] — RAII initialization of llama.cpp's backend.
//! - [`Model`] — RAII handle to a loaded model.
//! - [`Context`] — RAII handle to an inference context.
//! - [`LlamaError`] — error type.
//! - Tests that exercise the safe wrapper invariants WITHOUT loading a model
//!   (LlamaBackend init/Drop sequence; Model::load on a non-existent path
//!   returns the right error).

use std::ffi::CString;
use std::path::Path;
use std::ptr::NonNull;
use std::sync::Mutex;

use kx_llamacpp_sys as sys;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors raised by the safe wrapper.
#[derive(Debug, Error)]
pub enum LlamaError {
    /// The model file could not be loaded — bad path, malformed GGUF, or
    /// llama.cpp internally rejected the model.
    #[error("failed to load model from {path:?}")]
    LoadFailed {
        /// The path that was attempted.
        path: std::path::PathBuf,
    },

    /// Failed to create an inference context from a model.
    #[error("failed to create inference context")]
    ContextCreationFailed,

    /// The supplied path could not be converted to a C string (interior nul byte
    /// or non-UTF-8 on platforms where C strings must be UTF-8).
    #[error("path is not representable as a C string: {0:?}")]
    PathInvalid(std::path::PathBuf),

    /// An underlying I/O error while reading metadata before passing to llama.cpp.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

// ---------------------------------------------------------------------------
// LlamaBackend — RAII for llama.cpp's global init/free
// ---------------------------------------------------------------------------

/// RAII initialization of llama.cpp's global backend state.
///
/// Calls `llama_backend_init` on construction and `llama_backend_free` on Drop.
/// **Must outlive every [`Model`] and [`Context`]** — llama.cpp's backend state
/// is shared global state required by all subsequent calls.
///
/// Per llama.cpp's documented thread-safety: backend init/free is not thread-safe.
/// We serialize via an internal mutex so `LlamaBackend::new()` is safe to call
/// from any thread; concurrent calls block briefly.
pub struct LlamaBackend {
    // The mutex is here to record that we hold the global backend slot. The
    // unit field is sufficient — llama.cpp manages the actual state internally.
    _marker: std::marker::PhantomData<*const ()>,
}

// Backend is a global resource but the Rust type is a witness, not a holder.
// The backend itself is initialized once globally; multiple `LlamaBackend`
// instances are NOT supported (the backend is conceptually a singleton).
// We track init state via a process-global counter.

static BACKEND_INIT_LOCK: Mutex<usize> = Mutex::new(0);

impl LlamaBackend {
    /// Initialize llama.cpp's backend. Returns a RAII handle; the backend is
    /// freed when the last `LlamaBackend` is dropped.
    ///
    /// # Errors
    /// Currently infallible — llama.cpp's `llama_backend_init` does not return
    /// an error code in the version pinned at [`sys::PINNED_LLAMACPP_TAG`].
    pub fn new() -> Result<Self, LlamaError> {
        let mut guard = BACKEND_INIT_LOCK.lock().expect("poisoned backend lock");
        if *guard == 0 {
            // SAFETY: llama_backend_init is documented as safe to call once; the
            // mutex above serializes any racing init calls.
            unsafe { sys::llama_backend_init() };
        }
        *guard += 1;
        Ok(Self {
            _marker: std::marker::PhantomData,
        })
    }
}

impl Drop for LlamaBackend {
    fn drop(&mut self) {
        let mut guard = BACKEND_INIT_LOCK.lock().expect("poisoned backend lock");
        *guard = guard.saturating_sub(1);
        if *guard == 0 {
            // SAFETY: ref-counted; freed only when the last handle is dropped.
            unsafe { sys::llama_backend_free() };
        }
    }
}

// ---------------------------------------------------------------------------
// Model — RAII handle to a loaded model
// ---------------------------------------------------------------------------

/// RAII handle to a loaded llama.cpp model.
///
/// Construction performs the model load; Drop calls `llama_model_free`. The
/// `Model` borrows from the `LlamaBackend` (lifetime-tied) so the backend
/// outlives the model.
pub struct Model<'b> {
    // NonNull so the type is `!ZST` and impossible to construct without going
    // through `Model::load`.
    ptr: NonNull<sys::llama_model>,
    _backend: std::marker::PhantomData<&'b LlamaBackend>,
}

impl<'b> Model<'b> {
    /// Load a model from a GGUF file. Uses llama.cpp's default model parameters.
    ///
    /// # Errors
    /// - [`LlamaError::PathInvalid`] if `path` cannot be converted to a C string.
    /// - [`LlamaError::LoadFailed`] if llama.cpp returns a null pointer (file
    ///   missing, malformed GGUF, model rejected internally).
    pub fn load(_backend: &'b LlamaBackend, path: impl AsRef<Path>) -> Result<Self, LlamaError> {
        let path_ref = path.as_ref();
        let c_path = CString::new(path_ref.as_os_str().to_string_lossy().as_bytes())
            .map_err(|_| LlamaError::PathInvalid(path_ref.to_owned()))?;

        // SAFETY: llama_model_default_params is pure (returns a value struct);
        // llama_model_load_from_file is unsafe because it reads from disk and
        // returns a raw pointer (null on failure).
        let model_ptr = unsafe {
            let params = sys::llama_model_default_params();
            sys::llama_model_load_from_file(c_path.as_ptr(), params)
        };

        let ptr = NonNull::new(model_ptr).ok_or_else(|| LlamaError::LoadFailed {
            path: path_ref.to_owned(),
        })?;

        Ok(Self {
            ptr,
            _backend: std::marker::PhantomData,
        })
    }

    /// The model's embedding dimensionality.
    pub fn n_embd(&self) -> i32 {
        // SAFETY: ptr is non-null and points to a live model owned by this Model.
        unsafe { sys::llama_model_n_embd(self.ptr.as_ptr()) }
    }
}

impl<'b> Drop for Model<'b> {
    fn drop(&mut self) {
        // SAFETY: ptr is non-null and was produced by llama_model_load_from_file;
        // Drop runs exactly once.
        unsafe { sys::llama_model_free(self.ptr.as_ptr()) }
    }
}

// Model is Send if the underlying pointer is opaque + llama.cpp is documented as
// per-model-thread-safe-for-non-mutating-reads. We don't claim Sync; callers wrap
// in Mutex if cross-thread mutation is needed.
unsafe impl<'b> Send for Model<'b> {}

// ---------------------------------------------------------------------------
// Context — RAII handle to an inference context
// ---------------------------------------------------------------------------

/// RAII handle to an inference context for a model.
///
/// Construction creates a fresh KV-cache context tied to the model; Drop calls
/// `llama_free` to release the context. The `Context` borrows from the `Model`
/// so the model outlives the context.
pub struct Context<'m, 'b: 'm> {
    ptr: NonNull<sys::llama_context>,
    _model: std::marker::PhantomData<&'m Model<'b>>,
}

impl<'m, 'b: 'm> Context<'m, 'b> {
    /// Create a new inference context for `model`. Uses llama.cpp's default
    /// context parameters.
    ///
    /// # Errors
    /// [`LlamaError::ContextCreationFailed`] if llama.cpp returns null.
    pub fn new(model: &'m Model<'b>) -> Result<Self, LlamaError> {
        // SAFETY: llama_context_default_params is pure; llama_new_context_with_model
        // returns a raw pointer (null on failure).
        let ctx_ptr = unsafe {
            let params = sys::llama_context_default_params();
            sys::llama_init_from_model(model.ptr.as_ptr(), params)
        };

        let ptr = NonNull::new(ctx_ptr).ok_or(LlamaError::ContextCreationFailed)?;

        Ok(Self {
            ptr,
            _model: std::marker::PhantomData,
        })
    }

    /// The context window size (number of tokens this context can hold).
    pub fn n_ctx(&self) -> u32 {
        // SAFETY: ptr is non-null and points to a live context owned by this Context.
        unsafe { sys::llama_n_ctx(self.ptr.as_ptr()) }
    }
}

impl<'m, 'b: 'm> Drop for Context<'m, 'b> {
    fn drop(&mut self) {
        // SAFETY: ptr is non-null and was produced by llama_init_from_model;
        // Drop runs exactly once.
        unsafe { sys::llama_free(self.ptr.as_ptr()) }
    }
}

unsafe impl<'m, 'b: 'm> Send for Context<'m, 'b> {}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Exercises the backend init/free cycle through real FFI. Verifies that the
    /// link chain works end-to-end (kx-llamacpp → -sys → llama.cpp library).
    #[test]
    fn backend_init_and_drop() {
        let backend = LlamaBackend::new().expect("backend init must succeed");
        // Drop happens at end of scope. No model, no context — just init/free.
        drop(backend);
    }

    /// Two backends in sequence: the ref-counted init mechanism handles repeated
    /// construction safely.
    #[test]
    fn two_backends_in_sequence_work() {
        let b1 = LlamaBackend::new().unwrap();
        drop(b1);
        let b2 = LlamaBackend::new().unwrap();
        drop(b2);
    }

    /// Two concurrent backends share the same global state via ref-counting. The
    /// backend is only freed when both are dropped.
    #[test]
    fn two_concurrent_backends_share_state() {
        let b1 = LlamaBackend::new().unwrap();
        let b2 = LlamaBackend::new().unwrap();
        drop(b1);
        // Backend should still be alive (b2 holds it).
        drop(b2);
        // Now backend is freed.
    }

    /// Loading a non-existent file produces `LlamaError::LoadFailed`.
    #[test]
    fn load_nonexistent_model_returns_load_failed() {
        let backend = LlamaBackend::new().unwrap();
        let result = Model::load(&backend, "/nonexistent/path/to.gguf");
        // `Model` wraps a raw pointer and does not derive `Debug`; match the Err
        // variant directly rather than relying on Debug formatting.
        match result {
            Err(LlamaError::LoadFailed { path }) => {
                assert_eq!(path, std::path::PathBuf::from("/nonexistent/path/to.gguf"));
            }
            Err(other) => panic!("expected LoadFailed, got error: {other}"),
            Ok(_) => panic!("expected LoadFailed, got an unexpected Ok(Model)"),
        }
    }

    /// A path with a NUL byte fails with `PathInvalid`.
    #[test]
    fn path_with_nul_byte_returns_path_invalid() {
        let backend = LlamaBackend::new().unwrap();
        let bad_path = std::ffi::OsString::from("bad\0path.gguf");
        let result = Model::load(&backend, std::path::PathBuf::from(bad_path));
        assert!(matches!(result, Err(LlamaError::PathInvalid(_))));
    }

    /// The pinned tag constant is non-empty and matches what we expect to be
    /// using. Forces a tracker update when the submodule version moves.
    #[test]
    fn pinned_tag_is_b9000() {
        assert_eq!(sys::PINNED_LLAMACPP_TAG, "b9000");
    }
}
