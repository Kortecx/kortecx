// Public types crossing the dispatcher / backend seam.
//
// Two forward-compat hooks live here per the PR 8 strategy plan:
//   - `InferenceInput::Multimodal` reserved variant (no impl in v0.1).
//   - `InferenceParams.grammar` reserved Option field (no impl in v0.1).
//
// Both return `InferenceError::Unsupported` when used against the OSS
// `LlamaInferenceBackend` — the seam is exercised by tests so the
// `Err(Unsupported)` path is documented, not merely declarative.
//
// `InferenceParams` and `Grammar` live in `kx-mote` after D50
// (citation-admissibility freeze §2.51) because `MoteDef.inference_params`
// makes them identity-bearing. Re-exported here for API stability.

use std::time::Duration;

use kx_content::ContentRef;
pub use kx_mote::{Grammar, InferenceParams};
use kx_mote::{ModelId, Mote};
use kx_warrant::WarrantSpec;
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

/// Construct an [`InferenceParams`] from a [`Mote`] and its [`WarrantSpec`].
///
/// D50 (citation-admissibility freeze §2.51) makes `mote.def.inference_params`
/// the identity-bearing source for decoding parameters.
///
/// **This is the SOLE permitted constructor of dispatch-bound
/// `InferenceParams` in the OSS runtime.** Any future code path that
/// produces an `InferenceParams` value the dispatcher hands to a backend
/// MUST route through this function. The structural invariant — only
/// `from_mote` materialises dispatch-bound params, sourcing every
/// decoding field from `mote.def.inference_params` (which is identity-
/// bearing via `MoteDef::hash`) — is what guarantees memoizer
/// correctness post-D50. No runtime tripwire is shipped; reviewers of
/// any PR that introduces a second constructor MUST explain why the
/// substitute path preserves the same source-of-truth.
///
/// Returns the decoding fields verbatim from `mote.def.inference_params`
/// and refuses with [`InferenceError::ScopeViolation`] if the mote
/// declares a `max_output_tokens` above the warrant's ceiling.
///
/// # Errors
///
/// - [`InferenceError::ScopeViolation`] when
///   `mote.def.inference_params.max_output_tokens > warrant.model_route.max_output_tokens`.
pub fn inference_params_from_mote(
    mote: &Mote,
    warrant: &WarrantSpec,
) -> Result<InferenceParams, InferenceError> {
    let declared = &mote.def.inference_params;
    if declared.max_output_tokens > warrant.model_route.max_output_tokens {
        return Err(InferenceError::ScopeViolation {
            field: "max_output_tokens",
            requested: u64::from(declared.max_output_tokens),
            ceiling: u64::from(warrant.model_route.max_output_tokens),
        });
    }
    Ok(declared.clone())
}

/// Check that `params.max_output_tokens` does not exceed the warrant's
/// `model_route.max_output_tokens` ceiling. Returns
/// [`InferenceError::ScopeViolation`] on widen.
///
/// Backends call this from their `dispatch` impl to enforce D30's
/// monotonic-narrowing rule on a quantitative axis.
pub(crate) fn check_within(
    params: &InferenceParams,
    warrant: &WarrantSpec,
) -> Result<(), InferenceError> {
    if params.max_output_tokens > warrant.model_route.max_output_tokens {
        return Err(InferenceError::ScopeViolation {
            field: "max_output_tokens",
            requested: u64::from(params.max_output_tokens),
            ceiling: u64::from(warrant.model_route.max_output_tokens),
        });
    }
    Ok(())
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
