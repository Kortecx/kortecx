//! The [`check`] function — pure, total, deterministic structural-subtyping
//! check of [`ProvidedCapabilities`] against [`RequiredCapabilities`].

use crate::outcome::{DegradationReason, MissingCapability, ValidatorOutcome};
use crate::provided::ProvidedCapabilities;
use crate::requirements::RequiredCapabilities;

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
