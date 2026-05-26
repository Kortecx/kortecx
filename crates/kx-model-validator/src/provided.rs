//! [`ProvidedCapabilities`] — what a model exposes, populated by the model
//! registry from declared (or, in v2, verified) metadata. Carries the
//! [`Soundness`] tag that marks the v1 → v2 boundary.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::capabilities::{License, Modality, Quantization};

/// What a model exposes — populated by the model registry from declared
/// metadata.
///
/// Per D29: v1 trusts these declarations. The capability broker (P1.8.5) is
/// the runtime backstop that catches false declarations at execution. v2
/// (deferred) will add a capability probe to verify each declaration at
/// model-load time and cache the result.
///
/// # Examples
///
/// ```
/// use kx_model_validator::{License, Modality, ProvidedCapabilities, Quantization};
/// use std::collections::BTreeSet;
///
/// // What a Llama-3-8B-Instruct GGUF might expose.
/// let provided = ProvidedCapabilities::declared()
///     .with_context_window_tokens(8_192)
///     .with_native_tool_calling(true)
///     .with_modalities(BTreeSet::from([Modality::Text]))
///     .with_quantization(Quantization::Q4KM)
///     .with_chat_template(Some("llama-3-instruct".into()))
///     .with_license(License::LlamaCommunity);
/// assert_eq!(provided.context_window_tokens, 8_192);
/// assert!(provided.native_tool_calling);
/// ```
// `Eq` / `Hash` deliberately NOT derived: `eval_scores` contains `f32`
// values which only implement `PartialEq` / `PartialOrd`. The validator
// compares structurally via `check`; capability sets are never used as
// HashMap keys.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProvidedCapabilities {
    /// Context window in tokens (the loaded model's `n_ctx_train` or
    /// configured value).
    pub context_window_tokens: u32,

    /// `true` if the model has native tool/function-calling (vs. prompted
    /// emulation).
    pub native_tool_calling: bool,

    /// Input/output modalities the model supports.
    pub modalities: BTreeSet<Modality>,

    /// The loaded quantization format (single value — the model is loaded in
    /// exactly one format).
    pub quantization: Quantization,

    /// The model's chat-template name if present (e.g., `"llama-3-instruct"`,
    /// `"chatml"`), `None` if absent (raw completion only).
    pub chat_template: Option<String>,

    /// The model's distribution license.
    pub license: License,

    /// Soundness tag: v1 records `Declared`; v2 will record `Verified` after
    /// a capability probe. Recommender ranking can prefer verified providers
    /// (future).
    pub soundness: Soundness,

    /// Opt-in published eval scores keyed by eval name (e.g.,
    /// `"mmlu" → 0.823`). The workflow author names which eval to rank by;
    /// the recommender never picks one on its own (prevents flattering the
    /// house model).
    pub eval_scores: BTreeMap<String, f32>,
}

/// Soundness tag for [`ProvidedCapabilities`] (D29 v1 → v2 boundary).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum Soundness {
    /// The capabilities come from a model-supplied declaration (manifest,
    /// metadata file). v1's default — trust assumed.
    Declared = 0,
    /// The capabilities were verified at model-load time by a deterministic
    /// probe (v2 — deferred). The result is cached.
    Verified = 1,
}

impl ProvidedCapabilities {
    /// Start a `ProvidedCapabilities` builder in `Declared` soundness mode
    /// (v1 default).
    ///
    /// Returns a sensible baseline (text-only, F32, no tool calling, no chat
    /// template, Unknown license) that the caller refines via `with_*`
    /// chaining. Avoids forcing every call site to specify every field.
    #[must_use]
    pub fn declared() -> Self {
        let mut modalities = BTreeSet::new();
        modalities.insert(Modality::Text);
        Self {
            context_window_tokens: 0,
            native_tool_calling: false,
            modalities,
            quantization: Quantization::F32,
            chat_template: None,
            license: License::Unknown,
            soundness: Soundness::Declared,
            eval_scores: BTreeMap::new(),
        }
    }

    /// Same as [`Self::declared`] but stamped `Verified` (for v2 / probe-
    /// validated callers when that path exists).
    #[must_use]
    pub fn verified() -> Self {
        let mut p = Self::declared();
        p.soundness = Soundness::Verified;
        p
    }

    // ---- Builder methods (composable; chainable) -------------------------

    /// Set the context window in tokens.
    pub fn with_context_window_tokens(mut self, n: u32) -> Self {
        self.context_window_tokens = n;
        self
    }

    /// Set the native-tool-calling flag.
    pub fn with_native_tool_calling(mut self, on: bool) -> Self {
        self.native_tool_calling = on;
        self
    }

    /// Replace the modalities set.
    pub fn with_modalities(mut self, m: BTreeSet<Modality>) -> Self {
        self.modalities = m;
        self
    }

    /// Set the quantization format.
    pub fn with_quantization(mut self, q: Quantization) -> Self {
        self.quantization = q;
        self
    }

    /// Set the chat template name (or `None` for absent).
    pub fn with_chat_template(mut self, t: Option<String>) -> Self {
        self.chat_template = t;
        self
    }

    /// Set the license.
    pub fn with_license(mut self, l: License) -> Self {
        self.license = l;
        self
    }

    /// Add or replace one eval score.
    pub fn with_eval_score(mut self, name: impl Into<String>, score: f32) -> Self {
        self.eval_scores.insert(name.into(), score);
        self
    }
}
