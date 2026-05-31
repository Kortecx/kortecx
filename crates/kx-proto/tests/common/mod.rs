//! Shared fixtures: representative domain values used by the round-trip and
//! identity tests. Built from the real `kx-mote` / `kx-warrant` / `kx-content`
//! types so the proto<->domain mapping is exercised against genuine inputs.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::pedantic,
    dead_code,
    unreachable_pub
)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use kx_content::ContentRef;
use kx_mote::{
    ConfigKey, ConfigVal, EdgeMeta, EffectPattern, Grammar, GraphPosition, InferenceParams,
    InputDataId, LogicRef, ModelId, Mote, MoteDef, MoteId, NdClass, ParentRef, PromptTemplateHash,
    ToolName, ToolVersion, MOTE_DEF_SCHEMA_VERSION,
};
use kx_warrant::{
    ExecutorClass, FsMode, FsScope, Host, ModelRoute, MoteClass, NetScope, ResourceCeiling,
    ToolGrant, WarrantSpec,
};
use smallvec::SmallVec;

#[must_use]
pub fn sample_inference_params() -> InferenceParams {
    InferenceParams {
        max_output_tokens: 256,
        temperature_bps: 700,
        top_p_bps: 9_000,
        top_k: 40,
        seed: 12_345,
        stop_tokens: ["STOP".to_string(), "END".to_string()]
            .into_iter()
            .collect(),
        grammar: Some(Grammar::new(r#"{"type":"object"}"#)),
    }
}

#[must_use]
pub fn sample_mote_def() -> MoteDef {
    let mut tool_contract = BTreeMap::new();
    tool_contract.insert(ToolName("fs-read".into()), ToolVersion("1.2.0".into()));
    tool_contract.insert(
        ToolName("text-summarize".into()),
        ToolVersion("0.9.0".into()),
    );

    let mut config_subset = BTreeMap::new();
    config_subset.insert(ConfigKey("max_depth".into()), ConfigVal(vec![3]));

    MoteDef {
        critic_check: None,
        logic_ref: LogicRef::from_bytes([7u8; 32]),
        model_id: ModelId("llama-3.1-8b-instruct-q4_k_m".into()),
        prompt_template_hash: PromptTemplateHash::from_bytes([9u8; 32]),
        tool_contract,
        nd_class: NdClass::WorldMutating,
        config_subset,
        effect_pattern: EffectPattern::StageThenCommit,
        critic_for: Some(MoteId::from_bytes([3u8; 32])),
        is_topology_shaper: false,
        inference_params: sample_inference_params(),
        schema_version: MOTE_DEF_SCHEMA_VERSION,
    }
}

#[must_use]
pub fn sample_mote() -> Mote {
    let parents: SmallVec<[ParentRef; 4]> = [
        ParentRef {
            parent_id: MoteId::from_bytes([1u8; 32]),
            edge: EdgeMeta::data(),
        },
        ParentRef {
            parent_id: MoteId::from_bytes([2u8; 32]),
            edge: EdgeMeta::control_non_cascading(),
        },
    ]
    .into_iter()
    .collect();

    Mote::new(
        sample_mote_def(),
        InputDataId::from_bytes([5u8; 32]),
        GraphPosition(vec![0, 1, 2]),
        parents,
    )
}

#[must_use]
pub fn sample_warrant() -> WarrantSpec {
    let mut mounts = BTreeMap::new();
    mounts.insert(PathBuf::from("/tmp/in"), FsMode::ReadOnly);
    mounts.insert(PathBuf::from("/tmp/out"), FsMode::ReadWrite);

    let mut hosts = BTreeSet::new();
    hosts.insert(Host("api.example.com:443".into()));

    let mut tool_grants = BTreeSet::new();
    tool_grants.insert(ToolGrant {
        tool_id: ToolName("fs-read".into()),
        tool_version: ToolVersion("1.2.0".into()),
    });

    WarrantSpec {
        mote_class: MoteClass::WorldMutating,
        nd_class: MoteClass::WorldMutating,
        fs_scope: FsScope { mounts },
        net_scope: NetScope::EgressAllowlist(hosts),
        syscall_profile_ref: ContentRef::from_bytes([4u8; 32]),
        tool_grants,
        model_route: ModelRoute {
            model_id: ModelId("llama-3.1-8b-instruct-q4_k_m".into()),
            max_input_tokens: 4_096,
            max_output_tokens: 512,
            max_calls: 3,
        },
        resource_ceiling: ResourceCeiling {
            cpu_milli: 1_000,
            mem_bytes: 1 << 30,
            wall_clock_ms: 30_000,
            fd_count: 64,
            disk_bytes: 1 << 28,
        },
        environment_ref: Some(ContentRef::from_bytes([8u8; 32])),
        executor_class: ExecutorClass::MacOsSandbox,
    }
}
