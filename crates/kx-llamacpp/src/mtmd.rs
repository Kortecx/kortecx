//! `Mtmd` — safe RAII wrapper over llama.cpp's `mtmd` (multi-modal) C API.
//!
//! `mtmd` is the projector layer that turns media (images in PR-2; audio in
//! PR-3) into embeddings the text model decodes alongside its tokens. This
//! module exposes the minimum surface the in-process backend needs for the
//! IMAGE path:
//!
//! 1. [`Mtmd::from_file`] — load the projector (`mmproj`) and bind it to an
//!    already-loaded text [`Model`].
//! 2. [`Bitmap::from_image_buf`] — decode encoded image bytes (jpg/png/bmp/gif,
//!    via the vendored `stb_image` compiled into `libmtmd`) into a bitmap.
//!    **Fail-closed**: returns `Err` (never panics / UB) on undecodable bytes.
//! 3. [`Mtmd::tokenize`] — interleave the instruction text (with one media
//!    marker per image) and the bitmaps into a chunk list.
//! 4. [`Mtmd::eval_chunks`] — the multi-modal **prefill**: it runs
//!    `llama_decode` on text chunks and `mtmd_encode` + embedding-mode
//!    `llama_decode` on image chunks (handling non-causal attention / M-RoPE
//!    internally), advancing `n_past`. After it returns, generation continues
//!    with the ordinary [`crate::Generator`] / [`crate::Sampler`] loop via
//!    [`crate::Generator::from_prefilled`].
//!
//! ## Threading
//!
//! Like [`crate::Model`] / [`crate::Context`], these handles are `!Send` +
//! `!Sync` (they hold raw `mtmd_*` pointers and borrow the `!Sync` model).
//! `mtmd_helper_eval_chunks` is explicitly NOT thread-safe upstream — the
//! in-process backend uses these types exclusively on its single model-cache
//! owner thread, so no synchronization is needed and **no `unsafe impl Send`
//! is added** (the borrow checker keeps them pinned to one thread).
//!
//! ## FFI ownership (extends the audit table in [`crate`])
//!
//! | Type | FFI alloc | FFI free | Drop location |
//! |---|---|---|---|
//! | `Mtmd` | `mtmd_init_from_file` | `mtmd_free` | here |
//! | `Bitmap` | `mtmd_helper_bitmap_init_from_buf` | `mtmd_bitmap_free` | here |
//! | `InputChunks` | `mtmd_input_chunks_init` | `mtmd_input_chunks_free` | here |
//!
//! Each handle is wrapped in `NonNull` at construction and freed exactly once.

use std::ffi::{CStr, CString};
use std::path::{Path, PathBuf};
use std::ptr::NonNull;

use kx_llamacpp_sys as sys;

use crate::context::Context;
use crate::error::LlamaError;
use crate::model::Model;

/// RAII handle to a loaded multi-modal projector bound to a text [`Model`].
///
/// Borrows the model (`'m`) so the projector can never outlive the weights it
/// references; the backend (`'b`) outlives both.
pub struct Mtmd<'m, 'b: 'm> {
    ptr: NonNull<sys::mtmd_context>,
    _model: std::marker::PhantomData<&'m Model<'b>>,
}

impl<'m, 'b: 'm> Mtmd<'m, 'b> {
    /// Load the projector at `mmproj_path` and bind it to `model`.
    ///
    /// `n_threads <= 0` keeps llama.cpp's default thread count; `use_gpu`
    /// offloads the projector to the platform GPU backend when available
    /// (Metal on Apple Silicon; no effect on the CPU-only OSS Linux build).
    ///
    /// # Errors
    /// - [`LlamaError::PathInvalid`] if `mmproj_path` is not a valid C string.
    /// - [`LlamaError::MtmdInitFailed`] if llama.cpp returns a null context
    ///   (bad path, malformed projector, or projector/model mismatch).
    #[tracing::instrument(level = "info", skip(model, mmproj_path), fields(mmproj = %mmproj_path.as_ref().display()))]
    pub fn from_file(
        model: &'m Model<'b>,
        mmproj_path: impl AsRef<Path>,
        n_threads: i32,
        use_gpu: bool,
    ) -> Result<Self, LlamaError> {
        let path_ref = mmproj_path.as_ref();
        let c_path = CString::new(path_ref.as_os_str().to_string_lossy().as_bytes())
            .map_err(|_| LlamaError::PathInvalid(path_ref.to_owned()))?;

        // SAFETY: pure C function returning a value struct of safe defaults
        // (incl. the default media marker pointer, which we leave intact).
        let mut params = unsafe { sys::mtmd_context_params_default() };
        params.use_gpu = use_gpu;
        if n_threads > 0 {
            params.n_threads = n_threads;
        }

        // SAFETY: `c_path` is NUL-terminated and lives across the call;
        // `model.ptr` is a valid, non-null, still-borrowed model pointer
        // (the `&'m Model` borrow guarantees it outlives the returned `Mtmd`);
        // `params` is passed by value (a copy).
        let raw = unsafe { sys::mtmd_init_from_file(c_path.as_ptr(), model.ptr.as_ptr(), params) };

        let ptr = NonNull::new(raw).ok_or_else(|| LlamaError::MtmdInitFailed {
            path: path_ref.to_owned(),
        })?;

        Ok(Self {
            ptr,
            _model: std::marker::PhantomData,
        })
    }

    /// Whether the loaded projector accepts image input.
    #[must_use]
    pub fn supports_vision(&self) -> bool {
        // SAFETY: `self.ptr` is a valid, non-null mtmd context for `self`'s life.
        unsafe { sys::mtmd_support_vision(self.ptr.as_ptr()) }
    }

    /// Whether the loaded projector accepts audio input (PR-3 gate; an image
    /// projector returns false, so handing it audio fails closed).
    #[must_use]
    pub fn supports_audio(&self) -> bool {
        // SAFETY: as `supports_vision`.
        unsafe { sys::mtmd_support_audio(self.ptr.as_ptr()) }
    }

    /// The media marker string the model expects in the text where an image
    /// goes (e.g. `"<__media__>"`). One marker must appear per bitmap. Falls
    /// back to the canonical literal in the (never-observed) event the upstream
    /// marker is not valid UTF-8, so this is infallible.
    #[must_use]
    pub fn default_marker() -> &'static str {
        // SAFETY: `mtmd_default_marker` returns a pointer to a static,
        // NUL-terminated C string owned by libmtmd; valid for the program's
        // lifetime.
        let raw = unsafe { sys::mtmd_default_marker() };
        // The upstream marker is ASCII; fall back to the canonical literal if a
        // future pin ever returns non-UTF-8 (keeps this infallible).
        unsafe { CStr::from_ptr(raw) }
            .to_str()
            .unwrap_or("<__media__>")
    }

    /// Tokenize `text` (which must contain exactly `bitmaps.len()` media
    /// markers — see [`Self::default_marker`]) interleaved with `bitmaps` into
    /// a chunk list ready for [`Self::eval_chunks`].
    ///
    /// `bitmaps` must stay alive through the subsequent `eval_chunks` call.
    ///
    /// # Errors
    /// [`LlamaError::TokenizeChunksFailed`] if `mtmd_tokenize` returns non-zero
    /// (1 = marker/bitmap count mismatch; 2 = media preprocessing failure).
    pub fn tokenize(&self, text: &str, bitmaps: &[&Bitmap]) -> Result<InputChunks, LlamaError> {
        let c_text =
            CString::new(text.as_bytes()).map_err(|_| LlamaError::TokenizeChunksFailed(-1))?;

        // SAFETY: allocates an empty chunk container; non-null on success.
        let output_raw = unsafe { sys::mtmd_input_chunks_init() };
        let output = NonNull::new(output_raw)
            // OOM only; map to the tokenize-failure surface rather than panic.
            .ok_or(LlamaError::TokenizeChunksFailed(-2))?;
        // Own it immediately so any early return frees it via Drop.
        let chunks = InputChunks { ptr: output };

        let input_text = sys::mtmd_input_text {
            text: c_text.as_ptr(),
            add_special: true,
            parse_special: true,
        };

        // `mtmd_tokenize` takes `*mut *const mtmd_bitmap`; build a contiguous
        // array of bitmap pointers (read-only to mtmd, hence `*const` elements).
        let mut bmp_ptrs: Vec<*const sys::mtmd_bitmap> = bitmaps
            .iter()
            .map(|b| b.ptr.as_ptr().cast_const())
            .collect();

        // SAFETY: `self.ptr` valid; `chunks.ptr` valid + owned by `chunks`;
        // `&input_text` outlives the call and `c_text` backs its `text` field;
        // `bmp_ptrs` is a valid array of `bitmaps.len()` valid bitmap pointers
        // that outlive the call.
        let rc = unsafe {
            sys::mtmd_tokenize(
                self.ptr.as_ptr(),
                chunks.ptr.as_ptr(),
                std::ptr::addr_of!(input_text),
                bmp_ptrs.as_mut_ptr(),
                bmp_ptrs.len(),
            )
        };
        // Keep the C strings / bitmap-pointer array alive until after the call.
        drop(c_text);
        drop(bmp_ptrs);

        if rc != 0 {
            return Err(LlamaError::TokenizeChunksFailed(rc));
        }
        Ok(chunks)
    }

    /// Run the multi-modal **prefill** over `chunks` into `ctx`, starting at
    /// `n_past` in sequence `seq_id`, using batch size `n_batch`. With
    /// `logits_last == true` the final position's logits are computed so the
    /// first generation step can sample immediately.
    ///
    /// Returns the new `n_past` (the position the generator continues from).
    ///
    /// # Errors
    /// [`LlamaError::EvalChunksFailed`] if any chunk fails to encode/decode.
    pub fn eval_chunks(
        &self,
        ctx: &mut Context<'m, 'b>,
        chunks: &InputChunks,
        n_past: i32,
        seq_id: i32,
        n_batch: i32,
        logits_last: bool,
    ) -> Result<i32, LlamaError> {
        let mut new_n_past: sys::llama_pos = 0;
        // SAFETY: all pointers are valid + non-null for the call; `ctx.raw_mut`
        // hands the live llama_context the helper drives (it mutates the KV
        // cache, matching the `&mut` borrow). Single-threaded by construction
        // (the helper is not thread-safe; callers run it on the owner thread).
        let rc = unsafe {
            sys::mtmd_helper_eval_chunks(
                self.ptr.as_ptr(),
                ctx.raw_mut(),
                chunks.ptr.as_ptr(),
                n_past,
                seq_id,
                n_batch,
                logits_last,
                std::ptr::addr_of_mut!(new_n_past),
            )
        };
        if rc != 0 {
            return Err(LlamaError::EvalChunksFailed(rc));
        }
        Ok(new_n_past)
    }
}

impl Drop for Mtmd<'_, '_> {
    fn drop(&mut self) {
        // SAFETY: `ptr` came from `mtmd_init_from_file`; freed exactly once.
        unsafe { sys::mtmd_free(self.ptr.as_ptr()) }
    }
}

/// A loaded text [`Model`] bundled with a lazily-loaded, **cached** multi-modal
/// projector ([`Mtmd`]).
///
/// ## Why this type exists (PR-2.5)
///
/// [`Mtmd`] borrows the [`Model`] it is initialized from (`Mtmd<'m, 'b>` holds
/// `PhantomData<&'m Model<'b>>`), so a `Model` and an `Mtmd` derived from it
/// cannot live in the same ordinary struct in safe Rust — that is a
/// self-referential borrow. The in-process backend needs exactly that: keep one
/// model resident AND keep its projector resident across dispatches, instead of
/// re-running [`Mtmd::from_file`] (which re-uploads the projector to the GPU —
/// seconds of work) on **every** multi-modal call. This type resolves the
/// self-reference soundly and **contains the single `unsafe`** it requires, per
/// the crate invariant that all `unsafe` lives in `kx-llamacpp`.
///
/// ## Why it is sound (the two load-bearing facts)
///
/// 1. **Stable address.** The model is heap-pinned in a [`Box`], so its address
///    never moves when the bundle itself is moved (e.g. within a cache `Vec`
///    that reorders entries or reallocates).
/// 2. **Drop order.** Rust drops struct fields in *declaration order*, so the
///    `projector` field (declared first ⇒ `mtmd_free` first) is always dropped
///    strictly before the `model` field (`llama_model_free`). The projector
///    therefore never outlives the model despite the lifetime-erased self-borrow.
///    The `struct_field_drop_order_is_declaration_order` test guards the
///    declaration-order ⇒ drop-order mechanism this relies on.
///
/// The projector is stored at `Mtmd<'b, 'b>` (the model-borrow lifetime widened
/// to the backend lifetime `'b`). Because [`Mtmd`] is **covariant** in its
/// model-borrow lifetime, [`Self::projector`] can still be used wherever a
/// shorter-lived `Mtmd<'_, 'b>` is expected — e.g. alongside a per-dispatch
/// [`Context`] in [`Mtmd::eval_chunks`].
pub struct ModelWithProjector<'b> {
    // FIELD ORDER IS LOAD-BEARING — DO NOT REORDER. Rust drops struct fields in
    // declaration order; `projector` borrows `model`, so it MUST be declared
    // (and therefore dropped) before `model`.
    projector: Option<Mtmd<'b, 'b>>,
    /// The projector path currently cached in `projector` (if any). A request
    /// for a *different* path triggers a reload (defensive — a model identity
    /// normally maps to exactly one projector).
    mmproj_path: Option<PathBuf>,
    model: Box<Model<'b>>,
}

impl<'b> ModelWithProjector<'b> {
    /// Wrap a loaded model. No projector is loaded until [`Self::ensure_projector`].
    #[must_use]
    pub fn new(model: Model<'b>) -> Self {
        Self {
            projector: None,
            mmproj_path: None,
            model: Box::new(model),
        }
    }

    /// The wrapped text model.
    #[must_use]
    pub fn model(&self) -> &Model<'b> {
        &self.model
    }

    /// The cached projector, if one has been loaded via [`Self::ensure_projector`].
    #[must_use]
    pub fn projector(&self) -> Option<&Mtmd<'b, 'b>> {
        self.projector.as_ref()
    }

    /// Ensure a projector for `mmproj_path` is loaded and cached, loading it on a
    /// miss (or reloading if a *different* path is requested). Returns `true`
    /// iff a (re)load actually occurred — the caller uses this to count cold
    /// projector loads (the `mmproj_loads` metric): on a cache HIT it returns
    /// `false` and performs no FFI work.
    ///
    /// `n_threads` / `use_gpu` are forwarded to [`Mtmd::from_file`].
    ///
    /// # Errors
    /// [`LlamaError::MtmdInitFailed`] / [`LlamaError::PathInvalid`] propagated
    /// from [`Mtmd::from_file`].
    pub fn ensure_projector(
        &mut self,
        mmproj_path: &Path,
        n_threads: i32,
        use_gpu: bool,
    ) -> Result<bool, LlamaError> {
        if self.projector.is_some() && self.mmproj_path.as_deref() == Some(mmproj_path) {
            return Ok(false); // cache HIT — the projector is already resident.
        }
        // Miss (or a different projector path): drop any existing projector
        // FIRST — its `mtmd_free` runs while `self.model` is still alive, the
        // correct drop order — then load the new one.
        self.projector = None;
        self.mmproj_path = None;

        // SAFETY: we hand `Mtmd::from_file` a `&'b Model<'b>` synthesized from
        // the heap-pinned model via a raw pointer. Widening the borrow to `'b`
        // is a lie the type system cannot verify; it is made sound by this
        // type's two invariants (see the struct docs): (1) the model is
        // `Box`-pinned, so its address is stable for as long as `self` lives;
        // (2) field-declaration order guarantees the projector — stored into
        // `self.projector` below — is dropped strictly before `self.model`. The
        // returned `Mtmd` retains only its own `mtmd_context` pointer plus a
        // zero-sized `PhantomData` of the borrow (no dangling reference is kept
        // at runtime) and never dereferences the synthesized reference after
        // construction; `mtmd_init_from_file` reads `model.ptr` only during this
        // call, while the model is plainly alive and not concurrently mutated.
        let model_ref: &'b Model<'b> = unsafe { &*std::ptr::addr_of!(*self.model) };
        let projector = Mtmd::from_file(model_ref, mmproj_path, n_threads, use_gpu)?;
        self.projector = Some(projector);
        self.mmproj_path = Some(mmproj_path.to_owned());
        Ok(true)
    }
}

/// RAII handle to a decoded media bitmap.
pub struct Bitmap {
    ptr: NonNull<sys::mtmd_bitmap>,
}

impl Bitmap {
    /// Decode encoded image bytes (jpg/png/bmp/gif) into a bitmap via the
    /// projector's preprocessor (vendored `stb_image`).
    ///
    /// **Image-only (PR-2):** if the bytes decode as audio (auto-detected by
    /// magic bytes), the bitmap is freed and [`LlamaError::AudioNotSupported`]
    /// is returned.
    ///
    /// # Errors
    /// - [`LlamaError::BitmapDecodeFailed`] on undecodable / unsupported bytes
    ///   (the fail-closed boundary for untrusted media).
    /// - [`LlamaError::AudioNotSupported`] if the bytes are audio.
    pub fn from_image_buf(mtmd: &Mtmd<'_, '_>, bytes: &[u8]) -> Result<Self, LlamaError> {
        // SAFETY: `mtmd.ptr` valid; `bytes` is a valid `len`-byte buffer that
        // outlives the (synchronous) call. Returns a wrapper whose `.bitmap` is
        // null on decode failure (the `placeholder=false` path eagerly decodes;
        // `.video_ctx` is only set for video input, which this image-only path
        // never requests, so it is null here and needs no free).
        let raw = unsafe {
            sys::mtmd_helper_bitmap_init_from_buf(
                mtmd.ptr.as_ptr(),
                bytes.as_ptr(),
                bytes.len(),
                false,
            )
            .bitmap
        };
        let ptr = NonNull::new(raw).ok_or(LlamaError::BitmapDecodeFailed {
            n_bytes: bytes.len(),
        })?;
        let bitmap = Self { ptr };

        // SAFETY: `ptr` is a valid bitmap just constructed.
        if unsafe { sys::mtmd_bitmap_is_audio(bitmap.ptr.as_ptr()) } {
            // `bitmap` drops here, freeing the audio bitmap.
            return Err(LlamaError::AudioNotSupported);
        }
        Ok(bitmap)
    }
}

impl Drop for Bitmap {
    fn drop(&mut self) {
        // SAFETY: `ptr` came from `mtmd_helper_bitmap_init_from_buf`; freed once.
        unsafe { sys::mtmd_bitmap_free(self.ptr.as_ptr()) }
    }
}

/// RAII handle to a tokenized chunk list (text + image chunks) produced by
/// [`Mtmd::tokenize`] and consumed by [`Mtmd::eval_chunks`].
pub struct InputChunks {
    ptr: NonNull<sys::mtmd_input_chunks>,
}

impl InputChunks {
    /// Number of chunks (text + image) in the list.
    #[must_use]
    pub fn len(&self) -> usize {
        // SAFETY: `ptr` is a valid chunk list for `self`'s life.
        unsafe { sys::mtmd_input_chunks_size(self.ptr.as_ptr()) }
    }

    /// Whether the chunk list is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Drop for InputChunks {
    fn drop(&mut self) {
        // SAFETY: `ptr` came from `mtmd_input_chunks_init`; freed exactly once.
        // Frees the contained chunks too (per the upstream contract).
        unsafe { sys::mtmd_input_chunks_free(self.ptr.as_ptr()) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // `mtmd_default_marker` returns a static string and needs no model / backend
    // init, so this runs in the default (`--workspace`, no-model) test set. The
    // model-bound paths (`from_file` / `from_image_buf` / `tokenize` /
    // `eval_chunks`) are exercised by the `model-smoke-test-multimodal` gate,
    // which loads a real VLM + projector.
    #[test]
    fn default_marker_is_non_empty_utf8() {
        let marker = Mtmd::default_marker();
        assert!(!marker.is_empty(), "media marker must be non-empty");
        // The upstream marker is `<__media__>`; assert the stable shape without
        // hard-coding (a pin bump could change it) — it must be a bracketed tag.
        assert!(
            marker.starts_with('<') && marker.ends_with('>'),
            "media marker should be a bracketed tag, got {marker:?}"
        );
    }

    /// The load-bearing invariant behind [`ModelWithProjector`]'s soundness:
    /// Rust drops struct fields in *declaration order*, so a `projector` field
    /// declared before a `model` field is dropped first (`mtmd_free` before
    /// `llama_model_free`). The real bundle holds FFI handles we cannot
    /// instrument from a unit test, so this locks the *mechanism* on a mirror
    /// struct with the SAME field order. If `ModelWithProjector`'s fields are
    /// ever reordered, this guard must be revisited together with that change.
    #[test]
    fn struct_field_drop_order_is_declaration_order() {
        use std::cell::RefCell;
        thread_local! {
            static LOG: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
        }
        struct Projector;
        impl Drop for Projector {
            fn drop(&mut self) {
                LOG.with(|l| l.borrow_mut().push("projector"));
            }
        }
        struct Weights;
        impl Drop for Weights {
            fn drop(&mut self) {
                LOG.with(|l| l.borrow_mut().push("model"));
            }
        }
        // MIRROR of `ModelWithProjector`'s field order: projector BEFORE model.
        struct Mirror {
            _projector: Option<Projector>,
            _model: Box<Weights>,
        }
        LOG.with(|l| l.borrow_mut().clear());
        drop(Mirror {
            _projector: Some(Projector),
            _model: Box::new(Weights),
        });
        LOG.with(|l| {
            assert_eq!(
                *l.borrow(),
                vec!["projector", "model"],
                "projector (mtmd_free) must drop before model (llama_model_free)"
            );
        });
    }
}
