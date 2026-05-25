//! `Sampler` — RAII wrapper over `llama_sampler` chains.
//!
//! A sampler is a stack of token-selection stages. The most common shape is:
//!
//! ```text
//! [top_k | top_p | min_p | temp]  →  dist(seed)  →  token
//! ```
//!
//! Greedy sampling collapses the whole stack to a single `argmax` stage.
//!
//! Lifetime: a [`Sampler`] borrows the [`crate::LlamaBackend`] so that the
//! backend outlives every sampler chain.

use std::ptr::NonNull;

use kx_llamacpp_sys as sys;

use crate::backend::LlamaBackend;
use crate::context::Context;
use crate::error::LlamaError;
use crate::vocab::Token;

/// RAII handle to a `llama_sampler` chain.
///
/// # Examples
///
/// Build a greedy sampler:
///
/// ```
/// use kx_llamacpp::{LlamaBackend, Sampler};
///
/// let backend = LlamaBackend::new().unwrap();
/// let _greedy = Sampler::greedy(&backend).unwrap();
/// ```
///
/// Build a typical (top-k + top-p + temp + dist) chain:
///
/// ```
/// use kx_llamacpp::{LlamaBackend, Sampler};
///
/// let backend = LlamaBackend::new().unwrap();
/// let _typical = Sampler::typical(
///     &backend,
///     /* temp     */ 0.7,
///     /* top_k    */ 40,
///     /* top_p    */ 0.95,
///     /* seed     */ 42,
/// )
/// .unwrap();
/// ```
pub struct Sampler<'b> {
    ptr: NonNull<sys::llama_sampler>,
    _backend: std::marker::PhantomData<&'b LlamaBackend>,
}

impl<'b> Sampler<'b> {
    /// Construct an empty sampler chain ready for stages to be added.
    pub fn chain(backend: &'b LlamaBackend) -> SamplerChainBuilder<'b> {
        SamplerChainBuilder::new(backend)
    }

    /// Convenience: a greedy (argmax) sampler.
    ///
    /// # Errors
    /// [`LlamaError::SamplerInitFailed`] under OOM.
    pub fn greedy(backend: &'b LlamaBackend) -> Result<Self, LlamaError> {
        Self::chain(backend).add_greedy()?.build()
    }

    /// Convenience: a temperature + top-k + top-p + dist chain.
    ///
    /// # Errors
    /// [`LlamaError::SamplerInitFailed`] / [`LlamaError::SamplerChainFailed`].
    pub fn typical(
        backend: &'b LlamaBackend,
        temperature: f32,
        top_k: i32,
        top_p: f32,
        seed: u32,
    ) -> Result<Self, LlamaError> {
        Self::chain(backend)
            .add_top_k(top_k)?
            .add_top_p(top_p, 1)?
            .add_temp(temperature)?
            .add_dist(seed)?
            .build()
    }

    /// Sample the next token using the i-th set of logits in the last decoded
    /// batch. Use `-1` to sample from the last position.
    #[tracing::instrument(level = "trace", skip(self, ctx))]
    pub fn sample(&mut self, ctx: &mut Context<'_, '_>, idx: i32) -> Token {
        // SAFETY: sampler + ctx are live; the sampler reads logits via ctx.
        Token(unsafe { sys::llama_sampler_sample(self.ptr.as_ptr(), ctx.raw_mut(), idx) })
    }

    /// Inform the sampler that `token` was accepted (so stateful samplers like
    /// repetition penalties / mirostat can update their history).
    pub fn accept(&mut self, token: Token) {
        unsafe { sys::llama_sampler_accept(self.ptr.as_ptr(), token.0) }
    }

    /// Reset internal sampler state (clears repetition history, mirostat
    /// state, etc.). Does not change the chain composition.
    pub fn reset(&mut self) {
        unsafe { sys::llama_sampler_reset(self.ptr.as_ptr()) }
    }
}

impl<'b> Drop for Sampler<'b> {
    fn drop(&mut self) {
        // SAFETY: ptr was produced by llama_sampler_chain_init; Drop runs once.
        // llama.cpp frees the chain and all child samplers added via
        // llama_sampler_chain_add.
        unsafe { sys::llama_sampler_free(self.ptr.as_ptr()) }
    }
}

unsafe impl<'b> Send for Sampler<'b> {}

/// Builder for a sampler chain.
///
/// Each `add_*` returns `Result<Self, LlamaError>` so a NULL from llama.cpp
/// (only realistic under OOM) is surfaced rather than ignored. Call
/// [`Self::build`] to materialize a [`Sampler`].
pub struct SamplerChainBuilder<'b> {
    chain: NonNull<sys::llama_sampler>,
    _backend: std::marker::PhantomData<&'b LlamaBackend>,
}

impl<'b> SamplerChainBuilder<'b> {
    fn new(_backend: &'b LlamaBackend) -> Self {
        // SAFETY: pure C function returning a value struct (chain params).
        let params = unsafe { sys::llama_sampler_chain_default_params() };
        // SAFETY: returns a fresh chain pointer; NULL only under OOM.
        let chain_ptr = unsafe { sys::llama_sampler_chain_init(params) };
        let chain = NonNull::new(chain_ptr).expect("sampler_chain_init returned NULL");
        Self {
            chain,
            _backend: std::marker::PhantomData,
        }
    }

    fn add(
        self,
        stage_ptr: *mut sys::llama_sampler,
        name: &'static str,
    ) -> Result<Self, LlamaError> {
        if stage_ptr.is_null() {
            return Err(LlamaError::SamplerInitFailed(name));
        }
        // SAFETY: chain owns the stage after this call (frees it on chain free).
        unsafe { sys::llama_sampler_chain_add(self.chain.as_ptr(), stage_ptr) };
        Ok(self)
    }

    /// Append a greedy (argmax) stage. Typically used as the last stage of a
    /// chain that has already narrowed the candidate set.
    pub fn add_greedy(self) -> Result<Self, LlamaError> {
        let ptr = unsafe { sys::llama_sampler_init_greedy() };
        self.add(ptr, "greedy")
    }

    /// Append a sampling-by-distribution stage seeded with `seed` (final
    /// stochastic step of a typical chain).
    pub fn add_dist(self, seed: u32) -> Result<Self, LlamaError> {
        let ptr = unsafe { sys::llama_sampler_init_dist(seed) };
        self.add(ptr, "dist")
    }

    /// Append a top-k truncation stage.
    pub fn add_top_k(self, k: i32) -> Result<Self, LlamaError> {
        let ptr = unsafe { sys::llama_sampler_init_top_k(k) };
        self.add(ptr, "top_k")
    }

    /// Append a top-p (nucleus) truncation stage.
    pub fn add_top_p(self, p: f32, min_keep: usize) -> Result<Self, LlamaError> {
        let ptr = unsafe { sys::llama_sampler_init_top_p(p, min_keep) };
        self.add(ptr, "top_p")
    }

    /// Append a min-p truncation stage.
    pub fn add_min_p(self, p: f32, min_keep: usize) -> Result<Self, LlamaError> {
        let ptr = unsafe { sys::llama_sampler_init_min_p(p, min_keep) };
        self.add(ptr, "min_p")
    }

    /// Append a temperature scaling stage.
    pub fn add_temp(self, t: f32) -> Result<Self, LlamaError> {
        let ptr = unsafe { sys::llama_sampler_init_temp(t) };
        self.add(ptr, "temp")
    }

    /// Append an extended temperature stage (dynamic temperature).
    pub fn add_temp_ext(self, t: f32, delta: f32, exponent: f32) -> Result<Self, LlamaError> {
        let ptr = unsafe { sys::llama_sampler_init_temp_ext(t, delta, exponent) };
        self.add(ptr, "temp_ext")
    }

    /// Finalize the chain into a [`Sampler`].
    pub fn build(self) -> Result<Sampler<'b>, LlamaError> {
        Ok(Sampler {
            ptr: self.chain,
            _backend: std::marker::PhantomData,
        })
    }
}
