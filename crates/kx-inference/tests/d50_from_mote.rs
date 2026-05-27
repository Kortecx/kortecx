// Integration-test file.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]
//! D50 — dispatcher-side conversion tests for `inference_params_from_mote`.
//!
//! Pre-D50 `InferenceParams::from_warrant(&WarrantSpec)` took decoding params
//! from greedy defaults (ignoring the Mote). Post-D50,
//! `inference_params_from_mote(&Mote, &WarrantSpec)` reads decoding params
//! from `mote.def.inference_params` (identity-bearing) and refuses with
//! `ScopeViolation` when the mote declares a `max_output_tokens` above the
//! warrant's ceiling.

use std::collections::{BTreeMap, BTreeSet};

use kx_content::ContentRef;
use kx_inference::{inference_params_from_mote, InferenceError, InferenceParams};
use kx_mote::{
    EffectPattern, GraphPosition, InputDataId, LogicRef, ModelId, Mote, MoteDef, NdClass,
    PromptTemplateHash, MOTE_DEF_SCHEMA_VERSION,
};
use kx_warrant::{
    ExecutorClass, FsScope, ModelRoute, MoteClass, NetScope, ResourceCeiling, WarrantSpec,
};
use smallvec::SmallVec;

fn mote_with_params(params: InferenceParams) -> Mote {
    let def = MoteDef {
        logic_ref: LogicRef::from_bytes([0x11; 32]),
        model_id: ModelId("llama-3-8b:q4".into()),
        prompt_template_hash: PromptTemplateHash::from_bytes([0x22; 32]),
        tool_contract: BTreeMap::new(),
        nd_class: NdClass::ReadOnlyNondet,
        config_subset: BTreeMap::new(),
        effect_pattern: EffectPattern::IdempotentByConstruction,
        critic_for: None,
        is_topology_shaper: false,
        inference_params: params,
        schema_version: MOTE_DEF_SCHEMA_VERSION,
    };
    Mote::new(
        def,
        InputDataId::from_bytes([0x33; 32]),
        GraphPosition(b"root".to_vec()),
        SmallVec::new(),
    )
}

fn warrant_with_ceiling(max_output_tokens: u32) -> WarrantSpec {
    WarrantSpec {
        mote_class: MoteClass::ReadOnlyNondet,
        nd_class: MoteClass::ReadOnlyNondet,
        fs_scope: FsScope {
            mounts: BTreeMap::new(),
        },
        net_scope: NetScope::None,
        syscall_profile_ref: ContentRef::from_bytes([0u8; 32]),
        tool_grants: BTreeSet::new(),
        model_route: ModelRoute {
            model_id: ModelId("llama-3-8b:q4".into()),
            max_input_tokens: 2048,
            max_output_tokens,
            max_calls: 100,
        },
        resource_ceiling: ResourceCeiling {
            cpu_milli: 1000,
            mem_bytes: 1 << 30,
            wall_clock_ms: 60_000,
            fd_count: 64,
            disk_bytes: 1 << 28,
        },
        environment_ref: None,
        executor_class: ExecutorClass::Bwrap,
    }
}

#[test]
fn from_mote_reads_decoding_fields_from_mote_def_inference_params() {
    let mote = mote_with_params(InferenceParams {
        max_output_tokens: 256,
        temperature_bps: 5_000,
        top_p_bps: 9_000,
        top_k: 50,
        seed: 1337,
        stop_tokens: SmallVec::new(),
        grammar: None,
    });
    let warrant = warrant_with_ceiling(512);

    let params = inference_params_from_mote(&mote, &warrant)
        .expect("from_mote must succeed when mote's params are within warrant ceiling");

    assert_eq!(params.max_output_tokens, 256);
    assert_eq!(params.temperature_bps, 5_000);
    assert_eq!(params.top_p_bps, 9_000);
    assert_eq!(params.top_k, 50);
    assert_eq!(params.seed, 1337);
}

#[test]
fn from_mote_returns_scope_violation_when_mote_widens_warrant_ceiling() {
    // Mote declares max_output_tokens = 2048; warrant only allows 512.
    let mote = mote_with_params(InferenceParams {
        max_output_tokens: 2048,
        ..InferenceParams::default()
    });
    let warrant = warrant_with_ceiling(512);

    let err = inference_params_from_mote(&mote, &warrant)
        .expect_err("from_mote MUST refuse when mote.max_output_tokens > warrant ceiling");
    match err {
        InferenceError::ScopeViolation {
            field,
            requested,
            ceiling,
        } => {
            assert_eq!(field, "max_output_tokens");
            assert_eq!(requested, 2048);
            assert_eq!(ceiling, 512);
        }
        other => panic!("expected ScopeViolation, got {other:?}"),
    }
}

#[test]
fn from_mote_accepts_mote_equal_to_warrant_ceiling() {
    // Boundary: max_output_tokens == warrant ceiling is allowed (≤ check).
    let mote = mote_with_params(InferenceParams {
        max_output_tokens: 512,
        ..InferenceParams::default()
    });
    let warrant = warrant_with_ceiling(512);
    let params = inference_params_from_mote(&mote, &warrant).expect("equality is within ceiling");
    assert_eq!(params.max_output_tokens, 512);
}

#[test]
fn from_mote_is_deterministic_for_same_inputs() {
    let mote = mote_with_params(InferenceParams {
        max_output_tokens: 128,
        temperature_bps: 2_500,
        seed: 42,
        ..InferenceParams::default()
    });
    let warrant = warrant_with_ceiling(256);

    let a = inference_params_from_mote(&mote, &warrant).expect("call 1");
    let b = inference_params_from_mote(&mote, &warrant).expect("call 2");
    assert_eq!(a, b, "from_mote MUST be a pure function of (mote, warrant)");
}
