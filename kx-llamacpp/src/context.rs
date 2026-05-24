//! `Context` + `ContextParams` — RAII inference context, decode, KV-cache
//! management, logits + embeddings readout, and performance counters.

use std::ptr::NonNull;

use kx_llamacpp_sys as sys;

use crate::batch::Batch;
use crate::error::LlamaError;
use crate::model::Model;

/// Pooling strategy for embedding extraction.
///
/// Mirrors `llama_pooling_type`. For text-only generation, leave at
/// [`Self::Unspecified`] (llama.cpp picks per-model).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum PoolingType {
    /// Let llama.cpp pick the per-model default.
    Unspecified = -1,
    /// No pooling — per-token embeddings.
    None = 0,
    /// Mean over all tokens in the sequence.
    Mean = 1,
    /// Use the embedding at the CLS position.
    Cls = 2,
    /// Use the embedding at the last position.
    Last = 3,
    /// Rerank pooling (cls-head for reranker models).
    Rank = 4,
}

/// Builder for inference-context parameters.
///
/// Wraps llama.cpp's default `llama_context_params` and exposes the knobs
/// kortecx cares about. Defaults are inherited from llama.cpp.
pub struct ContextParams {
    inner: sys::llama_context_params,
}

impl Default for ContextParams {
    fn default() -> Self {
        Self::new()
    }
}

impl ContextParams {
    /// Construct with llama.cpp's defaults.
    pub fn new() -> Self {
        // SAFETY: pure C function returning a value struct.
        Self {
            inner: unsafe { sys::llama_context_default_params() },
        }
    }

    /// Text context size in tokens. 0 = take from model.
    pub fn with_n_ctx(mut self, n: u32) -> Self {
        self.inner.n_ctx = n;
        self
    }

    /// Logical maximum batch size submitted to `llama_decode`.
    pub fn with_n_batch(mut self, n: u32) -> Self {
        self.inner.n_batch = n;
        self
    }

    /// Physical maximum batch size.
    pub fn with_n_ubatch(mut self, n: u32) -> Self {
        self.inner.n_ubatch = n;
        self
    }

    /// Maximum number of sequences this context can hold.
    pub fn with_n_seq_max(mut self, n: u32) -> Self {
        self.inner.n_seq_max = n;
        self
    }

    /// Number of threads for generation. 0 = auto (llama.cpp picks).
    pub fn with_n_threads(mut self, n: i32) -> Self {
        self.inner.n_threads = n;
        self
    }

    /// Number of threads for batch processing.
    pub fn with_n_threads_batch(mut self, n: i32) -> Self {
        self.inner.n_threads_batch = n;
        self
    }

    /// Extract embeddings alongside logits during decode.
    pub fn with_embeddings(mut self, on: bool) -> Self {
        self.inner.embeddings = on;
        self
    }

    /// Embedding pooling strategy.
    pub fn with_pooling_type(mut self, pt: PoolingType) -> Self {
        // The bindgen-generated enum's discriminants match the C enum values.
        // SAFETY: transmute is between two `#[repr(i32)]` enums with matching
        // values; both sides are POD i32.
        self.inner.pooling_type =
            unsafe { core::mem::transmute::<i32, sys::llama_pooling_type>(pt as i32) };
        self
    }

    /// Offload the KQV matmul and KV cache to GPU when available.
    pub fn with_offload_kqv(mut self, on: bool) -> Self {
        self.inner.offload_kqv = on;
        self
    }

    /// Disable per-call performance timing collection (default: enabled).
    pub fn with_no_perf(mut self, on: bool) -> Self {
        self.inner.no_perf = on;
        self
    }
}

/// Performance counters snapshot.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PerfData {
    /// Absolute start time in milliseconds.
    pub t_start_ms: f64,
    /// Time spent loading the model in milliseconds.
    pub t_load_ms: f64,
    /// Time spent processing prompt tokens in milliseconds.
    pub t_p_eval_ms: f64,
    /// Time spent generating tokens in milliseconds.
    pub t_eval_ms: f64,
    /// Number of prompt tokens processed.
    pub n_p_eval: i32,
    /// Number of generated tokens.
    pub n_eval: i32,
    /// Number of reused compute graphs (cache hits).
    pub n_reused: i32,
}

/// RAII handle to an inference context for a model.
pub struct Context<'m, 'b: 'm> {
    pub(crate) ptr: NonNull<sys::llama_context>,
    n_vocab: i32,
    n_embd: i32,
    _model: std::marker::PhantomData<&'m Model<'b>>,
}

impl<'m, 'b: 'm> Context<'m, 'b> {
    /// Create a context with llama.cpp defaults.
    ///
    /// # Errors
    /// [`LlamaError::ContextCreationFailed`] if llama.cpp returns null.
    pub fn new(model: &'m Model<'b>) -> Result<Self, LlamaError> {
        Self::new_with_params(model, &ContextParams::new())
    }

    /// Create a context with custom parameters.
    ///
    /// # Errors
    /// [`LlamaError::ContextCreationFailed`] if llama.cpp returns null.
    pub fn new_with_params(
        model: &'m Model<'b>,
        params: &ContextParams,
    ) -> Result<Self, LlamaError> {
        // Cache n_vocab + n_embd so logits/embeddings slices are properly bounded.
        let vocab = model.vocab();
        let n_vocab = vocab.n_tokens();
        let n_embd = model.n_embd();

        // SAFETY: llama_init_from_model returns a raw pointer (null on failure).
        let ctx_ptr = unsafe { sys::llama_init_from_model(model.ptr.as_ptr(), params.inner) };

        let ptr = NonNull::new(ctx_ptr).ok_or(LlamaError::ContextCreationFailed)?;

        Ok(Self {
            ptr,
            n_vocab,
            n_embd,
            _model: std::marker::PhantomData,
        })
    }

    /// The context window size (number of tokens this context can hold).
    pub fn n_ctx(&self) -> u32 {
        // SAFETY: ptr is non-null and points to a live context.
        unsafe { sys::llama_n_ctx(self.ptr.as_ptr()) }
    }

    /// The logical batch size this context was created with.
    pub fn n_batch(&self) -> u32 {
        unsafe { sys::llama_n_batch(self.ptr.as_ptr()) }
    }

    /// The maximum number of sequences this context supports.
    pub fn n_seq_max(&self) -> u32 {
        unsafe { sys::llama_n_seq_max(self.ptr.as_ptr()) }
    }

    /// Run a forward pass over `batch`. Tokens with `compute_logits=true`
    /// will have their logits available via [`Self::logits_ith`].
    ///
    /// # Errors
    /// [`LlamaError::DecodeFailed`] for any non-zero return code.
    pub fn decode(&mut self, batch: &Batch) -> Result<(), LlamaError> {
        // SAFETY: ctx is live; batch.as_raw exposes valid arrays for the
        // duration of this call.
        let rc = unsafe { sys::llama_decode(self.ptr.as_ptr(), batch.as_raw()) };
        if rc == 0 {
            Ok(())
        } else {
            Err(LlamaError::DecodeFailed(rc))
        }
    }

    /// Run the encoder pass (for encoder-decoder models). Most decoder-only
    /// models won't need this.
    ///
    /// # Errors
    /// [`LlamaError::EncodeFailed`] for any non-zero return code.
    pub fn encode(&mut self, batch: &Batch) -> Result<(), LlamaError> {
        let rc = unsafe { sys::llama_encode(self.ptr.as_ptr(), batch.as_raw()) };
        if rc == 0 {
            Ok(())
        } else {
            Err(LlamaError::EncodeFailed(rc))
        }
    }

    /// Logits for the i-th output position in the last decoded batch.
    ///
    /// Returns `None` if `i` references a position whose `compute_logits` was
    /// false at decode time, or if no batch has been decoded yet.
    pub fn logits_ith(&self, i: i32) -> Option<&[f32]> {
        // SAFETY: returned pointer is valid until the next decode call; the
        // slice is bounded by the cached n_vocab (= vocab.n_tokens()).
        let ptr = unsafe { sys::llama_get_logits_ith(self.ptr.as_ptr(), i) };
        if ptr.is_null() {
            None
        } else {
            // SAFETY: llama.cpp guarantees the returned pointer addresses
            // `n_vocab` contiguous floats.
            Some(unsafe { core::slice::from_raw_parts(ptr, self.n_vocab as usize) })
        }
    }

    /// Logits for the last output position (convenience).
    pub fn logits_last(&self) -> Option<&[f32]> {
        self.logits_ith(-1)
    }

    /// Per-token embeddings for the i-th output position. Requires the context
    /// was created with `with_embeddings(true)`.
    pub fn embeddings_ith(&self, i: i32) -> Option<&[f32]> {
        let ptr = unsafe { sys::llama_get_embeddings_ith(self.ptr.as_ptr(), i) };
        if ptr.is_null() {
            None
        } else {
            Some(unsafe { core::slice::from_raw_parts(ptr, self.n_embd as usize) })
        }
    }

    /// Pooled embedding for a sequence. Requires the context was created with
    /// `with_embeddings(true)` and a non-`None` pooling type.
    ///
    /// # Errors
    /// [`LlamaError::EmbeddingsUnavailable`] if pooling is not configured
    /// (the returned pointer would be null).
    pub fn embeddings_seq(&self, seq_id: i32) -> Result<&[f32], LlamaError> {
        let ptr = unsafe { sys::llama_get_embeddings_seq(self.ptr.as_ptr(), seq_id) };
        if ptr.is_null() {
            Err(LlamaError::EmbeddingsUnavailable(
                "no pooled vector for seq_id (pooling disabled or seq absent)",
            ))
        } else {
            // SAFETY: pooled vector is `n_embd` floats.
            Ok(unsafe { core::slice::from_raw_parts(ptr, self.n_embd as usize) })
        }
    }

    // -- KV-cache management ------------------------------------------------

    /// Clear the KV cache. If `data` is true, also zero the underlying buffer
    /// (useful for benchmarking determinism).
    pub fn kv_cache_clear(&mut self, data: bool) {
        // SAFETY: ctx is live; the memory handle is owned by ctx.
        unsafe {
            let mem = sys::llama_get_memory(self.ptr.as_ptr());
            sys::llama_memory_clear(mem, data);
        }
    }

    /// Remove the tokens in `[p0, p1)` from sequence `seq_id`. `p1 < 0` means
    /// "through the end". Returns false if the underlying memory backend
    /// doesn't support the operation.
    pub fn kv_cache_seq_rm(&mut self, seq_id: i32, p0: i32, p1: i32) -> bool {
        unsafe {
            let mem = sys::llama_get_memory(self.ptr.as_ptr());
            sys::llama_memory_seq_rm(mem, seq_id, p0, p1)
        }
    }

    /// Copy positions `[p0, p1)` from `src` to `dst` (creates a parallel
    /// sequence sharing prefix state).
    pub fn kv_cache_seq_cp(&mut self, src: i32, dst: i32, p0: i32, p1: i32) {
        unsafe {
            let mem = sys::llama_get_memory(self.ptr.as_ptr());
            sys::llama_memory_seq_cp(mem, src, dst, p0, p1);
        }
    }

    /// Drop all sequences except `seq_id` from the KV cache.
    pub fn kv_cache_seq_keep(&mut self, seq_id: i32) {
        unsafe {
            let mem = sys::llama_get_memory(self.ptr.as_ptr());
            sys::llama_memory_seq_keep(mem, seq_id);
        }
    }

    /// Maximum position currently held for `seq_id`, or -1 if none.
    pub fn kv_cache_seq_pos_max(&self, seq_id: i32) -> i32 {
        unsafe {
            let mem = sys::llama_get_memory(self.ptr.as_ptr());
            sys::llama_memory_seq_pos_max(mem, seq_id)
        }
    }

    // -- Performance counters ----------------------------------------------

    /// Snapshot of performance counters (load time, prompt eval time,
    /// generation time, token counts).
    pub fn perf(&self) -> PerfData {
        // SAFETY: pure read from ctx.
        let raw = unsafe { sys::llama_perf_context(self.ptr.as_ptr()) };
        PerfData {
            t_start_ms: raw.t_start_ms,
            t_load_ms: raw.t_load_ms,
            t_p_eval_ms: raw.t_p_eval_ms,
            t_eval_ms: raw.t_eval_ms,
            n_p_eval: raw.n_p_eval,
            n_eval: raw.n_eval,
            n_reused: raw.n_reused,
        }
    }

    /// Reset performance counters to zero.
    pub fn perf_reset(&mut self) {
        unsafe { sys::llama_perf_context_reset(self.ptr.as_ptr()) }
    }

    /// Underlying context pointer for the sampler — used by [`crate::Sampler`]
    /// and not exposed publicly. This intentionally lives inside the crate.
    pub(crate) fn raw_mut(&mut self) -> *mut sys::llama_context {
        self.ptr.as_ptr()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pooling_type_repr_matches_c_enum() {
        // Sanity-check that our #[repr(i32)] discriminants match what bindgen
        // expects. Failing this means the model has been updated and the enum
        // variant numbers have shifted.
        assert_eq!(PoolingType::Unspecified as i32, -1);
        assert_eq!(PoolingType::None as i32, 0);
        assert_eq!(PoolingType::Mean as i32, 1);
        assert_eq!(PoolingType::Cls as i32, 2);
        assert_eq!(PoolingType::Last as i32, 3);
        assert_eq!(PoolingType::Rank as i32, 4);
    }

    #[test]
    fn context_params_builder_compiles() {
        // Just exercise the builder surface — no llama state is touched.
        let _p = ContextParams::new()
            .with_n_ctx(512)
            .with_n_batch(64)
            .with_n_ubatch(64)
            .with_n_seq_max(1)
            .with_n_threads(2)
            .with_n_threads_batch(2)
            .with_embeddings(false)
            .with_pooling_type(PoolingType::Unspecified)
            .with_offload_kqv(true)
            .with_no_perf(false);
    }
}
