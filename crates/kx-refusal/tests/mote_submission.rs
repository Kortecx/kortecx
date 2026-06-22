//! M1.3 — `validate_mote_submission` (the single-Mote SubmitMote-boundary gate):
//! the R-1/R-7/R-8/R-14/R-15 + R-10/D66 decision table, the D66 fail-closed
//! property (WM-only), and the narrow-vs-full sibling contract. These are the
//! UNIT + behavioral assertions for the predicate at its home crate; the
//! coordinator wires it (see `kx-coordinator/tests/submission_refusal.rs`).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::collections::{BTreeMap, BTreeSet};

use kx_mote::{
    EffectPattern, GraphPosition, InputDataId, LogicRef, ModelId, Mote, MoteDef, MoteId, NdClass,
    PromptTemplateHash, ToolName, ToolVersion, MOTE_DEF_SCHEMA_VERSION,
};
use kx_refusal::{
    native_critic_shape, native_judge_shape, validate_mote_submission, validate_submission,
    SubmissionRefusal, ToolResolution, WorkflowSubmission,
};
use kx_tool_registry::IdempotencyClass;
use smallvec::SmallVec;

fn warrant() -> kx_warrant::WarrantSpec {
    kx_warrant::WarrantSpec {
        mote_class: kx_warrant::MoteClass::Pure,
        nd_class: kx_warrant::MoteClass::Pure,
        fs_scope: kx_warrant::FsScope::empty(),
        net_scope: kx_warrant::NetScope::None,
        syscall_profile_ref: kx_content::ContentRef::from_bytes([0; 32]),
        tool_grants: BTreeSet::new(),
        model_route: kx_warrant::ModelRoute {
            model_id: ModelId("local".into()),
            max_input_tokens: 0,
            max_output_tokens: 0,
            max_calls: 0,
        },
        resource_ceiling: kx_warrant::ResourceCeiling {
            cpu_milli: 0,
            mem_bytes: 0,
            wall_clock_ms: 0,
            fd_count: 0,
            disk_bytes: 0,
        },
        environment_ref: None,
        executor_class: kx_warrant::ExecutorClass::Bwrap,
        ..Default::default()
    }
}

#[allow(clippy::too_many_arguments)]
fn build_mote(
    seed: u8,
    nd_class: NdClass,
    effect_pattern: EffectPattern,
    critic_for: Option<MoteId>,
    is_topology_shaper: bool,
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
        critic_for,
        is_topology_shaper,
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

/// A WORLD-MUTATING producer with a non-empty tool_contract (so R-1 never fires),
/// `StageThenCommit` (so R-2/R-9 are irrelevant), no critic role.
fn wm_producer(seed: u8) -> Mote {
    let mut tc = BTreeMap::new();
    tc.insert(ToolName("fs-write".into()), ToolVersion("1".into()));
    build_mote(
        seed,
        NdClass::WorldMutating,
        EffectPattern::StageThenCommit,
        None,
        false,
        tc,
    )
}

// === D66 — fail-closed on a tool-resolution miss (WORLD-MUTATING only) ========

#[test]
fn d66_refuses_world_mutating_with_unresolved_tools() {
    let err =
        validate_mote_submission(&wm_producer(1), false, &ToolResolution::Unresolved).unwrap_err();
    assert!(matches!(
        err,
        SubmissionRefusal::D66UnresolvableWorldMutatingTools { .. }
    ));
}

#[test]
fn d66_does_not_refuse_pure_with_unresolved_tools() {
    // A PURE Mote carries no double-fire hazard — an unresolvable warrant grant is
    // NOT a D66 refusal (it keeps M1.2's capture-skip behavior).
    let pure = build_mote(
        1,
        NdClass::Pure,
        EffectPattern::IdempotentByConstruction,
        None,
        false,
        BTreeMap::new(),
    );
    assert!(validate_mote_submission(&pure, false, &ToolResolution::Unresolved).is_ok());
}

#[test]
fn d66_does_not_refuse_read_only_nondet_with_unresolved_tools() {
    let rond = build_mote(
        1,
        NdClass::ReadOnlyNondet,
        EffectPattern::IdempotentByConstruction,
        None,
        false,
        BTreeMap::new(),
    );
    assert!(validate_mote_submission(&rond, false, &ToolResolution::Unresolved).is_ok());
}

// === R-10 — resolved AtLeastOnce gate (with the accept opt-in) ================

#[test]
fn clean_world_mutating_with_resolved_non_at_least_once_is_ok() {
    let resolution =
        ToolResolution::Resolved(vec![IdempotencyClass::Token, IdempotencyClass::Staged]);
    assert!(validate_mote_submission(&wm_producer(2), false, &resolution).is_ok());
}

#[test]
fn r10_refuses_at_least_once_without_accept() {
    let resolution = ToolResolution::Resolved(vec![IdempotencyClass::AtLeastOnce]);
    let err = validate_mote_submission(&wm_producer(3), false, &resolution).unwrap_err();
    assert!(matches!(
        err,
        SubmissionRefusal::R10AtLeastOnceWithoutAccept { .. }
    ));
}

#[test]
fn r10_accepts_at_least_once_with_explicit_accept() {
    let resolution = ToolResolution::Resolved(vec![IdempotencyClass::AtLeastOnce]);
    assert!(validate_mote_submission(&wm_producer(3), true, &resolution).is_ok());
}

// === R-1 precedence over D66 ==================================================

#[test]
fn r1_precedes_d66_for_empty_contract_idempotent_wm() {
    // A WM IdempotentByConstruction Mote with an EMPTY tool_contract is an R-1
    // refusal — checked BEFORE the D66 resolution miss.
    let m = build_mote(
        4,
        NdClass::WorldMutating,
        EffectPattern::IdempotentByConstruction,
        None,
        false,
        BTreeMap::new(),
    );
    let err = validate_mote_submission(&m, false, &ToolResolution::Unresolved).unwrap_err();
    assert!(
        matches!(err, SubmissionRefusal::R1NoIdempotentTool { .. }),
        "R-1 (not D66) fires first for an empty-contract IdempotentByConstruction WM Mote"
    );
}

// === Other sibling-INDEPENDENT predicates fire on the narrow path =============

#[test]
fn r7_self_refuses_world_mutating_critic() {
    let target = MoteId::from_bytes([0xEE; 32]);
    let mut tc = BTreeMap::new();
    tc.insert(ToolName("fs-write".into()), ToolVersion("1".into()));
    let wm_critic = build_mote(
        5,
        NdClass::WorldMutating,
        EffectPattern::StageThenCommit,
        Some(target),
        false,
        tc,
    );
    let err = validate_mote_submission(
        &wm_critic,
        false,
        &ToolResolution::Resolved(vec![IdempotencyClass::Staged]),
    )
    .unwrap_err();
    assert!(matches!(
        err,
        SubmissionRefusal::R7WorldMutatingCritic { .. }
    ));
}

#[test]
fn r8_refuses_shaper_that_is_also_a_critic() {
    let target = MoteId::from_bytes([0xEE; 32]);
    let shaper_critic = build_mote(
        6,
        NdClass::Pure,
        EffectPattern::IdempotentByConstruction,
        Some(target),
        true,
        BTreeMap::new(),
    );
    let err = validate_mote_submission(&shaper_critic, false, &ToolResolution::Resolved(vec![]))
        .unwrap_err();
    assert!(matches!(err, SubmissionRefusal::R8ShaperAndCritic { .. }));
}

#[test]
fn r14_refuses_world_mutating_shaper() {
    let mut tc = BTreeMap::new();
    tc.insert(ToolName("fs-write".into()), ToolVersion("1".into()));
    let wm_shaper = build_mote(
        7,
        NdClass::WorldMutating,
        EffectPattern::StageThenCommit,
        None,
        true,
        tc,
    );
    let err = validate_mote_submission(
        &wm_shaper,
        false,
        &ToolResolution::Resolved(vec![IdempotencyClass::Staged]),
    )
    .unwrap_err();
    assert!(matches!(
        err,
        SubmissionRefusal::R14WorldMutatingShaper { .. }
    ));
}

// === The narrow-vs-full sibling contract ======================================

#[test]
fn narrow_path_does_not_run_sibling_dependent_predicates() {
    // A critic Mote whose `critic_for` target is NOT in this single submit: the
    // single-Mote path must NOT refuse it (R-4 needs the target sibling, which is
    // submitted separately) — but the FULL-graph `validate_submission` DOES refuse
    // it (R-4). This locks the two-surface contract M1.3 relies on.
    let phantom_target = MoteId::from_bytes([0xAB; 32]);
    let critic = build_mote(
        8,
        NdClass::Pure,
        EffectPattern::IdempotentByConstruction,
        Some(phantom_target),
        false,
        BTreeMap::new(),
    );

    // Narrow (boundary) path: Ok — sibling predicates are not run.
    assert!(
        validate_mote_submission(&critic, false, &ToolResolution::Resolved(vec![])).is_ok(),
        "single-Mote path does not run R-4 (the target sibling is submitted separately)"
    );

    // Full-graph path: refused — R-4 (critic target missing).
    let mut motes = BTreeMap::new();
    motes.insert(critic.id, critic);
    let submission = WorkflowSubmission {
        run_id: [0u8; 32],
        master_warrant: warrant(),
        motes,
        accept_at_least_once: BTreeMap::new(),
    };
    let err = validate_submission(&submission).unwrap_err();
    assert!(
        matches!(err, SubmissionRefusal::R4CriticTargetMissing { .. }),
        "full-graph path runs R-4"
    );
}

// ---------------------------------------------------------------------------
// PR-2: code() ↔ Display-prefix pin
// ---------------------------------------------------------------------------

/// Every refusal's `code()` is exactly the prefix its `Display` message opens
/// with (`"{code}:"`). The gateway surfaces `code()` as `kx-refusal-code` gRPC
/// metadata; this pin guarantees the structured code can never drift from the
/// prose the same client reads in the Status detail.
#[test]
fn refusal_code_matches_display_prefix_for_every_variant() {
    let id = MoteId::from_bytes([0xcd; 32]);
    let other = MoteId::from_bytes([0xce; 32]);
    let variants: Vec<SubmissionRefusal> = vec![
        SubmissionRefusal::R1NoIdempotentTool { mote_id: id },
        SubmissionRefusal::R2NoCritic { mote_id: id },
        SubmissionRefusal::R3EffectPatternMissing { mote_id: id },
        SubmissionRefusal::R4CriticTargetMissing {
            mote_id: id,
            target: other,
        },
        SubmissionRefusal::R5CriticTargetWrongClass {
            mote_id: id,
            target: other,
            target_class: NdClass::Pure,
        },
        SubmissionRefusal::R6MultiCritic {
            first_critic: id,
            second_critic: other,
            target: id,
        },
        SubmissionRefusal::R7WorldMutatingCritic { mote_id: id },
        SubmissionRefusal::R8ShaperAndCritic { mote_id: id },
        SubmissionRefusal::R8bShaperImperativeSpawn { mote_id: id },
        SubmissionRefusal::R9CriticChainNotTerminating { mote_id: id },
        SubmissionRefusal::ValidatorTypeError {
            mote_id: id,
            missing_summary: "x".into(),
        },
        SubmissionRefusal::AttemptedWiden {
            mote_id: id,
            narrowing_error: "x".into(),
        },
        SubmissionRefusal::R10AtLeastOnceWithoutAccept { mote_id: id },
        SubmissionRefusal::R14WorldMutatingShaper { mote_id: id },
        SubmissionRefusal::R15NativeCheckShape { mote_id: id },
        SubmissionRefusal::D66UnresolvableWorldMutatingTools { mote_id: id },
    ];
    assert_eq!(variants.len(), 16, "the refusal vocabulary is CLOSED at 16");
    for refusal in &variants {
        let prose = refusal.to_string();
        let expected_prefix = format!("{}:", refusal.code());
        assert!(
            prose.starts_with(&expected_prefix),
            "Display for {:?} must open with '{expected_prefix}', got '{prose}'",
            refusal.code(),
        );
    }
}

// ===================== T-AGENT2 — the LLM-judge SHAPE gate =====================

/// Build a critic Mote carrying the given `critic_check` spec + nd_class + producer.
fn build_critic(
    nd_class: NdClass,
    critic_for: Option<MoteId>,
    is_topology_shaper: bool,
    spec: Option<kx_critic_types::CheckSpec>,
) -> Mote {
    let def = MoteDef {
        critic_check: spec,
        logic_ref: LogicRef::from_bytes([1; 32]),
        model_id: ModelId("local".into()),
        prompt_template_hash: PromptTemplateHash::from_bytes([2; 32]),
        tool_contract: BTreeMap::new(),
        nd_class,
        config_subset: BTreeMap::new(),
        effect_pattern: EffectPattern::IdempotentByConstruction,
        critic_for,
        is_topology_shaper,
        inference_params: kx_mote::InferenceParams::default(),
        schema_version: MOTE_DEF_SCHEMA_VERSION,
    };
    Mote::new(
        def,
        InputDataId::from_bytes([0; 32]),
        GraphPosition(vec![0xAA]),
        SmallVec::new(),
    )
}

fn llm_judge_spec() -> kx_critic_types::CheckSpec {
    kx_critic_types::CheckSpec::LlmJudge(kx_critic_types::LlmJudgeSpec {
        max_output_tokens: 64,
    })
}

#[test]
fn judge_shape_accepts_read_only_nondet_judge() {
    let producer = MoteId::from_bytes([5; 32]);
    let judge = build_critic(
        NdClass::ReadOnlyNondet,
        Some(producer),
        false,
        Some(llm_judge_spec()),
    );
    // The judge gate accepts a well-formed ReadOnlyNondet judge.
    assert!(native_judge_shape(&judge).is_ok());
    // The Pure-only native gate SKIPS a judge (so it can never mis-refuse it AND
    // stays a byte-mirror of the frozen executor's native-only R-15).
    assert!(native_critic_shape(&judge).is_ok());
    // The submission dispatch routes the judge to the judge gate ⇒ accepted.
    assert!(
        validate_mote_submission(&judge, false, &ToolResolution::Unresolved).is_ok(),
        "a well-formed ReadOnlyNondet LLM-judge must be admitted"
    );
}

#[test]
fn judge_shape_refuses_pure_judge() {
    // A judge that is Pure (the native-critic class) is ill-formed — a judge
    // samples the model, so it MUST be ReadOnlyNondet. Fail-closed under R-15.
    let producer = MoteId::from_bytes([5; 32]);
    let bad = build_critic(NdClass::Pure, Some(producer), false, Some(llm_judge_spec()));
    assert!(matches!(
        native_judge_shape(&bad),
        Err(SubmissionRefusal::R15NativeCheckShape { .. })
    ));
    assert!(matches!(
        validate_mote_submission(&bad, false, &ToolResolution::Unresolved),
        Err(SubmissionRefusal::R15NativeCheckShape { .. })
    ));
}

#[test]
fn judge_shape_refuses_judge_without_producer_or_as_shaper() {
    // No producer ⇒ R-15.
    let orphan = build_critic(NdClass::ReadOnlyNondet, None, false, Some(llm_judge_spec()));
    assert!(native_judge_shape(&orphan).is_err());
    // A topology shaper that is also a judge ⇒ refused by the judge gate.
    let producer = MoteId::from_bytes([5; 32]);
    let shaper_judge = build_critic(
        NdClass::ReadOnlyNondet,
        Some(producer),
        true,
        Some(llm_judge_spec()),
    );
    assert!(native_judge_shape(&shaper_judge).is_err());
}

#[test]
fn native_gate_still_refuses_read_only_nondet_native_critic() {
    // A NATIVE check (not a judge) that is ReadOnlyNondet stays refused — the
    // judge relaxation must NOT leak into the deterministic-check class.
    let producer = MoteId::from_bytes([5; 32]);
    let native_spec = kx_critic_types::CheckSpec::Schema(kx_critic_types::SchemaSpec {
        expected: kx_critic_types::SchemaTag::Json,
    });
    let bad = build_critic(
        NdClass::ReadOnlyNondet,
        Some(producer),
        false,
        Some(native_spec),
    );
    assert!(matches!(
        native_critic_shape(&bad),
        Err(SubmissionRefusal::R15NativeCheckShape { .. })
    ));
    assert!(validate_mote_submission(&bad, false, &ToolResolution::Unresolved).is_err());
}
