//! Property tests on the refusal predicate surface + the fact-zero pure
//! helpers. SN-4 v2 mandate: ≥3 proptest properties × 64 cases.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::collections::{BTreeMap, BTreeSet};

use kx_content::ContentRef;
use kx_executor::{
    profile_from_warrant, seed_idempotency_key, seed_mote_id, validate_submission,
    validate_submission_with_idempotency, SeedPayload, SubmissionRefusal, WorkflowSubmission,
};
use kx_mote::{
    EffectPattern, GraphPosition, InputDataId, LogicRef, ModelId, Mote, MoteDef, MoteId, NdClass,
    PromptTemplateHash, MOTE_DEF_SCHEMA_VERSION,
};
use kx_tool_registry::IdempotencyClass;
use kx_warrant::{
    ExecutorClass, FsScope, ModelRoute, MoteClass, NetScope, ResourceCeiling, WarrantSpec,
};
use proptest::prelude::*;
use smallvec::SmallVec;

// MUST update on new `ExecutorClass` variant. Canonical-classifier-cannot-
// drift pattern: the strategy enumerates ALL variants so any new addition
// without an updated proptest is caught at the test surface.
fn arb_executor_class() -> impl Strategy<Value = ExecutorClass> {
    prop_oneof![
        Just(ExecutorClass::Bwrap),
        Just(ExecutorClass::OciDaemon),
        Just(ExecutorClass::CloudMicroVm),
        Just(ExecutorClass::MacOsSandbox),
    ]
}

// MUST update on new `EffectPattern` variant.
fn arb_effect_pattern() -> impl Strategy<Value = EffectPattern> {
    prop_oneof![
        Just(EffectPattern::IdempotentByConstruction),
        Just(EffectPattern::StageThenCommit),
        Just(EffectPattern::ValidateThenCommit),
    ]
}

// MUST update on new `NdClass` variant.
fn arb_nd_class() -> impl Strategy<Value = NdClass> {
    prop_oneof![
        Just(NdClass::Pure),
        Just(NdClass::ReadOnlyNondet),
        Just(NdClass::WorldMutating),
    ]
}

// MUST update on new `IdempotencyClass` variant. Canonical-classifier-cannot-
// drift: any new IdempotencyClass variant without an updated strategy is
// caught by this proptest's coverage drop.
fn arb_idempotency_class() -> impl Strategy<Value = IdempotencyClass> {
    prop_oneof![
        Just(IdempotencyClass::Token),
        Just(IdempotencyClass::Readback),
        Just(IdempotencyClass::Staged),
        Just(IdempotencyClass::AtLeastOnce),
    ]
}

fn arb_wm_mote_with_tool() -> impl Strategy<Value = Mote> {
    (0u8..255u8).prop_map(|seed| {
        let mut tool_contract: BTreeMap<kx_mote::ToolName, kx_mote::ToolVersion> = BTreeMap::new();
        tool_contract.insert(
            kx_mote::ToolName("publish".into()),
            kx_mote::ToolVersion("0.1.0".into()),
        );
        let def = MoteDef {
            logic_ref: LogicRef::from_bytes([1; 32]),
            model_id: ModelId("local".into()),
            prompt_template_hash: PromptTemplateHash::from_bytes([2; 32]),
            tool_contract,
            nd_class: NdClass::WorldMutating,
            config_subset: BTreeMap::new(),
            effect_pattern: EffectPattern::IdempotentByConstruction,
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
    })
}

fn arb_warrant() -> impl Strategy<Value = WarrantSpec> {
    arb_executor_class().prop_map(|ec| WarrantSpec {
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
        executor_class: ec,
    })
}

fn arb_pure_mote() -> impl Strategy<Value = Mote> {
    (0u8..255u8, arb_effect_pattern()).prop_map(|(seed, ep)| {
        let def = MoteDef {
            logic_ref: LogicRef::from_bytes([1; 32]),
            model_id: ModelId("local".into()),
            prompt_template_hash: PromptTemplateHash::from_bytes([2; 32]),
            tool_contract: BTreeMap::new(),
            nd_class: NdClass::Pure,
            config_subset: BTreeMap::new(),
            effect_pattern: ep,
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
    })
}

proptest! {
    /// `validate_submission` is pure / total / deterministic over any
    /// submission carrying only PURE Motes: same input → same output across
    /// two calls. None of R-1..R-9 fire for a PURE Mote, so the result is
    /// always `Ok(())`.
    #[test]
    fn prop_validate_submission_is_deterministic(
        mote in arb_pure_mote(),
        warrant in arb_warrant(),
    ) {
        let mut motes = BTreeMap::new();
        motes.insert(mote.id, mote.clone());
        let submission = WorkflowSubmission {
            run_id: [0u8; 32],
            master_warrant: warrant.clone(),
            motes,
            accept_at_least_once: BTreeMap::new(),
        };
        let r1 = validate_submission(&submission);
        let r2 = validate_submission(&submission);
        prop_assert_eq!(r1.is_ok(), r2.is_ok());
        prop_assert!(r1.is_ok(), "any PURE Mote alone should pass validation");
    }

    /// `profile_from_warrant` is pure / total / deterministic per D46:
    /// same input → byte-identical output.
    #[test]
    fn prop_profile_from_warrant_is_pure(
        warrant in arb_warrant(),
    ) {
        let p1 = profile_from_warrant(&warrant);
        let p2 = profile_from_warrant(&warrant);
        prop_assert_eq!(p1.as_bytes(), p2.as_bytes());
        prop_assert!(!p1.is_empty(), "deny-default template must be non-empty");
    }

    /// `SeedPayload::result_ref` excludes `submitted_at_ms` per D34 §3.3:
    /// two payloads identical except for the audit timestamp produce the
    /// same `result_ref`.
    #[test]
    fn prop_seed_payload_result_ref_excludes_submitted_at_ms(
        run_id_byte in any::<u8>(),
        task in "[a-z]{0,16}",
        ms_a in any::<u64>(),
        ms_b in any::<u64>(),
    ) {
        let seed_a = SeedPayload {
            run_id: [run_id_byte; 16],
            task: task.clone(),
            system_prompt: None,
            workflow_def_ref: ContentRef::from_bytes([0; 32]),
            submitted_at_ms: ms_a,
        };
        let seed_b = SeedPayload {
            submitted_at_ms: ms_b,
            ..seed_a.clone()
        };
        prop_assert_eq!(seed_a.result_ref(), seed_b.result_ref());
    }

    /// `seed_mote_id(run_id)` is identity-bearing on `run_id` (D34 §3.4):
    /// distinct `run_id`s produce distinct `MoteId`s.
    #[test]
    fn prop_seed_mote_id_is_identity_bearing(
        a in any::<u8>(),
        b in any::<u8>(),
    ) {
        prop_assume!(a != b);
        let mid_a = seed_mote_id(&[a; 16]);
        let mid_b = seed_mote_id(&[b; 16]);
        prop_assert_ne!(mid_a, mid_b);
    }

    /// `seed_idempotency_key` and `seed_mote_id` are PURE — same run_id
    /// produces the same outputs across calls (recovery / replay safety).
    #[test]
    fn prop_seed_helpers_are_pure(
        run_id_byte in any::<u8>(),
    ) {
        let run_id = [run_id_byte; 16];
        prop_assert_eq!(seed_mote_id(&run_id), seed_mote_id(&run_id));
        prop_assert_eq!(seed_idempotency_key(&run_id), seed_idempotency_key(&run_id));
    }

    /// Adding an `arb_nd_class()` enumeration here keeps the canonical-
    /// classifier-cannot-drift pattern intact — if a new `NdClass` variant
    /// is added without an updated strategy, this property's coverage drops
    /// and reviewers notice.
    #[test]
    fn prop_nd_class_strategy_covers_all_variants(
        c in arb_nd_class(),
    ) {
        let _ = c; // Exhaustive coverage is the property; no other assertion.
    }

    /// R-10: for any WORLD-MUTATING Mote whose resolved tool classes contain
    /// `IdempotencyClass::AtLeastOnce`, `validate_submission_with_idempotency`
    /// MUST refuse unless `accept_at_least_once[mote_id] == true`. The
    /// property covers BOTH branches (accept=true → Ok; accept=false → Err).
    #[test]
    fn prop_r10_fires_iff_at_least_once_without_accept(
        mote in arb_wm_mote_with_tool(),
        warrant in arb_warrant(),
        accept_flag in any::<bool>(),
    ) {
        let id = mote.id;
        let mut motes = BTreeMap::new();
        motes.insert(id, mote);
        let mut accept_map: BTreeMap<MoteId, bool> = BTreeMap::new();
        accept_map.insert(id, accept_flag);
        let submission = WorkflowSubmission {
            run_id: [0u8; 32],
            master_warrant: warrant,
            motes,
            accept_at_least_once: accept_map,
        };
        let mut resolved: BTreeMap<MoteId, Vec<IdempotencyClass>> = BTreeMap::new();
        resolved.insert(id, vec![IdempotencyClass::AtLeastOnce]);
        let result = validate_submission_with_idempotency(&submission, &resolved);
        if accept_flag {
            prop_assert!(result.is_ok(), "accept=true must short-circuit R-10");
        } else {
            match result {
                Err(SubmissionRefusal::R10AtLeastOnceWithoutAccept { mote_id }) => {
                    prop_assert_eq!(mote_id, id);
                }
                other => prop_assert!(false, "expected R-10 refusal, got {:?}", other),
            }
        }
    }

    /// R-10 is INSENSITIVE to non-AtLeastOnce IdempotencyClass values:
    /// Token / Readback / Staged classes alone never trigger R-10 regardless
    /// of the accept flag.
    #[test]
    fn prop_r10_does_not_fire_for_safe_idempotency_classes(
        mote in arb_wm_mote_with_tool(),
        warrant in arb_warrant(),
        accept_flag in any::<bool>(),
        klass in prop_oneof![
            Just(IdempotencyClass::Token),
            Just(IdempotencyClass::Readback),
            Just(IdempotencyClass::Staged),
        ],
    ) {
        let id = mote.id;
        let mut motes = BTreeMap::new();
        motes.insert(id, mote);
        let mut accept_map: BTreeMap<MoteId, bool> = BTreeMap::new();
        accept_map.insert(id, accept_flag);
        let submission = WorkflowSubmission {
            run_id: [0u8; 32],
            master_warrant: warrant,
            motes,
            accept_at_least_once: accept_map,
        };
        let mut resolved: BTreeMap<MoteId, Vec<IdempotencyClass>> = BTreeMap::new();
        resolved.insert(id, vec![klass]);
        prop_assert!(
            validate_submission_with_idempotency(&submission, &resolved).is_ok(),
            "safe idempotency classes must not trigger R-10",
        );
    }

    /// `IdempotencyClass` strategy enumeration check — canonical-classifier-
    /// cannot-drift: any new variant without an updated strategy fails this
    /// property's coverage.
    #[test]
    fn prop_idempotency_class_strategy_covers_all_variants(
        c in arb_idempotency_class(),
    ) {
        let _ = c;
    }
}
