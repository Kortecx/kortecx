// Public types crossing the dispatcher / backend seam.
//
// Two forward-compat hooks live here per the PR 8 strategy plan:
//   - `InferenceInput::Multimodal` reserved variant (no impl in v0.1).
//   - `InferenceParams.grammar` reserved Option field (no impl in v0.1).
//
// Both return `InferenceError::Unsupported` when used against the OSS
// `LlamaInferenceBackend` — the seam is exercised by tests so the
// `Err(Unsupported)` path is documented, not merely declarative.

use std::time::Duration;

use kx_content::ContentRef;
use kx_mote::ModelId;
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;

/// Input to a single inference dispatch.
///
/// `Text` is the v0.1 surface. `Multimodal` is **reserved for future
/// cloud / local multimodal backends** — the OSS `LlamaInferenceBackend`
/// returns `InferenceError::Unsupported` on this variant.
///
/// The `content_refs` on `Multimodal` are BLAKE3-keyed pointers into the
/// `kx-content` store; future backends fetch the bytes and feed them
/// alongside the text to a vision-capable model.
///
/// # Examples
///
/// ```
/// use kx_inference::InferenceInput;
///
/// let prompt = InferenceInput::text("Hello, world!");
/// match prompt {
///     InferenceInput::Text(s) => assert_eq!(s, "Hello, world!"),
///     InferenceInput::Multimodal { .. } => unreachable!(),
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum InferenceInput {
    /// Text-only prompt — the only variant any v0.1 backend handles.
    Text(String),
    /// Multimodal prompt (text + content refs to images / audio / etc).
    /// Reserved; OSS v0.1 backends MUST return `Err(Unsupported)`.
    Multimodal {
        /// Text portion of the prompt (the multimodal model still
        /// consumes a textual instruction alongside the content refs).
        text: String,
        /// BLAKE3-keyed pointers to binary content (images / audio /
        /// other modalities) the multimodal backend will fetch from
        /// `kx-content`.
        content_refs: SmallVec<[ContentRef; 4]>,
    },
}

impl InferenceInput {
    /// Length of the textual portion of the input, in bytes. Used by the
    /// dispatcher when checking against
    /// `warrant.model_route.max_input_tokens` (the runtime treats the
    /// limit as bytes — token counting is the backend's job; the byte
    /// limit is a coarse upper bound that catches obvious overruns).
    #[must_use]
    pub fn text_len(&self) -> usize {
        match self {
            Self::Text(s) => s.len(),
            Self::Multimodal { text, .. } => text.len(),
        }
    }

    /// Construct a `Text` input from any string-like value.
    pub fn text(s: impl Into<String>) -> Self {
        Self::Text(s.into())
    }
}

/// Reserved opaque grammar specification for constrained generation.
///
/// Future backends interpret the payload (GBNF for llama.cpp, JSON
/// schema for cloud APIs). OSS v0.1 backends MUST return
/// `Err(Unsupported)` if `InferenceParams.grammar` is `Some(_)`.
///
/// # Examples
///
/// ```
/// use kx_inference::Grammar;
///
/// let g = Grammar::new(r#"{"type":"object"}"#);
/// assert_eq!(g.raw, r#"{"type":"object"}"#);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grammar {
    /// Opaque serialized grammar payload. Encoding chosen by the
    /// backend that will consume it (GBNF / JSON schema / etc).
    pub raw: String,
}

impl Grammar {
    /// Construct a grammar from any string-like value. The payload is
    /// opaque to this crate; backends interpret.
    pub fn new(raw: impl Into<String>) -> Self {
        Self { raw: raw.into() }
    }
}

/// Per-dispatch generation parameters.
///
/// Defaults are **greedy + deterministic** because the runtime's
/// content-addressed identity relies on inference outputs being
/// reproducible across attempts. If a workflow author wants stochastic
/// sampling, they widen these explicitly — at the cost of attempt
/// reproducibility on retry.
///
/// # Examples
///
/// ```
/// use kx_inference::InferenceParams;
///
/// // Defaults are greedy (temperature_bps == 0); grammar is reserved
/// // for future PRs and defaults to None.
/// let p = InferenceParams::default();
/// assert_eq!(p.temperature_bps, 0);
/// assert!(p.grammar.is_none());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InferenceParams {
    /// Maximum output tokens. Derived from
    /// `warrant.model_route.max_output_tokens` when constructed via
    /// `from_warrant`.
    pub max_output_tokens: u32,

    /// Sampling temperature in basis points (×10 000). Defaults to 0 —
    /// greedy decoding, deterministic. Integer-encoded so this struct
    /// is `Eq`-comparable and serializes to canonical bytes for
    /// idempotency-key derivation.
    pub temperature_bps: u32,

    /// Top-p (nucleus) in basis points. Defaults to 10 000 (= 1.0); only
    /// active when `temperature_bps > 0`.
    pub top_p_bps: u32,

    /// Top-k. 0 disables top-k. Active only when `temperature_bps > 0`.
    pub top_k: u32,

    /// Sampler seed. Locked at construction; same seed + same params +
    /// same model = bit-identical output even when temperature > 0.
    pub seed: u32,

    /// Stop tokens / stop sequences. Empty = no stop.
    pub stop_tokens: SmallVec<[String; 4]>,

    /// **Reserved**: opaque grammar specification for constrained
    /// generation (GBNF / JSON-schema). OSS v0.1 backends MUST return
    /// `Err(Unsupported)` on `Some(_)`. The field exists ahead of
    /// implementation so the trait stays additive when grammar support
    /// lands.
    pub grammar: Option<Grammar>,
}

impl Default for InferenceParams {
    fn default() -> Self {
        Self {
            max_output_tokens: 512,
            temperature_bps: 0,
            top_p_bps: 10_000,
            top_k: 0,
            seed: 0,
            stop_tokens: SmallVec::new(),
            grammar: None,
        }
    }
}

impl InferenceParams {
    /// Construct params with limits derived from the warrant.
    ///
    /// The warrant's `max_output_tokens` is the ceiling; downstream
    /// authors can tighten further but not widen (the dispatcher
    /// enforces this).
    #[must_use]
    pub fn from_warrant(warrant: &kx_warrant::WarrantSpec) -> Self {
        Self {
            max_output_tokens: warrant.model_route.max_output_tokens,
            ..Self::default()
        }
    }

    /// Check whether requested params stay within the warrant's
    /// quantitative limits. Returns `Err(ScopeViolation)` on widen.
    pub(crate) fn check_within(
        &self,
        warrant: &kx_warrant::WarrantSpec,
    ) -> Result<(), InferenceError> {
        if self.max_output_tokens > warrant.model_route.max_output_tokens {
            return Err(InferenceError::ScopeViolation {
                field: "max_output_tokens",
                requested: u64::from(self.max_output_tokens),
                ceiling: u64::from(warrant.model_route.max_output_tokens),
            });
        }
        Ok(())
    }
}

/// Result of a single inference dispatch.
///
/// `bytes` is the canonical serialization of the generated content
/// (UTF-8 for text models). The executor stages it through
/// `kx-content::ContentStore` and the resulting `ContentRef` is what
/// the journal records as `Committed.result_ref`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InferenceOutput {
    /// Generated bytes (text models: UTF-8 of the completion).
    pub bytes: Vec<u8>,

    /// Number of output tokens actually emitted by the backend. Used
    /// for cost accounting (cloud backends) + idempotency-key sanity
    /// checks. Approximate for local llama.cpp (token count maps 1:1
    /// to the tokens generated; reported as-is).
    pub output_tokens: u32,

    /// Backend identity that produced this output (e.g., `"kx-llamacpp"`
    /// or `"kx-cloud-vllm"`). Audit-trail field.
    pub backend_name: &'static str,

    /// The model id that produced this output. Echoed back so callers
    /// can confirm the dispatcher routed to the expected backend.
    pub model_id: ModelId,

    /// Wall-clock time spent inside the backend's dispatch. Excludes
    /// dispatcher overhead (validator / memoizer / scope checks).
    pub elapsed: Duration,
}

/// Failure modes surfaced by the dispatcher or any backend.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InferenceError {
    /// The feature is reserved but not implemented in this backend.
    ///
    /// Returned by OSS v0.1 backends on:
    ///   - `InferenceInput::Multimodal` (multimodal reserved for future PRs)
    ///   - `InferenceParams.grammar = Some(_)` (constrained generation reserved)
    ///
    /// **Not** a bug — a deliberate seam-shape decision documented in
    /// the PR 8 plan + the private-corpus HANDOFF §3.8.
    #[error("unsupported in this backend: {reason}")]
    Unsupported {
        /// Static reason string describing what's reserved.
        reason: &'static str,
    },

    /// The named `model_id` is not recognised by any registered backend.
    #[error("model not found: {model_id}")]
    ModelNotFound {
        /// The model id that was requested but had no backend / registry entry.
        model_id: String,
    },

    /// The named `model_id` does not match the warrant's `model_route`.
    /// Per D35, the dispatcher refuses to call a model the warrant did
    /// not authorise — even if the backend would serve it.
    #[error("warrant denies model: {model_id} (warrant route: {route})")]
    WarrantDeniesModel {
        /// The model id the caller tried to dispatch.
        model_id: String,
        /// The model id the warrant authorises.
        route: String,
    },

    /// Generic warrant-narrowing violation on a quantitative field
    /// (e.g., `max_output_tokens > ceiling`).
    #[error("scope violation on {field}: requested {requested}, ceiling {ceiling}")]
    ScopeViolation {
        /// The warrant field that was violated.
        field: &'static str,
        /// The requested value.
        requested: u64,
        /// The warrant's ceiling for this field.
        ceiling: u64,
    },

    /// The bind-time model validator (D29) emitted `TypeError`.
    #[error("model failed bind-time validation: {message}")]
    ModelValidation {
        /// Human-readable validation failure description.
        message: String,
    },

    /// `wall_clock_ms` elapsed before the backend returned.
    #[error("wall_clock timeout: {wall_clock_ms} ms")]
    Timeout {
        /// The wall-clock ceiling, in milliseconds.
        wall_clock_ms: u64,
    },

    /// The backend returned an internal failure (load failed, decode
    /// failed, sampler init failed, etc).
    #[error("backend `{backend}` failed: {message}")]
    BackendFailure {
        /// Identifier of the backend that failed.
        backend: &'static str,
        /// Backend-specific failure detail.
        message: String,
    },

    /// The dispatcher could not resolve a `ContentRef` from the content
    /// store during input assembly.
    #[error("content store miss: {content_ref:?}")]
    ContentStoreMiss {
        /// The content reference that could not be resolved.
        content_ref: ContentRef,
    },
}
