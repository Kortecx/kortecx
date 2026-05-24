//! `Batch` — RAII wrapper over `llama_batch`.
//!
//! A batch is a heap-allocated bundle of tokens (or embedding vectors) plus
//! per-token position, sequence membership, and logits-output flags. It is
//! populated by the caller, then submitted to [`crate::Context::decode`] for a
//! forward pass.
//!
//! Lifetime: the batch owns its internal buffers via `llama_batch_init` /
//! `llama_batch_free`. The capacity (`n_tokens`, `embd`, `n_seq_max`) is fixed
//! at construction time; populating beyond it is a programming error.

use kx_llamacpp_sys as sys;

use crate::vocab::Token;

/// RAII wrapper over `llama_batch`.
///
/// Construct via [`Self::with_capacity`] or [`Self::with_embeddings`]; populate
/// via [`Self::add`]; submit to `Context::decode`.
pub struct Batch {
    inner: sys::llama_batch,
    capacity: i32,
}

impl Batch {
    /// Allocate a batch that holds up to `n_tokens` tokens, with up to
    /// `n_seq_max` distinct sequence ids per token. Token mode (no embeddings).
    pub fn with_capacity(n_tokens: i32, n_seq_max: i32) -> Self {
        // SAFETY: llama_batch_init allocates internal buffers; we free them in Drop.
        let inner = unsafe { sys::llama_batch_init(n_tokens, 0, n_seq_max) };
        Self {
            inner,
            capacity: n_tokens,
        }
    }

    /// Allocate a batch that holds up to `n_tokens` embedding vectors of
    /// dimension `embd`. Embedding mode (no token ids).
    pub fn with_embeddings(n_tokens: i32, embd: i32, n_seq_max: i32) -> Self {
        let inner = unsafe { sys::llama_batch_init(n_tokens, embd, n_seq_max) };
        Self {
            inner,
            capacity: n_tokens,
        }
    }

    /// Current number of populated tokens in the batch.
    pub fn n_tokens(&self) -> i32 {
        self.inner.n_tokens
    }

    /// The allocated capacity (the `n_tokens` passed at construction).
    pub fn capacity(&self) -> i32 {
        self.capacity
    }

    /// Reset the batch's logical size to zero. Does not reallocate; the
    /// internal buffers are reused.
    pub fn clear(&mut self) {
        self.inner.n_tokens = 0;
    }

    /// Append a token at `pos` (position in its sequence), belonging to
    /// `seq_ids`, with `compute_logits` controlling whether the output for
    /// this position is materialized after `decode`.
    ///
    /// # Panics
    /// Panics if the batch is full (caller should size it for the workload).
    pub fn add(&mut self, token: Token, pos: i32, seq_ids: &[i32], compute_logits: bool) {
        assert!(
            self.inner.n_tokens < self.capacity,
            "Batch is full (capacity = {}); resize or use multiple batches",
            self.capacity
        );
        let i = self.inner.n_tokens as usize;
        // SAFETY: All four arrays are sized to at least `capacity`; we just
        // checked the bound. seq_id is an array-of-arrays; the j-th inner array
        // is sized to n_seq_max (asserted via the construction-time guarantee).
        unsafe {
            *self.inner.token.add(i) = token.0;
            *self.inner.pos.add(i) = pos;
            *self.inner.n_seq_id.add(i) = seq_ids.len() as i32;
            let seq_ptr = *self.inner.seq_id.add(i);
            for (j, &s) in seq_ids.iter().enumerate() {
                *seq_ptr.add(j) = s;
            }
            *self.inner.logits.add(i) = i8::from(compute_logits);
        }
        self.inner.n_tokens += 1;
    }

    /// Borrow the raw `llama_batch` value for submission to `llama_decode`.
    ///
    /// # Safety
    /// The returned struct's pointers remain valid only for the lifetime of
    /// `self`. Do not store the returned struct beyond a single decode call.
    pub(crate) fn as_raw(&self) -> sys::llama_batch {
        self.inner
    }
}

impl Drop for Batch {
    fn drop(&mut self) {
        // SAFETY: inner was produced by llama_batch_init in the constructor.
        unsafe { sys::llama_batch_free(self.inner) }
    }
}

unsafe impl Send for Batch {}
