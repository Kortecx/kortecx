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

use std::ffi::CString;
use std::ptr::NonNull;

use kx_llamacpp_sys as sys;

use crate::backend::LlamaBackend;
use crate::context::Context;
use crate::error::LlamaError;
use crate::vocab::{Token, Vocab};

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

    /// Append a **GBNF grammar** stage that masks every token which would
    /// violate `grammar` (rooted at `root`) to `-inf`, so the downstream
    /// selection stage (greedy / dist) can only pick a grammar-valid
    /// continuation. Place this FIRST in the chain — before any top-k / top-p
    /// truncation — so truncation can never remove the only grammar-valid
    /// token.
    ///
    /// The grammar is parsed at init; an unparseable GBNF makes
    /// `llama_sampler_init_grammar` return NULL, surfaced as
    /// [`LlamaError::SamplerInitFailed`] (the fail-closed boundary for a
    /// malformed grammar).
    ///
    /// # Safety contract
    /// The constructed grammar sampler stores `vocab`'s raw pointer internally
    /// and dereferences it at sampling time. The resulting [`Sampler`] therefore
    /// MUST NOT outlive the [`crate::Model`] that owns `vocab`. Call sites build
    /// and consume the sampler within a single generation scope where the model
    /// is live, which upholds this.
    ///
    /// # Errors
    /// [`LlamaError::GrammarStringInvalid`] if `grammar` / `root` contains an
    /// interior NUL; [`LlamaError::SamplerInitFailed`] on a GBNF parse failure
    /// (or host-OOM).
    pub fn add_grammar(
        self,
        vocab: &Vocab<'_, '_>,
        grammar: &str,
        root: &str,
    ) -> Result<Self, LlamaError> {
        let grammar_c = CString::new(grammar).map_err(|_| LlamaError::GrammarStringInvalid)?;
        let root_c = CString::new(root).map_err(|_| LlamaError::GrammarStringInvalid)?;
        // SAFETY: vocab ptr is borrowed from a live model; the two CStrings
        // outlive this call; llama.cpp parses the grammar during init and
        // returns NULL on parse failure (mapped to SamplerInitFailed by `add`).
        let ptr = unsafe {
            sys::llama_sampler_init_grammar(vocab.as_ptr(), grammar_c.as_ptr(), root_c.as_ptr())
        };
        self.add(ptr, "grammar")
    }

    /// Append a **lazy / triggered** GBNF grammar stage: generation flows
    /// completely unconstrained until one of `trigger_patterns` matches from the
    /// start of the output, at which point the grammar (rooted at `root`) is fed
    /// content starting at the pattern's first match group. This is what lets a
    /// ReAct turn emit a free-form prose answer OR commit to a constrained
    /// tool-call envelope (the grammar arms only once the model types the
    /// tool-call opener).
    ///
    /// Same FIRST-in-chain placement + [safety contract](Self::add_grammar) as
    /// [`Self::add_grammar`]. No trigger *tokens* are used (only patterns).
    ///
    /// # Errors
    /// [`LlamaError::GrammarStringInvalid`] if `grammar` / `root` / any pattern
    /// contains an interior NUL; [`LlamaError::SamplerInitFailed`] on a GBNF
    /// parse failure (or host-OOM).
    pub fn add_grammar_lazy(
        self,
        vocab: &Vocab<'_, '_>,
        grammar: &str,
        root: &str,
        trigger_patterns: &[&str],
    ) -> Result<Self, LlamaError> {
        let grammar_c = CString::new(grammar).map_err(|_| LlamaError::GrammarStringInvalid)?;
        let root_c = CString::new(root).map_err(|_| LlamaError::GrammarStringInvalid)?;
        let pattern_cs: Vec<CString> = trigger_patterns
            .iter()
            .map(|p| CString::new(*p).map_err(|_| LlamaError::GrammarStringInvalid))
            .collect::<Result<_, _>>()?;
        let mut pattern_ptrs: Vec<*const core::ffi::c_char> =
            pattern_cs.iter().map(|c| c.as_ptr()).collect();
        // SAFETY: vocab ptr borrowed from a live model; the grammar/root CStrings
        // and the pattern CStrings (kept alive in `pattern_cs`) outlive this
        // call; `pattern_ptrs` is a contiguous array of `pattern_ptrs.len()`
        // valid C-string pointers; no trigger tokens (null + 0). NULL on parse
        // failure is mapped to SamplerInitFailed by `add`.
        let ptr = unsafe {
            sys::llama_sampler_init_grammar_lazy_patterns(
                vocab.as_ptr(),
                grammar_c.as_ptr(),
                root_c.as_ptr(),
                pattern_ptrs.as_mut_ptr(),
                pattern_ptrs.len(),
                core::ptr::null(),
                0,
            )
        };
        self.add(ptr, "grammar_lazy")
    }

    /// Finalize the chain into a [`Sampler`].
    pub fn build(self) -> Result<Sampler<'b>, LlamaError> {
        Ok(Sampler {
            ptr: self.chain,
            _backend: std::marker::PhantomData,
        })
    }
}
