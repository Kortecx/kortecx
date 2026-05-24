//! Error type for the safe wrapper.

use std::path::PathBuf;
use thiserror::Error;

/// Errors raised by the safe wrapper.
#[derive(Debug, Error)]
pub enum LlamaError {
    /// The model file could not be loaded — bad path, malformed GGUF, or
    /// llama.cpp internally rejected the model.
    #[error("failed to load model from {path:?}")]
    LoadFailed {
        /// The path that was attempted.
        path: PathBuf,
    },

    /// Failed to create an inference context from a model.
    #[error("failed to create inference context")]
    ContextCreationFailed,

    /// The supplied path could not be converted to a C string (interior nul byte
    /// or non-UTF-8 on platforms where C strings must be UTF-8).
    #[error("path is not representable as a C string: {0:?}")]
    PathInvalid(PathBuf),

    /// Failed to tokenize the input text — typically because the supplied
    /// scratch buffer was too small AND llama.cpp rejected the resize loop.
    #[error("tokenization failed (rc = {0})")]
    TokenizeFailed(i32),

    /// Failed to detokenize (write the piece for a token to a buffer).
    #[error("detokenization failed for token {token} (rc = {rc})")]
    DetokenizeFailed {
        /// The token that failed to be detokenized.
        token: i32,
        /// The return code from `llama_token_to_piece`.
        rc: i32,
    },

    /// `llama_decode` returned a non-zero status. 1 = no kv slots, 2 = compute error.
    #[error("llama_decode returned non-zero status {0}")]
    DecodeFailed(i32),

    /// `llama_encode` returned a non-zero status (encoder-decoder models only).
    #[error("llama_encode returned non-zero status {0}")]
    EncodeFailed(i32),

    /// Sampler chain construction failed (e.g. llama_sampler_chain_init returned NULL,
    /// which would only happen under OOM).
    #[error("failed to construct sampler chain")]
    SamplerChainFailed,

    /// A constructor for a particular sampler returned NULL.
    #[error("failed to construct sampler: {0}")]
    SamplerInitFailed(&'static str),

    /// Embedding readout requested but the context was not configured with
    /// `with_embeddings(true)` — or the requested seq_id has no pooled vector.
    #[error("embeddings unavailable: {0}")]
    EmbeddingsUnavailable(&'static str),

    /// An underlying I/O error while reading metadata before passing to llama.cpp.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
