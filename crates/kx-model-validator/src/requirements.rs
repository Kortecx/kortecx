//! [`RequiredCapabilities`] — the workflow-author-supplied type signature a
//! model must satisfy at bind time.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::capabilities::{LicenseConstraint, Modality, Quantization};

/// What a task requires from any model it will be bound to.
///
/// Workflow-author-supplied at workflow-compile time. The runtime never
/// invents one.
///
/// Per D29's "no MoteDef field for v1" decision, this lives on the SDK side
/// as a separate input to [`crate::check`], NOT on the Mote's identity hash.
/// Adding it to `MoteDef` would force a `schema_version` bump (v3 → v4);
/// the v1 validator delivers its value without that disturbance. Re-evaluate
/// at P4.
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
