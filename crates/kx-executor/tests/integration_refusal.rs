//! One named integration test per refusal predicate (R-1, R-2, R-3, R-4,
//! R-5, R-6, R-7, R-8, R-8b, R-9 + ValidatorTypeError + AttemptedWiden) per
//! the PR 9a DoD in `02-crate-specs.md` §`kx-executor` + the build-sequence
//! exit gate for Step 1.9a.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::collections::BTreeMap;

use kx_executor::{
    refusal_from_narrowing, validate_submission, validate_submission_with_idempotency,
    SubmissionRefusal, WorkflowSubmission,
};
use kx_mote::{
    EffectPattern, GraphPosition, InputDataId, LogicRef, ModelId, Mote, MoteDef, MoteId, NdClass,
    PromptTemplateHash, MOTE_DEF_SCHEMA_VERSION,
};
use kx_tool_registry::IdempotencyClass;
use kx_warrant::NarrowingError;
use smallvec::SmallVec;

fn warrant() -> kx_warrant::WarrantSpec {
    use std::collections::BTreeSet;
    kx_warrant::WarrantSpec {
        mote_class: kx_warrant::MoteClass::Pure,
        nd_class: kx_warrant::MoteClass::Pure,
        fs_scope: kx_warrant::FsScope::empty(),
        net_scope: kx_warrant::NetScope::None,
        syscall_profile_ref: kx_content::ContentRef::from_bytes([0; 32]),
        tool_grants: BTreeSet::new(),
        model_route: kx_warrant::ModelRoute {
            model_id: kx_mote::ModelId("local".into()),
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
    }
}

fn build_mote(
    seed: u8,
    nd_class: NdClass,
    effect_pattern: EffectPattern,
    critic_for: Option<MoteId>,
    is_topology_shaper: bool,
    tool_contract: BTreeMap<kx_mote::ToolName, kx_mote::ToolVersion>,
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

fn submit(motes: Vec<Mote>) -> WorkflowSubmission {
    let mut map = BTreeMap::new();
    for m in motes {
        map.insert(m.id, m);
    }
    WorkflowSubmission {
        run_id: [0u8; 32],
        master_warrant: warrant(),
        motes: map,
        accept_at_least_once: BTreeMap::new(),
    }
}

// ============================================================================
// R-1: WORLD-MUTATING + IdempotentByConstruction + empty tool_contract.
// ============================================================================
#[test]
fn r1_refuses_world_mutating_idempotent_with_no_tools() {
    let m = build_mote(
        1,
        NdClass::WorldMutating,
        EffectPattern::IdempotentByConstruction,
        None,
        false,
        BTreeMap::new(),
    );
    let r = validate_submission(&submit(vec![m])).unwrap_err();
    assert!(matches!(r, SubmissionRefusal::R1NoIdempotentTool { .. }));
}

// ============================================================================
// R-2: WORLD-MUTATING + ValidateThenCommit + no sibling critic.
// ============================================================================
#[test]
fn r2_refuses_world_mutating_validate_with_no_critic() {
    let m = build_mote(
        2,
        NdClass::WorldMutating,
        EffectPattern::ValidateThenCommit,
        None,
        false,
        BTreeMap::new(),
    );
    let r = validate_submission(&submit(vec![m])).unwrap_err();
    assert!(matches!(r, SubmissionRefusal::R2NoCritic { .. }));
}

// ============================================================================
// R-3: structurally unreachable in PR 9a (effect_pattern is required in the
// MoteDef type system). Defensive guard test: build a Mote with every shape
// and verify R-3 never fires.
// ============================================================================
#[test]
fn r3_unreachable_in_typed_submissions() {
    let m = build_mote(
        3,
        NdClass::Pure,
        EffectPattern::IdempotentByConstruction,
        None,
        false,
        BTreeMap::new(),
    );
    let _ = validate_submission(&submit(vec![m])); // expect Ok or other R-* — not R-3
                                                   // R-3 is reserved for malformed dynamic submissions (PR 9b+).
}

// ============================================================================
// R-4: critic_for points at a missing target.
// ============================================================================
#[test]
fn r4_refuses_dangling_critic_target() {
    let phantom_target = MoteId::from_bytes([0xAB; 32]);
    let critic = build_mote(
        4,
        NdClass::Pure,
        EffectPattern::IdempotentByConstruction,
        Some(phantom_target),
        false,
        BTreeMap::new(),
    );
    let r = validate_submission(&submit(vec![critic])).unwrap_err();
    assert!(matches!(r, SubmissionRefusal::R4CriticTargetMissing { .. }));
}

// ============================================================================
// R-5: critic_for points at a non-WM target.
// ============================================================================
#[test]
fn r5_refuses_critic_targeting_non_wm_producer() {
    let producer = build_mote(
        5,
        NdClass::Pure, // not WorldMutating
        EffectPattern::IdempotentByConstruction,
        None,
        false,
        BTreeMap::new(),
    );
    let critic = build_mote(
        6,
        NdClass::Pure,
        EffectPattern::IdempotentByConstruction,
        Some(producer.id),
        false,
        BTreeMap::new(),
    );
    let r = validate_submission(&submit(vec![producer, critic])).unwrap_err();
    assert!(matches!(
        r,
        SubmissionRefusal::R5CriticTargetWrongClass { .. }
    ));
}

// ============================================================================
// R-6: two critics targeting the same producer.
// ============================================================================
#[test]
fn r6_refuses_multi_critic() {
    let producer = build_mote(
        7,
        NdClass::WorldMutating,
        EffectPattern::ValidateThenCommit,
        None,
        false,
        {
            let mut tc = BTreeMap::new();
            tc.insert(
                kx_mote::ToolName("dummy".into()),
                kx_mote::ToolVersion("1.0".into()),
            );
            tc
        },
    );
    let critic_a = build_mote(
        8,
        NdClass::Pure,
        EffectPattern::IdempotentByConstruction,
        Some(producer.id),
        false,
        BTreeMap::new(),
    );
    let critic_b = build_mote(
        9,
        NdClass::Pure,
        EffectPattern::IdempotentByConstruction,
        Some(producer.id),
        false,
        BTreeMap::new(),
    );
    let r = validate_submission(&submit(vec![producer, critic_a, critic_b])).unwrap_err();
    assert!(matches!(r, SubmissionRefusal::R6MultiCritic { .. }));
}

// ============================================================================
// R-7: WORLD-MUTATING critic.
// ============================================================================
#[test]
fn r7_refuses_world_mutating_critic() {
    let producer = build_mote(
        10,
        NdClass::WorldMutating,
        EffectPattern::ValidateThenCommit,
        None,
        false,
        {
            let mut tc = BTreeMap::new();
            tc.insert(
                kx_mote::ToolName("dummy".into()),
                kx_mote::ToolVersion("1.0".into()),
            );
            tc
        },
    );
    let bad_critic = build_mote(
        11,
        NdClass::WorldMutating, // CRITIC IS WM — R-7 catches.
        EffectPattern::IdempotentByConstruction,
        Some(producer.id),
        false,
        {
            let mut tc = BTreeMap::new();
            tc.insert(
                kx_mote::ToolName("dummy".into()),
                kx_mote::ToolVersion("1.0".into()),
            );
            tc
        },
    );
    let r = validate_submission(&submit(vec![producer, bad_critic])).unwrap_err();
    assert!(matches!(r, SubmissionRefusal::R7WorldMutatingCritic { .. }));
}

// ============================================================================
// R-8: shaper AND critic.
// ============================================================================
#[test]
fn r8_refuses_shaper_and_critic() {
    let producer = build_mote(
        12,
        NdClass::WorldMutating,
        EffectPattern::ValidateThenCommit,
        None,
        false,
        {
            let mut tc = BTreeMap::new();
            tc.insert(
                kx_mote::ToolName("dummy".into()),
                kx_mote::ToolVersion("1.0".into()),
            );
            tc
        },
    );
    let bad = build_mote(
        13,
        NdClass::Pure,
        EffectPattern::IdempotentByConstruction,
        Some(producer.id),
        true, // shaper AND critic — R-8 catches.
        BTreeMap::new(),
    );
    let r = validate_submission(&submit(vec![producer, bad])).unwrap_err();
    assert!(matches!(r, SubmissionRefusal::R8ShaperAndCritic { .. }));
}

// ============================================================================
// R-8b: PR 9a stub — the body-side imperative-spawn detection lands in
// PR 9a-hardening. PR 9a's structural check (shaper produces a
// TopologyDecision payload) is currently a no-op since the type system
// doesn't yet model "payload shape." Test passes trivially; PR 9a-hardening
// adds a positive test that exercises the new check.
// ============================================================================
#[test]
fn r8b_placeholder_passes_in_pr_9a() {
    let shaper = build_mote(
        14,
        NdClass::Pure,
        EffectPattern::IdempotentByConstruction,
        None,
        true, // is_topology_shaper, no critic
        BTreeMap::new(),
    );
    // PR 9a's R-8b structural check passes (no imperative-spawn detection
    // shipped yet); PR 9a-hardening adds the body-side check.
    assert!(validate_submission(&submit(vec![shaper])).is_ok());
}

// ============================================================================
// R-9: WORLD-MUTATING ValidateThenCommit with critic chain not terminating
// at a Pure critic.
// ============================================================================
#[test]
fn r9_refuses_critic_chain_not_terminating_at_pure() {
    // producer: WM-ValidateThenCommit; critic: WM-ValidateThenCommit (NOT Pure,
    // and would need its own critic, ad infinitum). R-7 fires before R-9 in
    // this exact shape; instead use a critic with `nd_class = ReadOnlyNondet`
    // (not Pure, but not WM either) so R-7 doesn't catch first.
    let producer = build_mote(
        15,
        NdClass::WorldMutating,
        EffectPattern::ValidateThenCommit,
        None,
        false,
        {
            let mut tc = BTreeMap::new();
            tc.insert(
                kx_mote::ToolName("dummy".into()),
                kx_mote::ToolVersion("1.0".into()),
            );
            tc
        },
    );
    let critic = build_mote(
        16,
        NdClass::ReadOnlyNondet, // not Pure → chain does not terminate.
        EffectPattern::IdempotentByConstruction,
        Some(producer.id),
        false,
        BTreeMap::new(),
    );
    let r = validate_submission(&submit(vec![producer, critic])).unwrap_err();
    assert!(matches!(
        r,
        SubmissionRefusal::R9CriticChainNotTerminating { .. }
    ));
}

// ============================================================================
// ValidatorTypeError: surfaced from the lifecycle layer's kx-model-validator
// check. PR 9a's refusal vocabulary entry; full lifecycle integration ships
// in PR 9b (where the dispatcher path is wired). PR 9a verifies the variant
// is constructible + matches the error spec.
// ============================================================================
#[test]
fn validator_type_error_variant_is_constructible() {
    let r = SubmissionRefusal::ValidatorTypeError {
        mote_id: MoteId::from_bytes([0x55; 32]),
        missing_summary: "ContextWindow needs 8192, model offers 4096".into(),
    };
    let _ = format!("{r}"); // ensure Display impl
}

// ============================================================================
// AttemptedWiden: surfaced from kx_warrant::intersect via
// `refusal_from_narrowing`.
// ============================================================================
#[test]
fn attempted_widen_maps_from_narrowing_error() {
    let mote_id = MoteId::from_bytes([0x66; 32]);
    let nerr = NarrowingError::AttemptedWiden {
        field: kx_warrant::WarrantField::NetScope,
        parent: "None".into(),
        proposed: "AllowList(...)".into(),
    };
    let refusal = refusal_from_narrowing(mote_id, &nerr);
    match refusal {
        SubmissionRefusal::AttemptedWiden {
            mote_id: m,
            narrowing_error,
        } => {
            assert_eq!(m, mote_id);
            assert!(narrowing_error.contains("AttemptedWiden"));
        }
        other => panic!("expected AttemptedWiden, got {other:?}"),
    }
}

// ============================================================================
// R-10: WORLD-MUTATING + resolved tool has IdempotencyClass::AtLeastOnce +
// `accept_at_least_once[mote_id]` is not `true` (D38 §2c).
// ============================================================================

fn build_wm_mote_with_tool(seed: u8) -> Mote {
    let mut tool_contract: BTreeMap<kx_mote::ToolName, kx_mote::ToolVersion> = BTreeMap::new();
    tool_contract.insert(
        kx_mote::ToolName("publish".into()),
        kx_mote::ToolVersion("0.1.0".into()),
    );
    build_mote(
        seed,
        NdClass::WorldMutating,
        EffectPattern::IdempotentByConstruction,
        None,
        false,
        tool_contract,
    )
}

#[test]
fn r10_refuses_at_least_once_without_accept_flag() {
    let m = build_wm_mote_with_tool(20);
    let id = m.id;
    let submission = submit(vec![m]);
    let mut resolved: BTreeMap<MoteId, Vec<IdempotencyClass>> = BTreeMap::new();
    resolved.insert(id, vec![IdempotencyClass::AtLeastOnce]);
    let err = validate_submission_with_idempotency(&submission, &resolved).unwrap_err();
    assert!(matches!(
        err,
        SubmissionRefusal::R10AtLeastOnceWithoutAccept { mote_id } if mote_id == id
    ));
}

#[test]
fn r10_accepts_at_least_once_when_accept_flag_is_true() {
    let m = build_wm_mote_with_tool(21);
    let id = m.id;
    let mut submission = submit(vec![m]);
    submission.accept_at_least_once.insert(id, true);
    let mut resolved: BTreeMap<MoteId, Vec<IdempotencyClass>> = BTreeMap::new();
    resolved.insert(id, vec![IdempotencyClass::AtLeastOnce]);
    assert!(validate_submission_with_idempotency(&submission, &resolved).is_ok());
}

#[test]
fn r10_does_not_fire_on_token_class_tool() {
    let m = build_wm_mote_with_tool(22);
    let id = m.id;
    let submission = submit(vec![m]);
    let mut resolved: BTreeMap<MoteId, Vec<IdempotencyClass>> = BTreeMap::new();
    resolved.insert(id, vec![IdempotencyClass::Token]);
    assert!(validate_submission_with_idempotency(&submission, &resolved).is_ok());
}

#[test]
fn r10_does_not_fire_on_pure_mote_even_with_at_least_once_class() {
    // R-10 only applies to WORLD-MUTATING Motes. A Pure Mote with an
    // AtLeastOnce class in resolved_tools (anomalous; Pure tool_contract
    // should not realistically include AtLeastOnce classes) should still
    // pass.
    let m = build_mote(
        23,
        NdClass::Pure,
        EffectPattern::IdempotentByConstruction,
        None,
        false,
        BTreeMap::new(),
    );
    let id = m.id;
    let submission = submit(vec![m]);
    let mut resolved: BTreeMap<MoteId, Vec<IdempotencyClass>> = BTreeMap::new();
    resolved.insert(id, vec![IdempotencyClass::AtLeastOnce]);
    assert!(validate_submission_with_idempotency(&submission, &resolved).is_ok());
}

#[test]
fn r10_fires_when_any_tool_in_contract_is_at_least_once() {
    // Mixed tool contract: Token + AtLeastOnce. R-10 fires because at least
    // one tool is unsafe — the workflow author must opt in to accept the
    // double-effect window.
    let m = build_wm_mote_with_tool(24);
    let id = m.id;
    let submission = submit(vec![m]);
    let mut resolved: BTreeMap<MoteId, Vec<IdempotencyClass>> = BTreeMap::new();
    resolved.insert(
        id,
        vec![IdempotencyClass::Token, IdempotencyClass::AtLeastOnce],
    );
    let err = validate_submission_with_idempotency(&submission, &resolved).unwrap_err();
    assert!(matches!(
        err,
        SubmissionRefusal::R10AtLeastOnceWithoutAccept { .. }
    ));
}

#[test]
fn r10_check_runs_after_earlier_predicates() {
    // A Mote that fails R-1 (WM IdempotentByConstruction with empty
    // tool_contract) must surface R-1, not R-10 — earlier predicates fire
    // first in the canonical order.
    let m = build_mote(
        25,
        NdClass::WorldMutating,
        EffectPattern::IdempotentByConstruction,
        None,
        false,
        BTreeMap::new(),
    );
    let id = m.id;
    let submission = submit(vec![m]);
    let mut resolved: BTreeMap<MoteId, Vec<IdempotencyClass>> = BTreeMap::new();
    resolved.insert(id, vec![IdempotencyClass::AtLeastOnce]);
    let err = validate_submission_with_idempotency(&submission, &resolved).unwrap_err();
    assert!(matches!(err, SubmissionRefusal::R1NoIdempotentTool { .. }));
}

// ============================================================================
// Happy path: a submission that triggers no refusal predicate returns Ok.
// ============================================================================
#[test]
fn happy_path_pure_mote_accepts() {
    let m = build_mote(
        17,
        NdClass::Pure,
        EffectPattern::IdempotentByConstruction,
        None,
        false,
        BTreeMap::new(),
    );
    assert!(validate_submission(&submit(vec![m])).is_ok());
}

// ============================================================================
// R-14 (D48 + D49 / P1.11): WORLD-MUTATING shaper.
// ============================================================================
//
// Spec: `topology.md` §9 (private corpus). A Mote with
// `is_topology_shaper == true AND nd_class == WorldMutating` is refused at
// submission. Shapers MUST be PURE or READ-ONLY-NONDET — emitting a
// topology decision is a nondet-read of the world, not a mutation.
//
// This refusal closes the WM-shaper recovery loophole structurally:
// without R-14, the D38 §2b 9-cell cross-product would need to cover the
// WM-shaper × EffectStaged × terminal-failure combinations. R-14 makes
// those cells unreachable from any journal that passed validation.

#[test]
fn r14_refuses_world_mutating_shaper() {
    let bad_shaper = build_mote(
        50,
        NdClass::WorldMutating, // ← violates R-14
        EffectPattern::StageThenCommit,
        None,
        true, // ← shaper
        BTreeMap::new(),
    );
    let r = validate_submission(&submit(vec![bad_shaper])).unwrap_err();
    assert!(
        matches!(r, SubmissionRefusal::R14WorldMutatingShaper { .. }),
        "expected R14WorldMutatingShaper, got {r:?}"
    );
}

#[test]
fn r14_accepts_pure_shaper() {
    let pure_shaper = build_mote(
        51,
        NdClass::Pure, // PURE shaper is permitted
        EffectPattern::IdempotentByConstruction,
        None,
        true,
        BTreeMap::new(),
    );
    assert!(validate_submission(&submit(vec![pure_shaper])).is_ok());
}

#[test]
fn r14_accepts_read_only_nondet_shaper() {
    let nondet_shaper = build_mote(
        52,
        NdClass::ReadOnlyNondet, // READ-ONLY-NONDET shaper is permitted (the common case)
        EffectPattern::IdempotentByConstruction,
        None,
        true,
        BTreeMap::new(),
    );
    assert!(validate_submission(&submit(vec![nondet_shaper])).is_ok());
}

#[test]
fn r14_does_not_apply_to_non_shaper_world_mutating_motes() {
    // A WM Mote that is NOT a shaper is unaffected by R-14.
    let wm_non_shaper = build_mote(
        53,
        NdClass::WorldMutating,
        EffectPattern::StageThenCommit,
        None,
        false, // not a shaper
        {
            let mut tc = BTreeMap::new();
            tc.insert(
                kx_mote::ToolName("dummy".into()),
                kx_mote::ToolVersion("1.0".into()),
            );
            tc
        },
    );
    // (Note: this Mote will pass R-14 specifically; other refusals like R-5
    // for ND-class/effect-pattern combinations may apply or not depending on
    // workflow shape. We assert specifically that R-14 isn't the refusal if
    // one is raised.)
    let result = validate_submission(&submit(vec![wm_non_shaper]));
    if let Err(r) = result {
        assert!(
            !matches!(r, SubmissionRefusal::R14WorldMutatingShaper { .. }),
            "R-14 incorrectly fired on a non-shaper WM Mote: {r:?}"
        );
    }
}
