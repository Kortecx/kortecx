// Integration-test file: compiled as a separate crate from the host lib;
// inherits workspace `[lints]` deny on `unwrap_used` / `expect_used` but tests
// legitimately use `.unwrap()` for fixture construction. The `pedantic` group
// is also allowed here — tests routinely do things pedantic flags (small-int
// casts on byte seeds, helper-fn definitions after let-bindings, etc.) that
// would be needless friction to refactor.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]
//! Property tests for `check` and the `Recommender` (SN-4 v2 #6).
//!
//! The validator's correctness contract from D29 is:
//! - **Pure**: identical inputs yield identical outputs.
//! - **Total**: every input pair returns a `ValidatorOutcome`.
//! - **Deterministic**: across threads, across calls, across rebuilds.
//!
//! These properties pin those across the arbitrary-input space.
//!
//! Properties:
//!
//! 1. `check(p, r)` is total — never panics.
//! 2. `check(p, r)` is deterministic — same inputs → same outcome.
//! 3. The `TypeError.missing` list is non-empty iff outcome is `TypeError`.
//! 4. `permissive` requirements + any provider → `TypeOk` or
//!    `DegradedSubtype` (never `TypeError` — permissive has no requirements).
//! 5. `Recommender::candidates` returns only acceptable candidates (no
//!    `TypeError` ever leaks through).
//! 6. `Recommender::candidates` is rank-stable: TypeOk before
//!    DegradedSubtype.
//! 7. Reflexivity: if `provided` satisfies `required` per the documented
//!    rules, `check` returns an acceptable outcome (or a `TypeError` whose
//!    `missing` list is consistent with the input).

use kx_model_validator::{
    check, DegradationReason, InMemoryModelRegistry, License, LicenseConstraint, MissingCapability,
    Modality, ProvidedCapabilities, Quantization, RankingPolicy, Recommender, RequiredCapabilities,
    ValidatorOutcome,
};
use kx_mote::ModelId;
use proptest::prelude::*;
use std::collections::{BTreeMap, BTreeSet};

// ---------------------------------------------------------------------------
// Strategies
// ---------------------------------------------------------------------------

fn arb_modality() -> impl Strategy<Value = Modality> {
    prop_oneof![
        Just(Modality::Text),
        Just(Modality::Vision),
        Just(Modality::Audio),
        Just(Modality::Embedding),
    ]
}

fn arb_quantization() -> impl Strategy<Value = Quantization> {
    prop_oneof![
        Just(Quantization::F32),
        Just(Quantization::F16),
        Just(Quantization::Bf16),
        Just(Quantization::Q8_0),
        Just(Quantization::Q5KM),
        Just(Quantization::Q4KM),
        Just(Quantization::Q4_0),
        Just(Quantization::Q2K),
    ]
}

fn arb_license() -> impl Strategy<Value = License> {
    prop_oneof![
        Just(License::SpdxId("Apache-2.0".into())),
        Just(License::SpdxId("MIT".into())),
        Just(License::SpdxId("GPL-3.0".into())),
        Just(License::LlamaCommunity),
        Just(License::OpenWeightsNonCommercial),
        Just(License::Proprietary),
        Just(License::Unknown),
    ]
}

fn arb_license_constraint() -> impl Strategy<Value = LicenseConstraint> {
    prop_oneof![
        Just(LicenseConstraint::NoRestriction),
        Just(LicenseConstraint::RequireCommercialOk),
        Just(LicenseConstraint::RequireRedistributable),
        proptest::collection::btree_set(arb_license(), 1..=3).prop_map(LicenseConstraint::OneOf),
    ]
}

fn arb_modality_set() -> impl Strategy<Value = BTreeSet<Modality>> {
    proptest::collection::btree_set(arb_modality(), 0..=4)
}

fn arb_quantization_set() -> impl Strategy<Value = BTreeSet<Quantization>> {
    proptest::collection::btree_set(arb_quantization(), 0..=8)
}

fn arb_chat_template() -> impl Strategy<Value = Option<String>> {
    prop_oneof![
        Just(None),
        Just(Some("chatml".into())),
        Just(Some("llama-3-instruct".into())),
        Just(Some("mistral-instruct".into())),
    ]
}

prop_compose! {
    fn arb_provided()(
        ctx in 0u32..=128_000,
        ntc in any::<bool>(),
        mods in arb_modality_set(),
        q in arb_quantization(),
        tmpl in arb_chat_template(),
        lic in arb_license(),
        m1_name in "[a-z]{2,6}",
        m1_score in 0.0f32..=1.0,
    ) -> ProvidedCapabilities {
        let mut p = ProvidedCapabilities::declared()
            .with_context_window_tokens(ctx)
            .with_native_tool_calling(ntc)
            .with_modalities(mods)
            .with_quantization(q)
            .with_chat_template(tmpl)
            .with_license(lic);
        let mut evals = BTreeMap::new();
        evals.insert(m1_name, m1_score);
        p.eval_scores = evals;
        p
    }
}

prop_compose! {
    fn arb_required()(
        ctx in 0u32..=128_000,
        ntc_req in any::<bool>(),
        ntc_pref in any::<bool>(),
        mods in arb_modality_set(),
        q_set in arb_quantization_set(),
        templ in any::<bool>(),
        lic_c in arb_license_constraint(),
    ) -> RequiredCapabilities {
        RequiredCapabilities {
            min_context_window_tokens: ctx,
            requires_native_tool_calling: ntc_req,
            prefers_native_tool_calling: ntc_pref,
            required_modalities: mods,
            allowed_quantizations: q_set,
            requires_chat_template: templ,
            license_constraint: lic_c,
        }
    }
}

// ---------------------------------------------------------------------------
// Properties
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        .. ProptestConfig::default()
    })]

    /// Property 1: `check` is total — never panics on any input pair.
    #[test]
    fn prop_check_is_total(p in arb_provided(), r in arb_required()) {
        let _ = check(&p, &r); // Reaching this line proves no panic.
    }

    /// Property 2: `check` is deterministic — same inputs → same outcome.
    #[test]
    fn prop_check_is_deterministic(p in arb_provided(), r in arb_required()) {
        let a = check(&p, &r);
        let b = check(&p, &r);
        prop_assert_eq!(a, b);
    }

    /// Property 3: `TypeError.missing` is non-empty iff outcome is `TypeError`.
    #[test]
    fn prop_type_error_missing_is_non_empty(p in arb_provided(), r in arb_required()) {
        match check(&p, &r) {
            ValidatorOutcome::TypeError { missing } => {
                prop_assert!(
                    !missing.is_empty(),
                    "TypeError must carry at least one MissingCapability"
                );
            }
            ValidatorOutcome::DegradedSubtype { reasons } => {
                prop_assert!(
                    !reasons.is_empty(),
                    "DegradedSubtype must carry at least one DegradationReason"
                );
            }
            ValidatorOutcome::TypeOk => {} // no inner data to check
        }
    }

    /// Property 4: permissive requirements + any provider → acceptable.
    /// Permissive has zero requirements, so no `TypeError` can be produced.
    /// At worst the outcome is `DegradedSubtype` (which permissive doesn't
    /// trigger either — preferences are zero) — so always `TypeOk`.
    #[test]
    fn prop_permissive_requirements_always_type_ok(p in arb_provided()) {
        let outcome = check(&p, &RequiredCapabilities::permissive());
        prop_assert_eq!(outcome, ValidatorOutcome::TypeOk);
    }

    /// Property 5: `Recommender::candidates` filters out `TypeError`.
    #[test]
    fn prop_recommender_filters_type_errors(
        providers in proptest::collection::vec(arb_provided(), 0..=8),
        r in arb_required(),
    ) {
        let mut reg = InMemoryModelRegistry::new();
        for (i, p) in providers.iter().enumerate() {
            reg.insert(ModelId(format!("model-{i}")), p.clone());
        }
        let rec = Recommender::new(&reg, RankingPolicy::default());
        let cands = rec.candidates(&r);
        for c in &cands {
            prop_assert!(
                c.outcome.is_acceptable(),
                "candidate {:?} has non-acceptable outcome {:?}",
                c.model_id,
                c.outcome
            );
        }
    }

    /// Property 6: TypeOk candidates come before DegradedSubtype in the
    /// ranked list.
    #[test]
    fn prop_recommender_ranks_type_ok_first(
        providers in proptest::collection::vec(arb_provided(), 0..=8),
        r in arb_required(),
    ) {
        let mut reg = InMemoryModelRegistry::new();
        for (i, p) in providers.iter().enumerate() {
            reg.insert(ModelId(format!("model-{i}")), p.clone());
        }
        let rec = Recommender::new(&reg, RankingPolicy::default());
        let cands = rec.candidates(&r);
        // Verify no Degraded appears before a TypeOk in the list.
        let mut seen_degraded = false;
        for c in &cands {
            match c.outcome {
                ValidatorOutcome::TypeOk => {
                    prop_assert!(
                        !seen_degraded,
                        "TypeOk candidate appeared after DegradedSubtype"
                    );
                }
                ValidatorOutcome::DegradedSubtype { .. } => {
                    seen_degraded = true;
                }
                ValidatorOutcome::TypeError { .. } => {
                    prop_assert!(false, "TypeError leaked into candidates");
                }
            }
        }
    }

    /// Property 7: reflexivity — when the missing list cites a capability,
    /// `provided` actually lacks it (decoder/encoder symmetry on the
    /// MissingCapability tags).
    #[test]
    fn prop_missing_list_is_consistent_with_provided(
        p in arb_provided(),
        r in arb_required(),
    ) {
        if let ValidatorOutcome::TypeError { missing } = check(&p, &r) {
            for m in &missing {
                match m {
                    MissingCapability::ContextWindowTooSmall { provided, required } => {
                        prop_assert_eq!(*provided, p.context_window_tokens);
                        prop_assert_eq!(*required, r.min_context_window_tokens);
                        prop_assert!(*provided < *required);
                    }
                    MissingCapability::NativeToolCallingRequired => {
                        prop_assert!(r.requires_native_tool_calling);
                        prop_assert!(!p.native_tool_calling);
                    }
                    MissingCapability::ModalityMissing { missing } => {
                        prop_assert!(r.required_modalities.contains(missing));
                        prop_assert!(!p.modalities.contains(missing));
                    }
                    MissingCapability::QuantizationNotAllowed { provided } => {
                        prop_assert_eq!(*provided, p.quantization);
                        prop_assert!(!r.allowed_quantizations.is_empty());
                        prop_assert!(!r.allowed_quantizations.contains(provided));
                    }
                    MissingCapability::ChatTemplateRequired => {
                        prop_assert!(r.requires_chat_template);
                        prop_assert!(p.chat_template.is_none());
                    }
                    MissingCapability::LicenseUnsatisfied { provided } => {
                        prop_assert_eq!(provided, &p.license);
                        prop_assert!(!r.license_constraint.is_satisfied_by(&p.license));
                    }
                }
            }
        }
    }

    /// Property 8: only-preferred (not required) capability missing →
    /// outcome is `DegradedSubtype`, never `TypeError`. Pin the
    /// preferred-vs-required boundary.
    #[test]
    fn prop_only_preferred_missing_yields_degraded(p in arb_provided()) {
        // Build requirements where everything is satisfied EXCEPT
        // native_tool_calling is preferred (not required) and the provider
        // lacks it.
        let mut p_no_tool = p.clone();
        p_no_tool.native_tool_calling = false;

        let r = RequiredCapabilities {
            min_context_window_tokens: 0, // satisfied
            requires_native_tool_calling: false, // not required
            prefers_native_tool_calling: true,   // preferred
            required_modalities: BTreeSet::new(),
            allowed_quantizations: BTreeSet::new(),
            requires_chat_template: false,
            license_constraint: LicenseConstraint::NoRestriction,
        };

        let outcome = check(&p_no_tool, &r);
        match outcome {
            ValidatorOutcome::DegradedSubtype { reasons } => {
                prop_assert!(reasons.contains(&DegradationReason::NativeToolCallingMissing));
            }
            other => prop_assert!(false, "expected DegradedSubtype, got {other:?}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Concurrency: pure-function determinism across threads (SN-4 v2 #7)
// ---------------------------------------------------------------------------

/// Compile-time `Send + Sync` assertions for the public types. The crate is
/// pure data; every type must be cross-thread shareable.
#[test]
fn public_types_are_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}

    assert_send_sync::<Modality>();
    assert_send_sync::<Quantization>();
    assert_send_sync::<License>();
    assert_send_sync::<LicenseConstraint>();
    assert_send_sync::<RequiredCapabilities>();
    assert_send_sync::<ProvidedCapabilities>();
    assert_send_sync::<ValidatorOutcome>();
    assert_send_sync::<DegradationReason>();
    assert_send_sync::<MissingCapability>();
    assert_send_sync::<InMemoryModelRegistry>();
    assert_send_sync::<RankingPolicy>();
}

/// `check` is a pure function — calling it concurrently from 4 threads with
/// the same inputs must produce identical results. Catches any hypothetical
/// thread-local in the validator path.
#[test]
fn check_is_thread_independent() {
    use std::sync::Arc;
    use std::thread;

    let provided = Arc::new(
        ProvidedCapabilities::declared()
            .with_context_window_tokens(8192)
            .with_native_tool_calling(true)
            .with_quantization(Quantization::Q4KM)
            .with_chat_template(Some("chatml".into()))
            .with_license(License::SpdxId("Apache-2.0".into())),
    );
    let required = Arc::new(RequiredCapabilities {
        min_context_window_tokens: 4096,
        requires_native_tool_calling: true,
        prefers_native_tool_calling: true,
        required_modalities: BTreeSet::from([Modality::Text]),
        allowed_quantizations: BTreeSet::from([Quantization::Q4KM]),
        requires_chat_template: true,
        license_constraint: LicenseConstraint::RequireCommercialOk,
    });

    let mut handles = Vec::with_capacity(4);
    for _ in 0..4 {
        let p = Arc::clone(&provided);
        let r = Arc::clone(&required);
        handles.push(thread::spawn(move || check(&p, &r)));
    }
    let results: Vec<ValidatorOutcome> = handles
        .into_iter()
        .map(|h| h.join().expect("worker panic"))
        .collect();

    let first = &results[0];
    for (i, r) in results.iter().enumerate() {
        assert_eq!(
            r, first,
            "thread {i} produced a different outcome than thread 0"
        );
    }
    assert_eq!(*first, ValidatorOutcome::TypeOk);
}

/// `Recommender::candidates` is pure over its inputs (the registry is
/// shared `Arc`-style). Calling it concurrently must produce identical
/// ranked lists.
#[test]
fn recommender_is_thread_independent() {
    use std::sync::Arc;
    use std::thread;

    let mut reg = InMemoryModelRegistry::new();
    for (name, ctx) in [
        ("alpha", 2048),
        ("beta", 8192),
        ("gamma", 32_768),
        ("delta", 128_000),
    ] {
        reg.insert(
            ModelId(name.into()),
            ProvidedCapabilities::declared().with_context_window_tokens(ctx),
        );
    }
    let reg: Arc<InMemoryModelRegistry> = Arc::new(reg);

    let required = Arc::new(RequiredCapabilities {
        min_context_window_tokens: 4096,
        ..RequiredCapabilities::permissive()
    });

    let mut handles = Vec::with_capacity(4);
    for _ in 0..4 {
        let reg = Arc::clone(&reg);
        let r = Arc::clone(&required);
        handles.push(thread::spawn(move || {
            let rec = Recommender::new(&*reg, RankingPolicy::default());
            rec.candidates(&r)
                .into_iter()
                .map(|c| c.model_id)
                .collect::<Vec<_>>()
        }));
    }

    let results: Vec<Vec<ModelId>> = handles
        .into_iter()
        .map(|h| h.join().expect("worker panic"))
        .collect();

    let first = &results[0];
    for (i, r) in results.iter().enumerate() {
        assert_eq!(
            r, first,
            "thread {i} produced a different ranked list than thread 0"
        );
    }
    // beta, gamma, delta satisfy 4096-token requirement; alpha does not.
    assert_eq!(first.len(), 3);
    assert!(!first.contains(&ModelId("alpha".into())));
}
