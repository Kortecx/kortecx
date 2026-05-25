#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::doc_markdown,
    clippy::return_self_not_must_use,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_lossless,
    clippy::match_same_arms
)]
// Inline test modules are exempted from the workspace deny on `unwrap_used` /
// `expect_used`. Integration tests under tests/*.rs carry per-file allows.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! # kx-model-validator — static bind-time fitness type check
//!
//! Model fitness as a **type check over capability sets**.
//!
//! - A task declares a [`RequiredCapabilities`] — the type signature it expects.
//! - A model exposes a [`ProvidedCapabilities`] — its actual type.
//! - [`check`] performs structural subtyping: the model is a valid binding iff
//!   its provided capabilities satisfy (are a superset of) the task's required
//!   ones.
//!
//! Runs at **bind time** — when a model is loaded, or before a Mote needing
//! that model is scheduled. It is a **static** check, never a mid-execution
//! discovery. The whole value: find the wrong model at load time, not three
//! Motes into a workflow.
//!
//! ## Three outcomes ([`ValidatorOutcome`])
//!
//! - [`ValidatorOutcome::TypeOk`] — `provided ⊇ required` — clean bind.
//! - [`ValidatorOutcome::DegradedSubtype`] — binds, but an optional/soft
//!   capability is missing or emulated (e.g. tool-calling done via prompting
//!   instead of native). Records the degraded mode for downstream callers
//!   (the formatter adapter compensates).
//! - [`ValidatorOutcome::TypeError`] — a REQUIRED capability is missing —
//!   refuses to bind, names the missing member and why.
//!
//! ## Hard boundary: interface, NOT quality
//!
//! The validator type-checks the model's **interface (signature)**, never its
//! output **quality (behavior)**. Capability is statically checkable; quality
//! is not — that's the workflow / eval / critic layer's job (orchestration vs
//! semantic correctness).
//!
//! The validator's smaller, honest claim ("I prove your model HAS the required
//! capabilities; I do not claim it'll be GOOD at them") is the source of its
//! credibility. Do not let it drift into a quality oracle.
//!
//! ## Soundness boundary: v1 → v2
//!
//! - **v1 (this crate)** — the validator type-checks against the model's
//!   **declared** [`ProvidedCapabilities`]. Declarations can lie. v1's guarantee
//!   is *"the model CLAIMS to satisfy the signature."* All v1 language uses
//!   "declared." `TypeOk` is more precisely "TypeOk-as-declared."
//! - **v2 (deferred)** — a capability probe will verify each declaration via a
//!   one-time deterministic test, result cached. v2 language will use "verified"
//!   and "guaranteed."
//! - In v1, the capability broker (P1.8.5) remains the runtime backstop that
//!   catches false declarations at execution.
//!
//! ## House model competes on equal merit
//!
//! The validator does **not** know which model is the house model. There is
//! no field, no flag, no boost. The type-theoretic framing makes the
//! no-favoritism property structural rather than aspirational. See
//! [`Recommender`] for the ranking discipline.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

pub use kx_mote::ModelId;

// ===========================================================================
// Capability primitives (small enums; stable u8/string discriminants for
// future on-disk encoding when capabilities land in a model registry).
// ===========================================================================

/// Input/output modalities a model supports.
///
/// Closed enum; new modalities require a coordinated update to the registry
/// schema. The discriminants are stable for forward-compat metadata storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum Modality {
    /// Text (always present; the baseline modality).
    Text = 0,
    /// Vision input (images, frames).
    Vision = 1,
    /// Audio input (speech, music).
    Audio = 2,
    /// Generic embedding output (the model emits a fixed-dim vector rather
    /// than tokens).
    Embedding = 3,
}

/// Quantization format the model is loaded in.
///
/// Closed enum for the formats kortecx's OSS inference path (llama.cpp via
/// `kx-llamacpp`) actually loads. Cloud backends with different format
/// support (e.g., vLLM with safetensors) plug their own [`Quantization`]
/// extensions in via the registry's free-form metadata, not here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum Quantization {
    /// Full 32-bit floats (no quantization).
    F32 = 0,
    /// Half-precision floats.
    F16 = 1,
    /// BFloat16.
    Bf16 = 2,
    /// 8-bit integer (gguf `Q8_0`, common for accuracy-sensitive deployments).
    Q8_0 = 10,
    /// 5-bit, K-quant medium.
    Q5KM = 20,
    /// 4-bit, K-quant medium (the sweet-spot default for many local
    /// deployments).
    Q4KM = 30,
    /// 4-bit, `Q4_0` (gguf legacy/portable).
    Q4_0 = 31,
    /// 2-bit, K-quant (memory-constrained deployments; large quality loss).
    Q2K = 40,
}

/// License under which a model is distributed.
///
/// Tagged as either an SPDX identifier (`SpdxId("Apache-2.0")`) or one of the
/// common open-but-restrictive license patterns. Workflow authors declare
/// constraints via [`LicenseConstraint`]; the check is an exact-match or
/// inclusion lookup, NOT a free-form SPDX expression parser.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum License {
    /// An SPDX identifier (e.g., `"Apache-2.0"`, `"MIT"`).
    SpdxId(String),
    /// Llama-style community license (commercial use permitted with
    /// per-account caps; common for Meta models).
    LlamaCommunity,
    /// Open-weights-non-commercial (research use only; e.g., some early
    /// Llama variants).
    OpenWeightsNonCommercial,
    /// Proprietary / closed (e.g., GPT-class API access; the runtime can
    /// call but not redistribute weights).
    Proprietary,
    /// License is unknown to the registry. Treated as restrictive by default
    /// (matches no constraint that requires a specific license).
    Unknown,
}

/// Workflow-author constraint on the model's license.
///
/// Constraints compose by intersection: declaring `RequireCommercialOk` AND
/// `RequireRedistributable` requires a license satisfying both.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum LicenseConstraint {
    /// No license restriction (any model permitted).
    NoRestriction,
    /// The license must explicitly allow commercial use.
    RequireCommercialOk,
    /// The license must allow weight redistribution (e.g., shipping the
    /// model file in a container image).
    RequireRedistributable,
    /// The model's license must be one of an explicit set (e.g.,
    /// `{"Apache-2.0", "MIT"}`).
    OneOf(BTreeSet<License>),
}

impl LicenseConstraint {
    /// `true` when `license` satisfies the constraint.
    #[must_use]
    pub fn is_satisfied_by(&self, license: &License) -> bool {
        match self {
            Self::NoRestriction => true,
            Self::RequireCommercialOk => commercial_use_permitted(license),
            Self::RequireRedistributable => redistribution_permitted(license),
            Self::OneOf(allowed) => allowed.contains(license),
        }
    }
}

/// Heuristic: does this license permit commercial use?
///
/// Conservative — only returns `true` for licenses we KNOW permit commercial
/// use. Unknown licenses are treated as restrictive.
fn commercial_use_permitted(license: &License) -> bool {
    match license {
        License::SpdxId(id) => matches!(
            id.as_str(),
            "Apache-2.0" | "MIT" | "BSD-2-Clause" | "BSD-3-Clause" | "ISC"
        ),
        License::LlamaCommunity => true, // commercial use permitted with caveats
        License::Proprietary => true,    // commercial API access IS commercial use
        License::OpenWeightsNonCommercial | License::Unknown => false,
    }
}

/// Heuristic: does this license permit redistribution of the model weights?
fn redistribution_permitted(license: &License) -> bool {
    match license {
        License::SpdxId(id) => matches!(
            id.as_str(),
            "Apache-2.0" | "MIT" | "BSD-2-Clause" | "BSD-3-Clause" | "ISC"
        ),
        License::LlamaCommunity => true, // permits redistribution under the community license
        License::Proprietary | License::OpenWeightsNonCommercial | License::Unknown => false,
    }
}

// ===========================================================================
// Capability sets
// ===========================================================================

/// What a task requires from any model it will be bound to.
///
/// Workflow-author-supplied at workflow-compile time. The runtime never
/// invents one.
///
/// Per D29's "no MoteDef field for v1" decision, this lives on the SDK side
/// as a separate input to [`check`], NOT on the Mote's identity hash. Adding
/// it to `MoteDef` would force a `schema_version` bump (v3 → v4); the v1
/// validator delivers its value without that disturbance. Re-evaluate at P4.
///
/// # Examples
///
/// ```
/// use kx_model_validator::{LicenseConstraint, Modality, Quantization, RequiredCapabilities};
/// use std::collections::BTreeSet;
///
/// // A reasonable default for a long-context tool-calling chat workflow.
/// let req = RequiredCapabilities {
///     min_context_window_tokens: 8_192,
///     requires_native_tool_calling: true,
///     prefers_native_tool_calling: true,
///     required_modalities: BTreeSet::from([Modality::Text]),
///     allowed_quantizations: BTreeSet::from([
///         Quantization::F16,
///         Quantization::Bf16,
///         Quantization::Q8_0,
///         Quantization::Q4KM,
///     ]),
///     requires_chat_template: true,
///     license_constraint: LicenseConstraint::RequireCommercialOk,
/// };
/// assert_eq!(req.min_context_window_tokens, 8_192);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RequiredCapabilities {
    /// Minimum context window the task needs (in tokens). Model's
    /// `context_window_tokens` must be ≥ this.
    pub min_context_window_tokens: u32,

    /// `true` if the workflow REQUIRES native tool-calling — TypeError if the
    /// model lacks it.
    pub requires_native_tool_calling: bool,

    /// `true` if the workflow PREFERS native tool-calling but can tolerate a
    /// degraded mode (the formatter adapter compensates). If both
    /// `requires_native_tool_calling` and `prefers_native_tool_calling` are
    /// `false`, native tool calling is not consulted.
    pub prefers_native_tool_calling: bool,

    /// Modalities the task needs. The model's `modalities` must be a superset.
    /// Empty set ≡ Text-only (the implicit baseline).
    pub required_modalities: BTreeSet<Modality>,

    /// Allowed quantization formats. The model's `quantization` must be in
    /// this set. Empty set ≡ any quantization permitted.
    pub allowed_quantizations: BTreeSet<Quantization>,

    /// `true` if the task needs a chat template present on the model (multi-
    /// turn / role-formatted prompting).
    pub requires_chat_template: bool,

    /// License constraint the model must satisfy.
    pub license_constraint: LicenseConstraint,
}

impl RequiredCapabilities {
    /// A constraint set that admits any model that exists (no requirements).
    ///
    /// Useful for tests, debugging, and workflows that don't care about
    /// fitness (the runtime will still refuse non-existent models — this just
    /// means the validator is permissive for any-existing model).
    #[must_use]
    pub fn permissive() -> Self {
        Self {
            min_context_window_tokens: 0,
            requires_native_tool_calling: false,
            prefers_native_tool_calling: false,
            required_modalities: BTreeSet::new(),
            allowed_quantizations: BTreeSet::new(),
            requires_chat_template: false,
            license_constraint: LicenseConstraint::NoRestriction,
        }
    }
}

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

// ===========================================================================
// ValidatorOutcome — the type-check result
// ===========================================================================

/// Why a capability binds in degraded mode.
///
/// Distinct from [`MissingCapability`] (which causes TypeError refusal). A
/// `DegradationReason` is information for the downstream caller — the
/// formatter adapter, the SDK author, the observability layer.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DegradationReason {
    /// `prefers_native_tool_calling` was set but `provided.native_tool_calling`
    /// is false. Tool calls will need prompted emulation; downstream code
    /// (the formatter adapter) should compensate.
    NativeToolCallingMissing,
}

/// What's missing when the check returns TypeError.
///
/// Named explicitly so the executor can surface a typed reason at refusal
/// time (Mote enters Failed with a specific cause).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MissingCapability {
    /// `provided.context_window_tokens < required.min_context_window_tokens`.
    ContextWindowTooSmall {
        /// What the model provides.
        provided: u32,
        /// What the task needs.
        required: u32,
    },
    /// `required.requires_native_tool_calling` set but
    /// `provided.native_tool_calling = false`.
    NativeToolCallingRequired,
    /// One or more required modalities are not in the model's modality set.
    ModalityMissing {
        /// The modality the task requires but the model lacks.
        missing: Modality,
    },
    /// The model's `quantization` is not in the task's `allowed_quantizations`.
    QuantizationNotAllowed {
        /// The model's quantization.
        provided: Quantization,
    },
    /// `requires_chat_template` set but `provided.chat_template` is `None`.
    ChatTemplateRequired,
    /// The model's license does not satisfy the task's constraint.
    LicenseUnsatisfied {
        /// The model's license.
        provided: License,
    },
}

/// Outcome of a [`check`] call.
///
/// Three variants:
/// - [`Self::TypeOk`] — `provided ⊇ required`. Clean bind.
/// - [`Self::DegradedSubtype`] — required ⊆ provided AND at least one
///   *preferred* (soft) capability is missing or emulated.
/// - [`Self::TypeError`] — at least one *required* capability is absent.
///   Refuse the bind.
///
/// # Examples
///
/// ```
/// use kx_model_validator::{check, ProvidedCapabilities, Quantization, RequiredCapabilities, ValidatorOutcome};
///
/// // Permissive task + any model = TypeOk.
/// let req = RequiredCapabilities::permissive();
/// let provided = ProvidedCapabilities::declared()
///     .with_context_window_tokens(4096)
///     .with_quantization(Quantization::Q4KM);
/// assert_eq!(check(&provided, &req), ValidatorOutcome::TypeOk);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ValidatorOutcome {
    /// Provided capabilities satisfy required + all preferences. Clean bind.
    TypeOk,
    /// Provided capabilities satisfy required, but a preferred (soft)
    /// capability is missing. The bind proceeds; the caller records the
    /// degraded mode (e.g., the formatter adapter compensates).
    DegradedSubtype {
        /// The reasons the bind is degraded (non-empty).
        reasons: Vec<DegradationReason>,
    },
    /// One or more required capabilities are missing. The bind is refused.
    /// The executor surfaces a typed error naming the missing members.
    TypeError {
        /// The capabilities that were required but absent (non-empty).
        missing: Vec<MissingCapability>,
    },
}

impl ValidatorOutcome {
    /// `true` if the bind would proceed (TypeOk or DegradedSubtype).
    #[must_use]
    pub fn is_acceptable(&self) -> bool {
        matches!(self, Self::TypeOk | Self::DegradedSubtype { .. })
    }

    /// `true` if the bind is refused.
    #[must_use]
    pub fn is_type_error(&self) -> bool {
        matches!(self, Self::TypeError { .. })
    }
}

// ===========================================================================
// The check function — pure, total, deterministic
// ===========================================================================

/// Perform the bind-time fitness type check.
///
/// **Pure**: no I/O, no model invocation, no journal access.
/// **Total**: every `(provided, required)` pair returns a `ValidatorOutcome`.
/// **Deterministic**: identical inputs yield identical outputs across calls
/// and threads.
///
/// Returns:
/// - [`ValidatorOutcome::TypeError`] if any REQUIRED capability is missing
///   (the missing list is non-empty in PR order: context window → tool
///   calling → modality → quantization → chat template → license).
/// - [`ValidatorOutcome::DegradedSubtype`] if no required capability is
///   missing but a PREFERRED capability is missing (e.g.,
///   `prefers_native_tool_calling` set and provider lacks native tool
///   calling).
/// - [`ValidatorOutcome::TypeOk`] otherwise.
#[tracing::instrument(level = "debug", skip(provided, required))]
#[must_use]
pub fn check(provided: &ProvidedCapabilities, required: &RequiredCapabilities) -> ValidatorOutcome {
    let mut missing: Vec<MissingCapability> = Vec::new();

    // 1. Context window.
    if provided.context_window_tokens < required.min_context_window_tokens {
        missing.push(MissingCapability::ContextWindowTooSmall {
            provided: provided.context_window_tokens,
            required: required.min_context_window_tokens,
        });
    }

    // 2. Native tool calling (required form).
    if required.requires_native_tool_calling && !provided.native_tool_calling {
        missing.push(MissingCapability::NativeToolCallingRequired);
    }

    // 3. Modalities.
    for m in &required.required_modalities {
        if !provided.modalities.contains(m) {
            missing.push(MissingCapability::ModalityMissing { missing: *m });
        }
    }

    // 4. Quantization.
    if !required.allowed_quantizations.is_empty()
        && !required
            .allowed_quantizations
            .contains(&provided.quantization)
    {
        missing.push(MissingCapability::QuantizationNotAllowed {
            provided: provided.quantization,
        });
    }

    // 5. Chat template.
    if required.requires_chat_template && provided.chat_template.is_none() {
        missing.push(MissingCapability::ChatTemplateRequired);
    }

    // 6. License.
    if !required
        .license_constraint
        .is_satisfied_by(&provided.license)
    {
        missing.push(MissingCapability::LicenseUnsatisfied {
            provided: provided.license.clone(),
        });
    }

    if !missing.is_empty() {
        return ValidatorOutcome::TypeError { missing };
    }

    // No required missing — check preferences for degraded mode.
    let mut reasons: Vec<DegradationReason> = Vec::new();
    if required.prefers_native_tool_calling
        && !required.requires_native_tool_calling
        && !provided.native_tool_calling
    {
        reasons.push(DegradationReason::NativeToolCallingMissing);
    }

    if reasons.is_empty() {
        ValidatorOutcome::TypeOk
    } else {
        ValidatorOutcome::DegradedSubtype { reasons }
    }
}

// ===========================================================================
// ModelRegistry trait + in-memory impl
// ===========================================================================

/// A lookup over models the runtime knows about, keyed by [`ModelId`].
///
/// OSS ships [`InMemoryModelRegistry`]; cloud impls (hosted registry, S3-
/// backed manifests) plug in behind the same trait per D28 (cloud-first
/// scale principle).
pub trait ModelRegistry: Send + Sync {
    /// Look up a model's declared (or verified) provided capabilities.
    fn lookup(&self, model_id: &ModelId) -> Option<ProvidedCapabilities>;

    /// Iterate every model in the registry. Used by [`Recommender::candidates`]
    /// to find substitutes when the requested model fails its check.
    fn entries(&self) -> Vec<(ModelId, ProvidedCapabilities)>;
}

/// In-memory `ModelRegistry`. The OSS default; cloud impls bring their own.
///
/// # Examples
///
/// ```
/// use kx_model_validator::{InMemoryModelRegistry, ModelRegistry, ProvidedCapabilities};
/// use kx_mote::ModelId;
///
/// let mut reg = InMemoryModelRegistry::new();
/// reg.insert(
///     ModelId("llama-3-8b-instruct".into()),
///     ProvidedCapabilities::declared().with_context_window_tokens(8_192),
/// );
/// assert!(reg.lookup(&ModelId("llama-3-8b-instruct".into())).is_some());
/// assert!(reg.lookup(&ModelId("unknown".into())).is_none());
/// ```
#[derive(Debug, Default, Clone)]
pub struct InMemoryModelRegistry {
    by_id: BTreeMap<ModelId, ProvidedCapabilities>,
}

impl InMemoryModelRegistry {
    /// Construct an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert (or replace) a model entry.
    pub fn insert(&mut self, id: ModelId, capabilities: ProvidedCapabilities) {
        self.by_id.insert(id, capabilities);
    }

    /// Remove an entry; returns whether it was present.
    pub fn remove(&mut self, id: &ModelId) -> bool {
        self.by_id.remove(id).is_some()
    }

    /// Number of entries in the registry.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    /// `true` when the registry has no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }
}

impl ModelRegistry for InMemoryModelRegistry {
    fn lookup(&self, model_id: &ModelId) -> Option<ProvidedCapabilities> {
        self.by_id.get(model_id).cloned()
    }

    fn entries(&self) -> Vec<(ModelId, ProvidedCapabilities)> {
        self.by_id
            .iter()
            .map(|(id, cap)| (id.clone(), cap.clone()))
            .collect()
    }
}

// ===========================================================================
// Recommender — type inference as a thin layer over `check`
// ===========================================================================

/// Ranking authority used by [`Recommender::candidates`].
///
/// Strict precedence (D29):
/// 1. **Deterministic capability match** (always). Models that fail the
///    check are filtered out; among the remaining, TypeOk outranks
///    DegradedSubtype, and within each tier, models with more soft
///    preferences satisfied outrank fewer.
/// 2. **Workflow-named eval score** (optional). If the caller names an eval,
///    higher scores rank higher within tier 1.
/// 3. **In-runtime Mote-measured performance** (deferred to P1.13+). v1
///    returns a stable order within the tier; v2 will plug a journal-derived
///    score in here.
///
/// **The recommender does NOT know which model is the house model.** No
/// flag, no boost, no field. Type framing makes no-favoritism structural.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RankingPolicy {
    /// If `Some(name)`, the recommender ranks acceptable candidates by
    /// `provided.eval_scores[name]` descending. Caller-chosen, never picked
    /// by the recommender.
    pub eval_metric: Option<String>,
}

/// One candidate result from the recommender.
///
/// Sorted within a `Vec<Candidate>` by the [`RankingPolicy`] precedence.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    /// The candidate's identity.
    pub model_id: ModelId,
    /// The check outcome against the workflow's `required` capabilities.
    pub outcome: ValidatorOutcome,
    /// The candidate's declared capabilities (carried for the caller's
    /// inspection — e.g., showing why one ranked above another).
    pub provided: ProvidedCapabilities,
    /// The eval score under the workflow's `RankingPolicy.eval_metric`, if
    /// present and applicable.
    pub eval_score: Option<f32>,
}

/// Thin layer over [`check`] + a [`ModelRegistry`]. Suggests, never
/// substitutes.
///
/// Same principle as the broker (P1.8.5) never silently rewriting a tool
/// call: the recommender returns ranked candidates; the caller (SDK or
/// runtime router) decides whether to act on the suggestion.
///
/// # Examples
///
/// ```
/// use kx_model_validator::{
///     check, InMemoryModelRegistry, ProvidedCapabilities, Quantization,
///     RankingPolicy, Recommender, RequiredCapabilities, ValidatorOutcome,
/// };
/// use kx_mote::ModelId;
/// use std::collections::BTreeSet;
///
/// let mut reg = InMemoryModelRegistry::new();
/// reg.insert(
///     ModelId("small-text".into()),
///     ProvidedCapabilities::declared().with_context_window_tokens(2_048),
/// );
/// reg.insert(
///     ModelId("big-text".into()),
///     ProvidedCapabilities::declared().with_context_window_tokens(32_768),
/// );
///
/// let req = RequiredCapabilities {
///     min_context_window_tokens: 16_000,
///     ..RequiredCapabilities::permissive()
/// };
///
/// let recommender = Recommender::new(&reg, RankingPolicy::default());
/// let candidates = recommender.candidates(&req);
///
/// // Only big-text satisfies the 16k context requirement.
/// assert_eq!(candidates.len(), 1);
/// assert_eq!(candidates[0].model_id, ModelId("big-text".into()));
/// assert_eq!(candidates[0].outcome, ValidatorOutcome::TypeOk);
/// ```
pub struct Recommender<'a, R: ModelRegistry + ?Sized> {
    registry: &'a R,
    policy: RankingPolicy,
}

impl<'a, R: ModelRegistry + ?Sized> Recommender<'a, R> {
    /// Construct a recommender over `registry` with the given ranking
    /// `policy`.
    pub fn new(registry: &'a R, policy: RankingPolicy) -> Self {
        Self { registry, policy }
    }

    /// Check one specific model against the requirements.
    pub fn check_model(
        &self,
        model_id: &ModelId,
        required: &RequiredCapabilities,
    ) -> Option<Candidate> {
        let provided = self.registry.lookup(model_id)?;
        let outcome = check(&provided, required);
        let eval_score = self.eval_score_of(&provided);
        Some(Candidate {
            model_id: model_id.clone(),
            outcome,
            provided,
            eval_score,
        })
    }

    /// Return every model in the registry whose check is `TypeOk` or
    /// `DegradedSubtype`, ranked by the configured `RankingPolicy`.
    ///
    /// TypeError candidates are NOT returned — the caller asked for
    /// "candidates that could bind."
    pub fn candidates(&self, required: &RequiredCapabilities) -> Vec<Candidate> {
        let mut out: Vec<Candidate> = self
            .registry
            .entries()
            .into_iter()
            .filter_map(|(id, provided)| {
                let outcome = check(&provided, required);
                if outcome.is_acceptable() {
                    let eval_score = self.eval_score_of(&provided);
                    Some(Candidate {
                        model_id: id,
                        outcome,
                        provided,
                        eval_score,
                    })
                } else {
                    None
                }
            })
            .collect();

        // Stable sort: TypeOk before DegradedSubtype; within each, higher
        // eval score first (if eval_metric is set); otherwise ModelId
        // ascending (deterministic, no implicit preference).
        out.sort_by(|a, b| {
            let a_rank = outcome_rank(&a.outcome);
            let b_rank = outcome_rank(&b.outcome);
            a_rank.cmp(&b_rank).then_with(|| {
                // Higher eval score first → compare b vs a.
                match (a.eval_score, b.eval_score) {
                    (Some(av), Some(bv)) => {
                        bv.partial_cmp(&av).unwrap_or(std::cmp::Ordering::Equal)
                    }
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => a.model_id.cmp(&b.model_id),
                }
            })
        });

        out
    }

    fn eval_score_of(&self, provided: &ProvidedCapabilities) -> Option<f32> {
        self.policy
            .eval_metric
            .as_deref()
            .and_then(|name| provided.eval_scores.get(name).copied())
    }
}

/// Numeric rank for outcome sorting: lower = better.
fn outcome_rank(o: &ValidatorOutcome) -> u8 {
    match o {
        ValidatorOutcome::TypeOk => 0,
        ValidatorOutcome::DegradedSubtype { .. } => 1,
        ValidatorOutcome::TypeError { .. } => 2,
    }
}

// ===========================================================================
// Unit tests (in-module; integration + proptest live in tests/)
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn permissive_provided() -> ProvidedCapabilities {
        ProvidedCapabilities::declared()
            .with_context_window_tokens(8_192)
            .with_native_tool_calling(true)
            .with_quantization(Quantization::Q4KM)
            .with_chat_template(Some("chatml".into()))
            .with_license(License::SpdxId("Apache-2.0".into()))
    }

    #[test]
    fn permissive_required_with_capable_provided_is_type_ok() {
        let outcome = check(&permissive_provided(), &RequiredCapabilities::permissive());
        assert_eq!(outcome, ValidatorOutcome::TypeOk);
    }

    #[test]
    fn context_window_too_small_is_type_error() {
        let provided = ProvidedCapabilities::declared().with_context_window_tokens(2_048);
        let required = RequiredCapabilities {
            min_context_window_tokens: 8_192,
            ..RequiredCapabilities::permissive()
        };
        match check(&provided, &required) {
            ValidatorOutcome::TypeError { missing } => {
                assert!(missing
                    .iter()
                    .any(|m| matches!(m, MissingCapability::ContextWindowTooSmall { .. })));
            }
            other => panic!("expected TypeError, got {other:?}"),
        }
    }

    #[test]
    fn required_tool_calling_missing_is_type_error() {
        let provided = permissive_provided().with_native_tool_calling(false);
        let required = RequiredCapabilities {
            requires_native_tool_calling: true,
            ..RequiredCapabilities::permissive()
        };
        let outcome = check(&provided, &required);
        assert!(outcome.is_type_error());
    }

    #[test]
    fn preferred_tool_calling_missing_is_degraded_not_error() {
        let provided = permissive_provided().with_native_tool_calling(false);
        let required = RequiredCapabilities {
            requires_native_tool_calling: false,
            prefers_native_tool_calling: true,
            ..RequiredCapabilities::permissive()
        };
        match check(&provided, &required) {
            ValidatorOutcome::DegradedSubtype { reasons } => {
                assert!(reasons.contains(&DegradationReason::NativeToolCallingMissing));
            }
            other => panic!("expected DegradedSubtype, got {other:?}"),
        }
    }

    #[test]
    fn required_modality_missing_is_type_error() {
        let provided = permissive_provided(); // only Text
        let required = RequiredCapabilities {
            required_modalities: BTreeSet::from([Modality::Vision]),
            ..RequiredCapabilities::permissive()
        };
        assert!(check(&provided, &required).is_type_error());
    }

    #[test]
    fn quantization_not_in_allowed_set_is_type_error() {
        let provided = permissive_provided().with_quantization(Quantization::Q2K);
        let required = RequiredCapabilities {
            allowed_quantizations: BTreeSet::from([
                Quantization::F16,
                Quantization::Q8_0,
                Quantization::Q4KM,
            ]),
            ..RequiredCapabilities::permissive()
        };
        assert!(check(&provided, &required).is_type_error());
    }

    #[test]
    fn empty_allowed_quantizations_admits_any() {
        let provided = permissive_provided().with_quantization(Quantization::Q2K);
        let required = RequiredCapabilities {
            allowed_quantizations: BTreeSet::new(),
            ..RequiredCapabilities::permissive()
        };
        assert_eq!(check(&provided, &required), ValidatorOutcome::TypeOk);
    }

    #[test]
    fn missing_chat_template_required_is_type_error() {
        let provided = permissive_provided().with_chat_template(None);
        let required = RequiredCapabilities {
            requires_chat_template: true,
            ..RequiredCapabilities::permissive()
        };
        assert!(check(&provided, &required).is_type_error());
    }

    #[test]
    fn license_constraint_no_restriction_admits_any() {
        let provided = permissive_provided().with_license(License::Unknown);
        let required = RequiredCapabilities::permissive();
        assert_eq!(check(&provided, &required), ValidatorOutcome::TypeOk);
    }

    #[test]
    fn license_constraint_commercial_ok_rejects_unknown() {
        let provided = permissive_provided().with_license(License::Unknown);
        let required = RequiredCapabilities {
            license_constraint: LicenseConstraint::RequireCommercialOk,
            ..RequiredCapabilities::permissive()
        };
        assert!(check(&provided, &required).is_type_error());
    }

    #[test]
    fn license_constraint_commercial_ok_admits_apache() {
        let provided = permissive_provided().with_license(License::SpdxId("Apache-2.0".into()));
        let required = RequiredCapabilities {
            license_constraint: LicenseConstraint::RequireCommercialOk,
            ..RequiredCapabilities::permissive()
        };
        assert_eq!(check(&provided, &required), ValidatorOutcome::TypeOk);
    }

    #[test]
    fn license_constraint_one_of_works() {
        let provided = permissive_provided().with_license(License::SpdxId("MIT".into()));
        let allowed: BTreeSet<License> = BTreeSet::from([
            License::SpdxId("MIT".into()),
            License::SpdxId("Apache-2.0".into()),
        ]);
        let required = RequiredCapabilities {
            license_constraint: LicenseConstraint::OneOf(allowed),
            ..RequiredCapabilities::permissive()
        };
        assert_eq!(check(&provided, &required), ValidatorOutcome::TypeOk);
    }

    #[test]
    fn multiple_missing_capabilities_all_reported() {
        let provided = ProvidedCapabilities::declared(); // minimal: 0 ctx, no tools, F32, no template, Unknown license
        let required = RequiredCapabilities {
            min_context_window_tokens: 4096,
            requires_native_tool_calling: true,
            requires_chat_template: true,
            license_constraint: LicenseConstraint::RequireCommercialOk,
            ..RequiredCapabilities::permissive()
        };
        match check(&provided, &required) {
            ValidatorOutcome::TypeError { missing } => {
                assert_eq!(missing.len(), 4, "expected 4 missing, got: {missing:?}");
            }
            other => panic!("expected TypeError, got {other:?}"),
        }
    }

    // ---- Recommender ----------------------------------------------------

    #[test]
    fn recommender_filters_out_type_errors() {
        let mut reg = InMemoryModelRegistry::new();
        reg.insert(
            ModelId("small".into()),
            ProvidedCapabilities::declared().with_context_window_tokens(2_048),
        );
        reg.insert(
            ModelId("big".into()),
            ProvidedCapabilities::declared().with_context_window_tokens(32_768),
        );
        let r = Recommender::new(&reg, RankingPolicy::default());
        let required = RequiredCapabilities {
            min_context_window_tokens: 16_000,
            ..RequiredCapabilities::permissive()
        };
        let cands = r.candidates(&required);
        assert_eq!(cands.len(), 1);
        assert_eq!(cands[0].model_id, ModelId("big".into()));
    }

    #[test]
    fn recommender_ranks_type_ok_before_degraded() {
        let mut reg = InMemoryModelRegistry::new();
        // model_a: has native tool calling → TypeOk
        reg.insert(
            ModelId("a-native".into()),
            ProvidedCapabilities::declared()
                .with_context_window_tokens(8_192)
                .with_native_tool_calling(true),
        );
        // model_b: lacks native tool calling but it's only preferred → Degraded
        reg.insert(
            ModelId("b-emulated".into()),
            ProvidedCapabilities::declared()
                .with_context_window_tokens(8_192)
                .with_native_tool_calling(false),
        );
        let r = Recommender::new(&reg, RankingPolicy::default());
        let required = RequiredCapabilities {
            prefers_native_tool_calling: true,
            ..RequiredCapabilities::permissive()
        };
        let cands = r.candidates(&required);
        assert_eq!(cands.len(), 2);
        // TypeOk first
        assert_eq!(cands[0].model_id, ModelId("a-native".into()));
        assert_eq!(cands[0].outcome, ValidatorOutcome::TypeOk);
        // Degraded second
        assert_eq!(cands[1].model_id, ModelId("b-emulated".into()));
        assert!(matches!(
            cands[1].outcome,
            ValidatorOutcome::DegradedSubtype { .. }
        ));
    }

    #[test]
    fn recommender_uses_named_eval_when_set() {
        let mut reg = InMemoryModelRegistry::new();
        reg.insert(
            ModelId("alpha".into()),
            ProvidedCapabilities::declared()
                .with_context_window_tokens(8_192)
                .with_eval_score("mmlu", 0.65),
        );
        reg.insert(
            ModelId("beta".into()),
            ProvidedCapabilities::declared()
                .with_context_window_tokens(8_192)
                .with_eval_score("mmlu", 0.82),
        );
        let policy = RankingPolicy {
            eval_metric: Some("mmlu".into()),
        };
        let r = Recommender::new(&reg, policy);
        let cands = r.candidates(&RequiredCapabilities::permissive());
        assert_eq!(cands[0].model_id, ModelId("beta".into())); // higher mmlu first
        assert_eq!(cands[1].model_id, ModelId("alpha".into()));
    }

    #[test]
    fn check_model_returns_none_for_unknown() {
        let reg = InMemoryModelRegistry::new();
        let r = Recommender::new(&reg, RankingPolicy::default());
        assert!(r
            .check_model(
                &ModelId("ghost".into()),
                &RequiredCapabilities::permissive(),
            )
            .is_none());
    }

    #[test]
    fn outcome_is_acceptable_helpers() {
        assert!(ValidatorOutcome::TypeOk.is_acceptable());
        assert!(ValidatorOutcome::DegradedSubtype { reasons: vec![] }.is_acceptable());
        assert!(!ValidatorOutcome::TypeError { missing: vec![] }.is_acceptable());
        assert!(!ValidatorOutcome::TypeOk.is_type_error());
    }
}
