//! `LlamaBackend` — RAII for llama.cpp's global init / free.
//!
//! Per llama.cpp's documented thread-safety: backend init/free is not thread-safe.
//! We serialize via an internal mutex + ref-count so [`LlamaBackend::new`] is safe
//! to call from any thread, and the backend is only freed when the last handle
//! is dropped.

use std::sync::Mutex;

use kx_llamacpp_sys as sys;

use crate::error::LlamaError;

/// RAII initialization of llama.cpp's global backend state.
///
/// Calls `llama_backend_init` on construction and `llama_backend_free` on the
/// final Drop. **Must outlive every [`crate::Model`], [`crate::Context`], and
/// [`crate::Sampler`]** — llama.cpp's backend state is shared global state
/// required by all subsequent calls.
pub struct LlamaBackend {
    _marker: std::marker::PhantomData<*const ()>,
}

// Backend is a global resource but the Rust type is a witness, not a holder.
// The backend itself is initialized once globally; multiple `LlamaBackend`
// instances share state via a process-global ref-count.
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
