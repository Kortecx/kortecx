//! [`ValidatorOutcome`] — the three-variant type-check result, plus the
//! named [`MissingCapability`] / [`DegradationReason`] vocabularies that
//! make each refusal/degradation reason inspectable.

use serde::{Deserialize, Serialize};

use crate::capabilities::{License, Modality, Quantization};

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

/// Outcome of a [`crate::check`] call.
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
