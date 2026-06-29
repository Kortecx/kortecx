//! [`crate::InferenceParams`] — per-dispatch generation parameters; identity-bearing
//! after D50 since [`crate::MoteDef`] now carries an `inference_params` field
//! that participates in `mote_def_hash`.
//!
//! These types live here (not in `kx-inference`) because they're part of the
//! identity substrate per D4 — two MoteDefs differing only in decoding params
//! MUST produce different `mote_def_hash`/`MoteId`, and the cleanest way to
//! make that structural is to put the type next to the rest of the
//! identity-bearing `MoteDef` shape. The dispatcher-side bridge fns
//! (`inference_params_from_mote`, `check_within`) stay in `kx-inference`
//! since they reference `WarrantSpec`.
//!
//! Constrained-generation hooks (reserved in PR 8, IMPLEMENTED in RC2):
//!   - [`crate::Grammar`] — opaque constrained-generation payload (RC2 carries a
//!     serialized `kx_grammar::ToolEnvelopeSpec`; the type stays opaque here).
//!   - [`crate::InferenceParams::grammar`] — `Option` field, default `None`. When
//!     `Some`, the OSS backends HONOR it (GBNF on llama.cpp, JSON-mode on Ollama)
//!     to constrain tool-calling. Identity-bearing (D50) but, in the live serve
//!     loop, populated OFF-MoteDef at dispatch from the warrant's granted tools
//!     (off-digest, D108.2) — see `kx-gateway`'s `model_exec`.

use serde::{Deserialize, Serialize};
use smallvec::SmallVec;

/// Opaque grammar specification for constrained generation.
///
/// The payload is interpreted by the backend (RC2: a serialized
/// `kx_grammar::ToolEnvelopeSpec` the in-process llama.cpp backend renders to
/// GBNF and the Ollama backend renders to a JSON Schema). The type stays opaque
/// here so the identity substrate carries no engine coupling. A backend that
/// cannot honor a `Some(_)` grammar returns `Err(Unsupported)`.
///
/// # Examples
///
/// ```
/// use kx_mote::Grammar;
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
/// **D50 identity invariant**: these fields participate in
/// `MoteDef::hash` because [`crate::MoteDef`] carries an
/// `inference_params: InferenceParams` field. Two MoteDefs differing
/// only in `temperature_bps` (or any other decoding field) produce
/// DIFFERENT `mote_def_hash` and DIFFERENT `MoteId`. The dispatcher's
/// bridge fn `inference_params_from_mote` (in `kx-inference`) is the
/// only blessed path to construct dispatch-bound params; the structural
/// invariant — sole constructor sourcing every field from the identity
/// substrate — is what guarantees memoizer correctness.
///
/// # Examples
///
/// ```
/// use kx_mote::InferenceParams;
///
/// // Defaults are greedy (temperature_bps == 0); grammar defaults to None
/// // (set per-dispatch from the granted tools to constrain tool-calling).
/// let p = InferenceParams::default();
/// assert_eq!(p.temperature_bps, 0);
/// assert!(p.grammar.is_none());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InferenceParams {
    /// Maximum output tokens. Workflow-declared ceiling; at dispatch
    /// time the warrant's `model_route.max_output_tokens` is the
    /// binding ceiling and `inference_params_from_mote` returns
    /// `ScopeViolation` if this field exceeds it.
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

    /// Opaque grammar specification for constrained generation (GBNF /
    /// JSON-schema). Default `None` (unconstrained). When `Some(_)`, the OSS
    /// backends constrain decoding to it (RC2); a backend that cannot returns
    /// `Err(Unsupported)`. Identity-bearing (D50) — but the live serve loop sets
    /// it OFF-MoteDef at dispatch (off-digest, D108.2), so the canonical demo
    /// (which grants no tools) keeps `grammar: None` and the digest is invariant.
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
