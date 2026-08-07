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
//!  - A decode call returns an error (yielded as `Some(Err(...))`, then `None`).
//!  - The context's `n_ctx` is exhausted — yielded as
//!    `Some(Err(`[`LlamaError::ContextExhausted`]`))`, then `None`.
//!
//! ⚠ The last two are ERRORS, not ends-of-stream, and the exhaustion case is the
//! reason why. `None` means "the model finished"; a full context means "the model
//! was CUT OFF". Reporting both as `None` made a truncated generation
//! indistinguishable from a complete one, which is how a clipped tool call read as
//! a model that would not emit one.
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

/// Has the sequence reached the context window?
///
/// Pure + total, and extracted out of [`Iterator::next`] so the BOUNDARY can be
/// pinned without a GGUF.
///
/// ⚠ **What this does and does not buy, stated precisely, because the difference
/// was measured rather than assumed.** The unit tests below hold two things: that
/// the boundary is `>=` and not `>`, and that [`LlamaError::ContextExhausted`]
/// says the output was truncated. They do NOT hold the thing that actually
/// regressed — the CHANNEL. Reverting [`Iterator::next`] to `return None` here
/// leaves every test in this module GREEN (verified: 3 passed under exactly that
/// mutation), because no test that cannot load a model can observe what `next()`
/// returns. The channel is held ONLY by
/// `tests/smoke.rs::smoke_generator_reports_a_full_context_instead_of_stopping_silently`,
/// which is behind the `model-smoke-test` feature and is NOT run by `just ci` —
/// run it with `just smoke-test-with-model` before trusting this path.
#[must_use]
fn context_is_exhausted(pos: i32, n_ctx: u32) -> bool {
    match u32::try_from(pos) {
        Ok(p) => p >= n_ctx,
        // A negative position cannot arise (it is only incremented from a
        // non-negative prefill), but it is treated as exhausted rather than
        // ignored: stopping is recoverable, an unbounded decode loop is not.
        Err(_) => true,
    }
}

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

    /// Construct a generator over an **already-prefilled** context, positioned
    /// at `n_past`. Unlike [`Self::new`], this does NOT decode a prompt — the
    /// caller has already populated the KV cache (e.g. via the multi-modal
    /// prefill `mtmd_helper_eval_chunks`, run with `logits_last = true` so the
    /// last position's logits are ready for the first sample). The iterator
    /// then continues the ordinary sample → decode → feed-back loop verbatim.
    ///
    /// `n_past` is the number of positions already in sequence 0 (the value
    /// returned by the prefill).
    pub fn from_prefilled(
        ctx: &'ctx mut Context<'m, 'b>,
        sampler: &'s mut Sampler<'b>,
        vocab: &'v Vocab<'m, 'b>,
        n_past: i32,
    ) -> Self {
        Self {
            ctx,
            sampler,
            vocab,
            pos: n_past,
            done: false,
        }
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
        if context_is_exhausted(self.pos, self.ctx.n_ctx()) {
            self.done = true;
            let pos = u32::try_from(self.pos).unwrap_or(u32::MAX);
            return Some(Err(LlamaError::ContextExhausted {
                n_ctx: self.ctx.n_ctx(),
                pos,
            }));
        }

        // 1. Sample the next token from the last decoded position's logits.
        //
        // ⚠ DO NOT call `sampler.accept(token)` here. `llama_sampler_sample` ALREADY ends
        // with `llama_sampler_accept(smpl, token)` (llama-sampler.cpp), so an accept here
        // is a SECOND accept of the same token.
        //
        // That was harmless for years and is not harmless now. A stateless chain ignores
        // accept, and repetition penalties merely double-count — but a LAZY GRAMMAR is
        // stateful in a way that cannot survive it. The first accept of the tool-call
        // opener fires the trigger and REPLAYS that opener into the grammar; the second
        // accept feeds the same text to a grammar that has already consumed it, every
        // parse stack dies, and `llama_grammar_accept_token` throws — through an
        // `extern "C"` boundary, which Rust cannot catch. The process ABORTS mid-decode.
        //
        // Measured: with the double accept, the engagement counters climbed by 2 per
        // token and a Gemma-4 tool call aborted the owner thread the instant its trigger
        // fired. This is why the counter has an `awaiting` field at all.
        let token = self.sampler.sample(self.ctx, -1);

        // 2. Stop after yielding EOG — the model has signaled it's done.
        if token.is_eog(self.vocab) {
            self.done = true;
            return Some(Ok(token));
        }

        // 3. Decode the new token at the current position so the NEXT call
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

#[cfg(test)]
mod tests {
    use super::{context_is_exhausted, LlamaError};

    /// The boundary itself. `pos` counts tokens ALREADY decoded, so `pos == n_ctx`
    /// means the window is full and the next token has nowhere to go — the `>=`
    /// is load-bearing and an off-by-one here would let a decode run one token
    /// past the window.
    #[test]
    fn exhaustion_is_reached_at_the_window_not_past_it() {
        assert!(!context_is_exhausted(0, 8), "an empty window is not full");
        assert!(!context_is_exhausted(7, 8), "one slot left is not full");
        assert!(context_is_exhausted(8, 8), "pos == n_ctx IS full");
        assert!(context_is_exhausted(9, 8), "past the window is full");
        // A zero-length window is full before any token is decoded.
        assert!(context_is_exhausted(0, 0));
    }

    /// A position that cannot be represented is treated as exhausted. The old
    /// code reached the same verdict by `pos as u32` WRAPPING a negative into a
    /// huge value — accidentally right, for a reason that would not survive
    /// anyone changing the cast. Pinned so the intent outlives the arithmetic.
    #[test]
    fn an_impossible_position_stops_rather_than_loops() {
        assert!(context_is_exhausted(-1, 8));
        assert!(context_is_exhausted(i32::MIN, u32::MAX));
    }

    /// The MESSAGE half of the silent-truncation fix.
    ///
    /// ⚠ Deliberately NOT called a regression guard for the defect: the defect was
    /// a wrong CHANNEL (`None`, the same signal a finished model sends), and this
    /// test cannot see the channel. It pins what it can — that exhaustion has an
    /// error variant at all, and that the variant tells a reader the output is
    /// INCOMPLETE rather than merely short. See `context_is_exhausted` for which
    /// test actually holds the channel, and why it is not in `just ci`.
    #[test]
    fn exhaustion_reports_truncation_rather_than_completion() {
        let e = LlamaError::ContextExhausted {
            n_ctx: 8192,
            pos: 8192,
        };
        let msg = e.to_string();
        assert!(
            msg.contains("context window exhausted"),
            "the operator must be able to name the condition: {msg}"
        );
        assert!(
            msg.contains("truncated"),
            "a caller reading this must learn the output is INCOMPLETE, which is \
             the whole difference from a clean stop: {msg}"
        );
        assert!(
            msg.contains("8192"),
            "the window that was hit is the actionable number: {msg}"
        );
    }
}
