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
    validate_mote_submission, validate_submission, SubmissionRefusal, ToolResolution,
    WorkflowSubmission,
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
