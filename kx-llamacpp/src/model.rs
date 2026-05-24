//! `Model` + `ModelParams` — RAII model handle plus the loader's builder.

use std::ffi::CString;
use std::path::Path;
use std::ptr::NonNull;

use kx_llamacpp_sys as sys;

use crate::backend::LlamaBackend;
use crate::batch::Batch;
use crate::context::{Context, ContextParams, PoolingType};
use crate::error::LlamaError;
use crate::vocab::Vocab;

/// Builder for the model-loader parameters.
///
/// Wraps llama.cpp's default `llama_model_params` and exposes the knobs
/// kortecx cares about. Defaults are inherited from llama.cpp so every field
/// you don't touch stays at the upstream-recommended value.
pub struct ModelParams {
    inner: sys::llama_model_params,
}

impl Default for ModelParams {
    fn default() -> Self {
        Self::new()
    }
}

impl ModelParams {
    /// Construct with llama.cpp's defaults.
    pub fn new() -> Self {
        // SAFETY: llama_model_default_params is a pure C function that returns
        // a value struct populated with safe defaults.
        Self {
            inner: unsafe { sys::llama_model_default_params() },
        }
    }

    /// Number of layers to offload to GPU. Negative = all. Per D28 CUDA is
    /// disabled in OSS builds, so this only has effect when llama.cpp's
    /// platform-default GPU backend is enabled (e.g. Metal on Apple Silicon).
    pub fn with_n_gpu_layers(mut self, n: i32) -> Self {
        self.inner.n_gpu_layers = n;
        self
    }

    /// Load only the vocabulary, no weights. Useful for tokenizer-only access.
    pub fn with_vocab_only(mut self, on: bool) -> Self {
        self.inner.vocab_only = on;
        self
    }

    /// Use mmap if the platform supports it (default true on most OSes).
    pub fn with_use_mmap(mut self, on: bool) -> Self {
        self.inner.use_mmap = on;
        self
    }

    /// Lock model into RAM (prevents paging). Default false.
    pub fn with_use_mlock(mut self, on: bool) -> Self {
        self.inner.use_mlock = on;
        self
    }

    /// Validate tensor data when loading (slower; default false).
    pub fn with_check_tensors(mut self, on: bool) -> Self {
        self.inner.check_tensors = on;
        self
    }
}

/// RAII handle to a loaded llama.cpp model.
///
/// Construction performs the model load; Drop calls `llama_model_free`. The
/// `Model` borrows from the `LlamaBackend` so the backend outlives the model.
pub struct Model<'b> {
    pub(crate) ptr: NonNull<sys::llama_model>,
    _backend: std::marker::PhantomData<&'b LlamaBackend>,
}

impl<'b> Model<'b> {
    /// Load a model from a GGUF file using llama.cpp's default model parameters.
    ///
    /// # Errors
    /// - [`LlamaError::PathInvalid`] if `path` cannot be converted to a C string.
    /// - [`LlamaError::LoadFailed`] if llama.cpp returns a null pointer.
    pub fn load(backend: &'b LlamaBackend, path: impl AsRef<Path>) -> Result<Self, LlamaError> {
        Self::load_with_params(backend, path, &ModelParams::new())
    }

    /// Load a model from a GGUF file using the supplied parameters.
    ///
    /// # Errors
    /// Same as [`Self::load`].
    #[tracing::instrument(level = "info", skip(_backend, params), fields(path = %path.as_ref().display()))]
    pub fn load_with_params(
        _backend: &'b LlamaBackend,
        path: impl AsRef<Path>,
        params: &ModelParams,
    ) -> Result<Self, LlamaError> {
        let path_ref = path.as_ref();
        let c_path = CString::new(path_ref.as_os_str().to_string_lossy().as_bytes())
            .map_err(|_| LlamaError::PathInvalid(path_ref.to_owned()))?;

        // SAFETY: llama_model_load_from_file is unsafe because it reads from
        // disk and returns a raw pointer (null on failure). We pass a valid
        // params struct (copy-by-value) and a NUL-terminated path.
        let model_ptr = unsafe { sys::llama_model_load_from_file(c_path.as_ptr(), params.inner) };

        let ptr = NonNull::new(model_ptr).ok_or_else(|| LlamaError::LoadFailed {
            path: path_ref.to_owned(),
        })?;

        tracing::debug!(
            n_params = unsafe { sys::llama_model_n_params(ptr.as_ptr()) },
            size_bytes = unsafe { sys::llama_model_size(ptr.as_ptr()) },
            "model loaded"
        );

        Ok(Self {
            ptr,
            _backend: std::marker::PhantomData,
        })
    }

    /// Get the vocabulary owned by this model. The returned [`Vocab`] borrows
    /// from `self` and is invalidated when the model is dropped.
    pub fn vocab(&self) -> Vocab<'_, 'b> {
        // SAFETY: llama_model_get_vocab returns a non-null pointer into the
        // model's owned vocab; the lifetime of the returned reference is tied
        // to `self` so the vocab cannot outlive the model.
        let raw = unsafe { sys::llama_model_get_vocab(self.ptr.as_ptr()) };
        // llama.cpp guarantees a non-null vocab for any successfully-loaded model.
        let ptr = NonNull::new(raw.cast_mut()).expect("model_get_vocab returned NULL");
        Vocab::from_raw(ptr)
    }

    /// Embedding dimensionality.
    pub fn n_embd(&self) -> i32 {
        // SAFETY: ptr is non-null and points to a live model owned by this Model.
        unsafe { sys::llama_model_n_embd(self.ptr.as_ptr()) }
    }

    /// Training context size (max position embedding).
    pub fn n_ctx_train(&self) -> i32 {
        unsafe { sys::llama_model_n_ctx_train(self.ptr.as_ptr()) }
    }

    /// Number of transformer layers.
    pub fn n_layer(&self) -> i32 {
        unsafe { sys::llama_model_n_layer(self.ptr.as_ptr()) }
    }

    /// Number of attention heads.
    pub fn n_head(&self) -> i32 {
        unsafe { sys::llama_model_n_head(self.ptr.as_ptr()) }
    }

    /// Number of key/value attention heads (grouped-query attention).
    pub fn n_head_kv(&self) -> i32 {
        unsafe { sys::llama_model_n_head_kv(self.ptr.as_ptr()) }
    }

    /// Total parameter count.
    pub fn n_params(&self) -> u64 {
        unsafe { sys::llama_model_n_params(self.ptr.as_ptr()) }
    }

    /// On-disk model size in bytes.
    pub fn size(&self) -> u64 {
        unsafe { sys::llama_model_size(self.ptr.as_ptr()) }
    }

    /// One-shot **HF-shaped embedding**: tokenize `text`, decode in an
    /// embedding-mode context, return the mean-pooled embedding vector.
    ///
    /// This is the unit-level mirror of HuggingFace Transformers'
    /// `model.encode(text)` — three lines instead of forty. For batching or
    /// finer control over pooling, construct a [`Context`] manually with
    /// the desired `ContextParams`.
    ///
    /// Per the cross-backend symmetry contract (P5.1 / P5.1.5), this is the
    /// **same shape** `kx-cloud-inference-vllm` and `kx-cloud-inference-sglang`
    /// will expose: `embed(text) -> Vec<f32>`.
    ///
    /// # Errors
    /// - [`LlamaError::TokenizeFailed`] if tokenization fails.
    /// - [`LlamaError::ContextCreationFailed`] if a fresh context cannot be
    ///   allocated.
    /// - [`LlamaError::DecodeFailed`] if the decode pass fails.
    /// - [`LlamaError::EmbeddingsUnavailable`] if pooling didn't produce a
    ///   vector (model lacks the necessary metadata).
    ///
    /// # Determinism
    /// `embed(x)` is deterministic for fixed `x` and fixed model — proved by
    /// `smoke_embed_one_shot_determinism` in `tests/smoke.rs`.
    #[tracing::instrument(level = "info", skip(self), fields(text_len = text.len()))]
    pub fn embed(&self, text: &str) -> Result<Vec<f32>, LlamaError> {
        let vocab = self.vocab();
        let tokens = vocab.tokenize(text, /* add_special */ true, false)?;
        if tokens.is_empty() {
            return Err(LlamaError::TokenizeFailed(0));
        }

        // Embedding-mode context, mean-pool across the sequence.
        let params = ContextParams::new()
            .with_n_ctx(tokens.len().max(8) as u32 * 2)
            .with_n_batch(tokens.len() as u32)
            .with_n_ubatch(tokens.len() as u32)
            .with_n_seq_max(1)
            .with_embeddings(true)
            .with_pooling_type(PoolingType::Mean);
        let mut ctx = Context::new_with_params(self, &params)?;

        // Decode the prompt; mean-pool reads from all positions, so every
        // position needs compute_logits = true.
        let mut batch = Batch::with_capacity(tokens.len() as i32, 1);
        for (i, &t) in tokens.iter().enumerate() {
            batch.add(t, i as i32, &[0], true);
        }
        ctx.decode(&batch)?;

        // Read the pooled vector. Mean pooling produces one vector per
        // sequence; we own seq 0.
        let pooled = ctx.embeddings_seq(0)?;
        Ok(pooled.to_vec())
    }

    /// Human-readable model description (e.g. "llama 7B Q4_0").
    pub fn desc(&self) -> String {
        let mut buf = [0u8; 256];
        // SAFETY: buf is mut-borrowed; len is its capacity; llama writes at
        // most `len` bytes and returns the length written.
        let n = unsafe {
            sys::llama_model_desc(
                self.ptr.as_ptr(),
                buf.as_mut_ptr().cast::<core::ffi::c_char>(),
                buf.len(),
            )
        };
        let n = n.max(0) as usize;
        String::from_utf8_lossy(&buf[..n.min(buf.len())]).into_owned()
    }
}

impl<'b> Drop for Model<'b> {
    fn drop(&mut self) {
        // SAFETY: ptr is non-null and was produced by llama_model_load_from_file;
        // Drop runs exactly once.
        unsafe { sys::llama_model_free(self.ptr.as_ptr()) }
    }
}

// Model is Send if the underlying pointer is opaque + llama.cpp is documented
// as per-model-thread-safe-for-non-mutating-reads. We don't claim Sync; callers
// wrap in Mutex if cross-thread mutation is needed.
unsafe impl<'b> Send for Model<'b> {}
