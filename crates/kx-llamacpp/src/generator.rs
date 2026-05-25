//! `Generator` — HF-shaped one-shot generation iterator.
//!
//! Bundles a [`crate::Context`], a [`crate::Sampler`], a [`crate::Vocab`], and
//! a tokenized prompt into an `Iterator<Item = Result<Token, LlamaError>>`,
//! so callers write:
//!
//! ```ignore
//! let prompt_tokens = vocab.tokenize("Hello", true, false)?;
//! let mut gen = Generator::new(&mut ctx, &mut sampler, &vocab, prompt_tokens)?;
//! let tokens: Vec<Token> = gen.by_ref().take(32).collect::<Result<_, _>>()?;
//! ```
//!
//! instead of the manual decode → sample → feed-back-into-batch loop.
//!
//! ## Stopping
//!
//! The iterator yields tokens until:
//!  - The caller stops calling `next()` (typical case — combined with `take(N)`).
//!  - The model emits a token for which [`crate::Vocab::is_eog`] returns true.
//!  - The context's `n_ctx` is exhausted (returns `None`).
//!  - A decode call returns an error (yielded as `Some(Err(...))`, then `None`).
//!
//! ## Cross-backend symmetry
//!
//! The shape of `Generator` is the contract every future `InferenceBackend`
//! adapter is intended to mirror: an `Iterator<Item = Result<Token, _>>` over
//! a token stream, regardless of whether the underlying engine runs in
//! process or over the network, and regardless of whether the engine
//! exploits batching, prefix reuse, or speculative decoding under the hood.
//! Adding a new backend amounts to "implement this iterator."

use crate::batch::Batch;
use crate::context::Context;
use crate::error::LlamaError;
use crate::sampler::Sampler;
use crate::vocab::{Token, Vocab};

/// HF-shaped generation iterator. Yields `Result<Token, LlamaError>` until
/// the model emits EOG, the context fills up, or the caller stops asking.
pub struct Generator<'ctx, 'm, 'b, 's, 'v> {
    ctx: &'ctx mut Context<'m, 'b>,
    sampler: &'s mut Sampler<'b>,
    vocab: &'v Vocab<'m, 'b>,
    /// Current position in the sequence (i.e. how many tokens have been
    /// decoded into seq 0 so far).
    pos: i32,
    /// Whether the iterator has terminated (EOG emitted, context full, or
    /// a decode error occurred). Once `done`, `next()` returns `None`.
    done: bool,
}

impl<'ctx, 'm, 'b, 's, 'v> Generator<'ctx, 'm, 'b, 's, 'v> {
    /// Construct a generator: tokenize the prompt, decode it into `ctx`, and
    /// position the iterator at the next token to be sampled.
    ///
    /// The KV cache for sequence 0 is populated by this constructor. Callers
    /// who want a fresh KV cache should construct a fresh [`Context`] or
    /// call [`Context::kv_cache_clear`] beforehand.
    ///
    /// # Errors
    /// - [`LlamaError::DecodeFailed`] if the initial prompt decode fails.
    pub fn new(
        ctx: &'ctx mut Context<'m, 'b>,
        sampler: &'s mut Sampler<'b>,
        vocab: &'v Vocab<'m, 'b>,
        prompt_tokens: Vec<Token>,
    ) -> Result<Self, LlamaError> {
        assert!(
            !prompt_tokens.is_empty(),
            "Generator requires at least one prompt token; tokenize first then pass the vec"
        );

        // Decode the entire prompt; only the last position needs logits.
        let n = prompt_tokens.len();
        let mut batch = Batch::with_capacity(n as i32, 1);
        for (i, &t) in prompt_tokens.iter().enumerate() {
            let last = i + 1 == n;
            batch.add(t, i as i32, &[0], last);
        }
        ctx.decode(&batch)?;

        Ok(Self {
            ctx,
            sampler,
            vocab,
            pos: n as i32, // next token will be sampled into this position
            done: false,
        })
    }

    /// Maximum sequence length this iterator can produce before the context
    /// window is exhausted.
    pub fn n_ctx(&self) -> u32 {
        self.ctx.n_ctx()
    }

    /// Current position (= number of tokens already decoded into seq 0).
    pub fn pos(&self) -> i32 {
        self.pos
    }
}

impl<'ctx, 'm, 'b, 's, 'v> Iterator for Generator<'ctx, 'm, 'b, 's, 'v> {
    type Item = Result<Token, LlamaError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        if (self.pos as u32) >= self.ctx.n_ctx() {
            self.done = true;
            return None;
        }

        // 1. Sample the next token from the last decoded position's logits.
        let token = self.sampler.sample(self.ctx, -1);

        // 2. Inform the sampler (no-op for stateless chains; matters for
        //    repetition penalties / mirostat).
        self.sampler.accept(token);

        // 3. Stop after yielding EOG — the model has signaled it's done.
        if token.is_eog(self.vocab) {
            self.done = true;
            return Some(Ok(token));
        }

        // 4. Decode the new token at the current position so the NEXT call
        //    to next() can sample from updated logits.
        let mut step = Batch::with_capacity(1, 1);
        step.add(token, self.pos, &[0], true);
        if let Err(e) = self.ctx.decode(&step) {
            self.done = true;
            return Some(Err(e));
        }
        self.pos += 1;

        Some(Ok(token))
    }
}
