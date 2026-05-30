//! ITEM-B — verify-by-rerun integration test.
//!
//! Confirms: a PURE Mote re-runs to a byte-identical `result_ref` (Confirmed); a
//! wrong expected ref reports Diverged; and the PURE-only guard refuses
//! ReadOnlyNondet + WorldMutating before any execution.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::collections::{BTreeMap, BTreeSet};

use kx_content::ContentRef;
use kx_executor::{
    verify_pure_rerun, LocalResourceManager, MoteExecutor, Rootfs, TestMoteExecutor, VerifyError,
    VerifyOutcome,
};
use kx_mote::{
    EffectPattern, GraphPosition, InputDataId, LogicRef, ModelId, Mote, MoteDef, NdClass,
    PromptTemplateHash, MOTE_DEF_SCHEMA_VERSION,
};
use kx_warrant::{
    ExecutorClass, FsScope, ModelRoute, MoteClass, NetScope, ResourceCeiling, WarrantSpec,
};
use smallvec::SmallVec;

fn permissive_warrant() -> WarrantSpec {
    WarrantSpec {
        mote_class: MoteClass::Pure,
        nd_class: MoteClass::Pure,
        fs_scope: FsScope::empty(),
        net_scope: NetScope::None,
        syscall_profile_ref: ContentRef::from_bytes([0; 32]),
        tool_grants: BTreeSet::new(),
        model_route: ModelRoute {
            model_id: ModelId("local".into()),
            max_input_tokens: 0,
            max_output_tokens: 0,
            max_calls: 0,
        },
        resource_ceiling: ResourceCeiling {
            cpu_milli: 0,
            mem_bytes: 0,
            wall_clock_ms: 0,
            fd_count: 0,
            disk_bytes: 0,
        },
        environment_ref: None,
        executor_class: ExecutorClass::Bwrap,
    }
}

fn mote_with_class(nd: NdClass, effect_pattern: EffectPattern, pos: u8) -> Mote {
    let def = MoteDef {
        critic_check: None,
        logic_ref: LogicRef::from_bytes([1; 32]),
        model_id: ModelId("local".into()),
        prompt_template_hash: PromptTemplateHash::from_bytes([2; 32]),
        tool_contract: BTreeMap::new(),
        nd_class: nd,
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
        GraphPosition(vec![pos]),
        SmallVec::new(),
    )
}

#[test]
fn pure_rerun_confirms_when_body_is_reproducible() {
    let rm = LocalResourceManager::dev_defaults();
    let executor = TestMoteExecutor::deterministic();
    let warrant = permissive_warrant();
    let mote = mote_with_class(NdClass::Pure, EffectPattern::IdempotentByConstruction, 1);

    // The committed ref a caller would have read from the journal == a prior run.
    let expected = executor
        .run(&mote, &warrant, None::<Rootfs>)
        .unwrap()
        .result_ref;

    let outcome = verify_pure_rerun(&mote, &warrant, expected, &rm, &executor).unwrap();
    assert_eq!(
        outcome,
        VerifyOutcome::Confirmed {
            result_ref: expected
        }
    );
    assert!(outcome.is_confirmed());
}

#[test]
fn pure_rerun_diverges_when_expected_differs() {
    let rm = LocalResourceManager::dev_defaults();
    let executor = TestMoteExecutor::deterministic();
    let warrant = permissive_warrant();
    let mote = mote_with_class(NdClass::Pure, EffectPattern::IdempotentByConstruction, 2);

    let bogus = ContentRef::from_bytes([0xAB; 32]);
    let outcome = verify_pure_rerun(&mote, &warrant, bogus, &rm, &executor).unwrap();
    match outcome {
        VerifyOutcome::Diverged { expected, observed } => {
            assert_eq!(expected, bogus);
            assert_ne!(
                observed, bogus,
                "the re-run produced its own (different) ref"
            );
        }
        other => panic!("expected Diverged, got {other:?}"),
    }
    assert!(!outcome.is_confirmed());
}

#[test]
fn rerun_refuses_read_only_nondet_before_execution() {
    let rm = LocalResourceManager::dev_defaults();
    let executor = TestMoteExecutor::deterministic();
    let warrant = permissive_warrant();
    let mote = mote_with_class(NdClass::ReadOnlyNondet, EffectPattern::StageThenCommit, 3);

    let err = verify_pure_rerun(
        &mote,
        &warrant,
        ContentRef::from_bytes([1; 32]),
        &rm,
        &executor,
    )
    .unwrap_err();
    assert!(matches!(err, VerifyError::NotPure(NdClass::ReadOnlyNondet)));
}

#[test]
fn rerun_refuses_world_mutating_before_execution() {
    let rm = LocalResourceManager::dev_defaults();
    let executor = TestMoteExecutor::deterministic();
    let warrant = permissive_warrant();
    let mote = mote_with_class(NdClass::WorldMutating, EffectPattern::StageThenCommit, 4);

    let err = verify_pure_rerun(
        &mote,
        &warrant,
        ContentRef::from_bytes([1; 32]),
        &rm,
        &executor,
    )
    .unwrap_err();
    assert!(matches!(err, VerifyError::NotPure(NdClass::WorldMutating)));
}
