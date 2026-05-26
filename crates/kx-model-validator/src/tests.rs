//! Inline unit tests for the crate. Integration + proptest tests live under
//! `tests/`. This file is the post-Rule-3-split extraction of what was the
//! `#[cfg(test)] mod tests { ... }` block at the bottom of the original
//! `lib.rs`; bodies are unchanged.

use std::collections::BTreeSet;

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
