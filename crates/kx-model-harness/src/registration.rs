//! M-agent — register the campaign model as a typed, validated default.
//!
//! The companion model repo (`kortecx-model-base-0.0.1`) finetunes **Qwen3-4B**
//! (Apache-2.0, `q4_k_m` GGUF, ChatML, native tool-calling that emits the
//! `{"tool_call":{name,version,args}}` envelope) and, in its `export/`
//! `register_kortecx` step, registers a [`ModelDescriptor`] +
//! [`ProvidedCapabilities`] and asserts the validator returns
//! [`ValidatorOutcome::TypeOk`]. This module is the runtime-side counterpart:
//! one call builds the descriptor + capabilities for a GGUF and type-checks it
//! against the kortecx agent's [`RequiredCapabilities`].
//!
//! The check is **fail-closed**: a model that does not satisfy the agent's
//! signature (no native tool-calling, wrong modality, a non-commercial license,
//! a quantization outside the allow-set) is refused here rather than discovered
//! mid-dispatch. Capabilities are `Declared` (D29 v1 trusts the declaration; a
//! load-time probe is a later `Verified` upgrade).

use std::collections::BTreeSet;
use std::path::PathBuf;

use kx_model_store::ModelDescriptor;
use kx_model_validator::{
    check, License, LicenseConstraint, Modality, ProvidedCapabilities, Quantization,
    RequiredCapabilities, ValidatorOutcome,
};
use kx_mote::ModelId;

/// Minimum context window (tokens) the kortecx agent's tool-use loop requires.
/// A floor, not the model's full window — kept low enough that the small
/// stand-in models used before the finetune ships also bind.
pub const AGENT_MIN_CTX_TOKENS: u32 = 2048;

/// Why [`register_kortecx`] refused to register a model.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RegistrationError {
    /// The model's declared capabilities did not satisfy the agent's required
    /// signature. Carries the validator's verdict for diagnostics.
    #[error("model is not TypeOk for the kortecx agent: {0:?}")]
    NotTypeOk(ValidatorOutcome),
}

/// The kortecx agent's required model signature: native tool-calling, Text,
/// a chat template, a commercial-OK license, and a `q4_k_m`/`q8_0`/`f16`
/// quantization. Exposed so callers and tests can inspect the contract the
/// campaign model is validated against.
#[must_use]
pub fn kortecx_agent_requirements() -> RequiredCapabilities {
    RequiredCapabilities {
        min_context_window_tokens: AGENT_MIN_CTX_TOKENS,
        requires_native_tool_calling: true,
        prefers_native_tool_calling: true,
        required_modalities: BTreeSet::from([Modality::Text]),
        allowed_quantizations: BTreeSet::from([
            Quantization::Q4KM,
            Quantization::Q8_0,
            Quantization::F16,
        ]),
        requires_chat_template: true,
        license_constraint: LicenseConstraint::RequireCommercialOk,
    }
}

/// The Apache-2.0, Text, native-tool-calling, ChatML, `q4_k_m` capabilities the
/// campaign model declares.
#[must_use]
pub fn kortecx_agent_provided(context_window: u32) -> ProvidedCapabilities {
    ProvidedCapabilities::declared()
        .with_context_window_tokens(context_window)
        .with_native_tool_calling(true)
        .with_modalities(BTreeSet::from([Modality::Text]))
        .with_quantization(Quantization::Q4KM)
        .with_chat_template(Some("chatml".to_string()))
        .with_license(License::SpdxId("Apache-2.0".to_string()))
}

/// Build the campaign model's [`ModelDescriptor`] + [`ProvidedCapabilities`] for
/// `gguf` and assert it type-checks against [`kortecx_agent_requirements`].
///
/// `context_window` is the `n_ctx` the runtime will serve the model at (e.g.
/// from [`kx_model_store::read_context_length`], clamped). Returns the pair on
/// [`ValidatorOutcome::TypeOk`]; otherwise [`RegistrationError::NotTypeOk`].
///
/// # Errors
/// [`RegistrationError::NotTypeOk`] if the model fails the agent's signature.
pub fn register_kortecx(
    model_id: ModelId,
    gguf: impl Into<PathBuf>,
    context_window: u32,
) -> Result<(ModelDescriptor, ProvidedCapabilities), RegistrationError> {
    let descriptor = ModelDescriptor::text(model_id, gguf, context_window);
    let provided = kortecx_agent_provided(context_window);
    match check(&provided, &kortecx_agent_requirements()) {
        ValidatorOutcome::TypeOk => Ok((descriptor, provided)),
        other => Err(RegistrationError::NotTypeOk(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mid() -> ModelId {
        ModelId("qwen3-4b-instruct:q4_k_m:deadbeef".to_string())
    }

    #[test]
    fn qwen3_agent_model_is_type_ok() {
        let (desc, provided) =
            register_kortecx(mid(), "/m/qwen3-4b.gguf", 40960).expect("agent model binds");
        assert_eq!(desc.context_window, 40960);
        assert!(provided.native_tool_calling);
        assert_eq!(
            check(&provided, &kortecx_agent_requirements()),
            ValidatorOutcome::TypeOk
        );
    }

    #[test]
    fn small_standin_at_floor_ctx_still_binds() {
        // The pre-finetune stand-in may be served at a modest n_ctx; as long as
        // it meets the floor it must still register.
        let r = register_kortecx(mid(), "/m/standin.gguf", AGENT_MIN_CTX_TOKENS);
        assert!(r.is_ok(), "a model at the ctx floor must bind: {r:?}");
    }

    #[test]
    fn below_floor_ctx_is_refused() {
        let r = register_kortecx(mid(), "/m/tiny.gguf", AGENT_MIN_CTX_TOKENS - 1);
        assert!(matches!(r, Err(RegistrationError::NotTypeOk(_))));
    }

    #[test]
    fn non_tool_calling_model_is_refused() {
        // A provided-caps set without native tool-calling fails the agent's
        // hard requirement — proves the gate actually bites.
        let provided = kortecx_agent_provided(8192).with_native_tool_calling(false);
        assert!(matches!(
            check(&provided, &kortecx_agent_requirements()),
            ValidatorOutcome::TypeError { .. }
        ));
    }
}
