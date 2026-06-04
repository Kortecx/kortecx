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
    ///
    /// **Reachability (SN-4 #4):** the test fixture (`stories260K.gguf`) is a
    /// decoder-only model so `Context::encode` cannot be exercised at the
    /// wrapper layer today. This variant is asserted to be reachable in the
    /// `kx-inference` integration tests at P1.8 once an encoder-decoder model
    /// is part of the test corpus.
    #[error("llama_encode returned non-zero status {0}")]
    EncodeFailed(i32),

    /// Sampler chain construction failed (e.g. `llama_sampler_chain_init`
    /// returned NULL).
    ///
    /// **Reachability (SN-4 #4):** llama.cpp's `llama_sampler_chain_init` is
    /// documented to fail only under host-OOM, which cannot be reliably
    /// induced in a test. The variant is kept for API completeness; if it
    /// ever fires in production it indicates the runtime is in an
    /// unrecoverable allocator state.
    #[error("failed to construct sampler chain")]
    SamplerChainFailed,

    /// A constructor for a particular sampler returned NULL.
    ///
    /// **Reachability (SN-4 #4):** like [`Self::SamplerChainFailed`], NULL
    /// from `llama_sampler_init_*` only occurs under host-OOM. Kept for API
    /// completeness; not testable without a fault-injection harness.
    #[error("failed to construct sampler: {0}")]
    SamplerInitFailed(&'static str),

    /// Embedding readout requested but the context was not configured with
    /// `with_embeddings(true)` — or the requested seq_id has no pooled vector.
    #[error("embeddings unavailable: {0}")]
    EmbeddingsUnavailable(&'static str),

    /// KV-cache state serialization produced fewer bytes than the size getter
    /// promised, OR restoration read 0 bytes (the C-API's "load failed"
    /// signal). Upstream `llama_state_seq_get_data` / `llama_state_seq_set_data`
    /// contract violation.
    #[error("KV-cache state {op}: expected {expected} bytes, got {got}")]
    StateOpFailed {
        /// "save" or "restore".
        op: &'static str,
        /// Bytes expected from the size getter (or expected to read).
        expected: usize,
        /// Bytes actually written / read.
        got: usize,
    },

    /// `llama_chat_apply_template` returned a non-positive status. Typical
    /// cause: the template string is malformed or references unknown
    /// variables.
    #[error("chat-template apply failed (rc = {0})")]
    ChatTemplateFailed(i32),

    /// `mtmd_init_from_file` returned NULL — the multi-modal projector (`mmproj`)
    /// could not be loaded (bad path, malformed projector GGUF, or a projector
    /// incompatible with the text model).
    #[error("failed to initialize mtmd (multi-modal) context from mmproj {path:?}")]
    MtmdInitFailed {
        /// The projector path that was attempted.
        path: PathBuf,
    },

    /// `mtmd_helper_bitmap_init_from_buf` returned NULL — the media bytes could
    /// not be decoded (corrupt/truncated image, or a format stb/miniaudio does
    /// not support). The fail-closed boundary for untrusted media bytes.
    #[error("failed to decode media bytes into an mtmd bitmap ({n_bytes} bytes)")]
    BitmapDecodeFailed {
        /// Length of the buffer that failed to decode.
        n_bytes: usize,
    },

    /// Media bytes decoded as **audio** but the caller only accepts images
    /// (PR-2 is image-only; audio lands in PR-3 behind a default-off feature).
    #[error("audio media is not supported on this path (image-only)")]
    AudioNotSupported,

    /// `mtmd_tokenize` returned a non-zero status. 1 = the number of media
    /// markers in the text does not match the number of bitmaps; 2 = media
    /// preprocessing failed.
    #[error("mtmd_tokenize returned non-zero status {0}")]
    TokenizeChunksFailed(i32),

    /// `mtmd_helper_eval_chunks` returned a non-zero status — a text/image chunk
    /// failed to encode or `llama_decode` failed during the multi-modal prefill.
    #[error("mtmd_helper_eval_chunks returned non-zero status {0}")]
    EvalChunksFailed(i32),

    /// An underlying I/O error while reading metadata before passing to llama.cpp.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
