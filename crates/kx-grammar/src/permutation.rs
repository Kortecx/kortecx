//! [`PermutationSpec`] — the constraint for an LLM listwise-RERANK turn (`RC4c`).
//!
//! A rerank turn shows the model `n` retrieved candidate passages and asks it to
//! emit their indices in best→worst order: a JSON array that is a PERMUTATION of
//! `[0, n)`.
//!
//! ## Engine support
//! - **Ollama** renders this as a strict whole-response [`Self::to_ollama_format`]
//!   JSON schema (the rerank turn's entire output is the array, so a strict format is
//!   exactly right — `T-OLLAMA-GRAMMAR-FORMAT`).
//! - **llama.cpp** does NOT constrain it with a GBNF: the char-level grammar sampler
//!   crashes on a digit-array constraint against some tokenizers (Gemma's
//!   digit/punctuation tokens — `T-RERANK-GBNF-CRASH`). The model emits a clean array
//!   after its reasoning anyway, and the parser enforces validity.
//!
//! ## Boundaries (SN-8)
//! Neither path enforces distinctness/range in the model layer. The fail-closed
//! `kx_toolcall::parse_permutation` is the AUTHORITY on validity: a non-permutation
//! output is rejected and the caller keeps the upstream (RRF/MMR) order. The model
//! proposes an order; the runtime enforces exact validity.

use serde::{Deserialize, Serialize};

/// The listwise-rerank constraint: the model must emit a JSON array of `n`
/// integers (the candidate indices, best→worst). `n` is the retrieved-candidate
/// count, known at dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermutationSpec {
    /// The number of candidates to rerank — the array length AND the (exclusive)
    /// upper bound on each index.
    pub n: u32,
}

impl PermutationSpec {
    /// A permutation constraint over `n` candidates.
    #[must_use]
    pub fn new(n: u32) -> Self {
        Self { n }
    }

    /// Render to an Ollama `format` JSON Schema — a whole-response integer array of
    /// length `n` with each item in `[0, n)`. (The rerank turn has exactly ONE
    /// valid output shape, so a strict whole-response Ollama format is unambiguous —
    /// the `T-OLLAMA-GRAMMAR-FORMAT` case that does NOT need a lazy/triggered mode.)
    #[must_use]
    pub fn to_ollama_format(&self) -> serde_json::Value {
        crate::ollama::render_permutation(self.n)
    }
}
