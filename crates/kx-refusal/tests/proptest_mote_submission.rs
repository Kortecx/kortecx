//! M1.3 property tests for `validate_mote_submission` (SN-4: ≥3 properties).
//!
//! P1 — a non-WORLD-MUTATING plain producer is NEVER refused on resolution
//!      grounds (D66 is WM-only; a non-WM Mote carries no double-fire hazard).
//! P2 — a WORLD-MUTATING plain producer with UNRESOLVED tools is ALWAYS refused
//!      (R-1 if its contract is empty+IdempotentByConstruction, else D66) — the
//!      historical fail-OPEN can never regress.
//! P3 — a WORLD-MUTATING producer with RESOLVED tools matches the R-10 contract:
//!      refused iff (some class is AtLeastOnce AND accept_at_least_once is false).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::collections::BTreeMap;

use kx_mote::{
    EffectPattern, GraphPosition, InputDataId, LogicRef, ModelId, Mote, MoteDef, NdClass,
    PromptTemplateHash, ToolName, ToolVersion, MOTE_DEF_SCHEMA_VERSION,
};
use kx_refusal::{validate_mote_submission, SubmissionRefusal, ToolResolution};
use kx_tool_registry::IdempotencyClass;
use proptest::prelude::*;
use smallvec::SmallVec;

fn plain_mote(
    seed: u8,
    nd_class: NdClass,
    effect_pattern: EffectPattern,
    tool_contract: BTreeMap<ToolName, ToolVersion>,
) -> Mote {
    let def = MoteDef {
        critic_check: None,
        logic_ref: LogicRef::from_bytes([1; 32]),
        model_id: ModelId("local".into()),
        prompt_template_hash: PromptTemplateHash::from_bytes([2; 32]),
        tool_contract,
        nd_class,
        config_subset: BTreeMap::new(),
        effect_pattern,
        critic_for: None,
        is_topology_shaper: false,
        inference_params: kx_mote::InferenceParams::default(),
        schema_version: MOTE_DEF_SCHEMA_VERSION,
    };
    Mote::new(
        def,
        InputDataId::from_bytes([0; 32]),
        GraphPosition(vec![seed]),
        SmallVec::new(),
    )
}

fn effect_pattern_strategy() -> impl Strategy<Value = EffectPattern> {
    prop_oneof![
        Just(EffectPattern::IdempotentByConstruction),
        Just(EffectPattern::StageThenCommit),
        Just(EffectPattern::ValidateThenCommit),
    ]
}

fn class_strategy() -> impl Strategy<Value = IdempotencyClass> {
    prop_oneof![
        Just(IdempotencyClass::Token),
        Just(IdempotencyClass::Readback),
        Just(IdempotencyClass::Staged),
        Just(IdempotencyClass::AtLeastOnce),
    ]
}

fn non_empty_contract() -> BTreeMap<ToolName, ToolVersion> {
    let mut tc = BTreeMap::new();
    tc.insert(ToolName("fs-write".into()), ToolVersion("1".into()));
    tc
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    /// P1 — a non-WM plain producer is never refused on resolution grounds.
    #[test]
    fn p1_non_world_mutating_is_never_refused_on_resolution(
        is_pure in any::<bool>(),
        ep in effect_pattern_strategy(),
        has_tools in any::<bool>(),
        accept in any::<bool>(),
        unresolved in any::<bool>(),
        classes in proptest::collection::vec(class_strategy(), 0..4),
    ) {
        let nd = if is_pure { NdClass::Pure } else { NdClass::ReadOnlyNondet };
        let tc = if has_tools { non_empty_contract() } else { BTreeMap::new() };
        let mote = plain_mote(1, nd, ep, tc);
        let resolution = if unresolved {
            ToolResolution::Unresolved
        } else {
            ToolResolution::Resolved(classes)
        };
        prop_assert!(
            validate_mote_submission(&mote, accept, &resolution).is_ok(),
            "a non-WM plain producer is admitted regardless of tool resolution"
        );
    }

    /// P2 — a WM plain producer with UNRESOLVED tools is always refused.
    #[test]
    fn p2_world_mutating_unresolved_is_always_refused(
        ep in effect_pattern_strategy(),
        has_tools in any::<bool>(),
        accept in any::<bool>(),
    ) {
        let tc = if has_tools { non_empty_contract() } else { BTreeMap::new() };
        let mote = plain_mote(2, NdClass::WorldMutating, ep, tc.clone());
        let result = validate_mote_submission(&mote, accept, &ToolResolution::Unresolved);
        prop_assert!(result.is_err(), "a WM Mote with unresolvable tools is never silently admitted");
        // R-1 only when the contract is empty AND the pattern is IdempotentByConstruction.
        let expect_r1 = tc.is_empty() && ep == EffectPattern::IdempotentByConstruction;
        match result.unwrap_err() {
            SubmissionRefusal::R1NoIdempotentTool { .. } => prop_assert!(expect_r1),
            SubmissionRefusal::D66UnresolvableWorldMutatingTools { .. } => prop_assert!(!expect_r1),
            other => prop_assert!(false, "unexpected refusal {:?}", other),
        }
    }

    /// P3 — a WM producer with RESOLVED tools matches the R-10 contract.
    #[test]
    fn p3_world_mutating_resolved_matches_r10(
        ep in effect_pattern_strategy(),
        accept in any::<bool>(),
        classes in proptest::collection::vec(class_strategy(), 1..5),
    ) {
        // Non-empty contract so R-1 never fires; the resolved classes drive R-10.
        let mote = plain_mote(3, NdClass::WorldMutating, ep, non_empty_contract());
        let result = validate_mote_submission(
            &mote,
            accept,
            &ToolResolution::Resolved(classes.clone()),
        );
        let has_at_least_once = classes.iter().any(|c| matches!(c, IdempotencyClass::AtLeastOnce));
        let should_refuse = has_at_least_once && !accept;
        prop_assert_eq!(result.is_err(), should_refuse);
        if let Err(err) = result {
            prop_assert!(
                matches!(err, SubmissionRefusal::R10AtLeastOnceWithoutAccept { .. }),
                "a resolved WM refusal must be R-10"
            );
        }
    }
}
