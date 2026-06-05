//! Compact PURE fixtures for the gateway e2e: a parentless PURE Mote + a warrant
//! whose `executor_class` matches the embedded worker the server registers
//! (`kx_gateway::default_executor_class()`), so the worker leases it. Built from
//! the real `kx-mote` / `kx-warrant` types (mirrors `kx-worker/tests/common`).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::pedantic,
    dead_code,
    unreachable_pub
)]

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;

use kx_gateway::GatewayConfig;
use kx_mote::{
    EdgeMeta, EffectPattern, GraphPosition, InferenceParams, InputDataId, LogicRef, ModelId, Mote,
    MoteDef, MoteId, NdClass, ParentRef, PromptTemplateHash, MOTE_DEF_SCHEMA_VERSION,
};
use kx_warrant::{
    FsMode, FsScope, Host, ModelRoute, MoteClass, NetScope, ResourceCeiling, WarrantSpec,
};
use smallvec::SmallVec;
use tempfile::TempDir;

/// A gateway config rooted at `dir` (ephemeral journal + content + catalog).
/// `dev_allow_local` installs the dev resolver; a non-empty `auth_tokens`
/// (token → party) installs the bearer-token resolver instead.
#[must_use]
pub fn gateway_config(
    dir: &TempDir,
    dev_allow_local: bool,
    auth_tokens: HashMap<String, String>,
) -> GatewayConfig {
    GatewayConfig {
        listen: "127.0.0.1:0".parse().unwrap(),
        journal_path: dir.path().join("kx.db"),
        content_root: dir.path().join("blobs"),
        max_lease: 16,
        dev_allow_local,
        auth_tokens,
        catalog_dir: None,
    }
}

fn pure_def() -> MoteDef {
    MoteDef {
        critic_check: None,
        logic_ref: LogicRef::from_bytes([7u8; 32]),
        model_id: ModelId("llama-3.1-8b-instruct-q4_k_m".into()),
        prompt_template_hash: PromptTemplateHash::from_bytes([9u8; 32]),
        tool_contract: BTreeMap::new(),
        nd_class: NdClass::Pure,
        config_subset: BTreeMap::new(),
        effect_pattern: EffectPattern::IdempotentByConstruction,
        critic_for: None,
        is_topology_shaper: false,
        inference_params: InferenceParams::default(),
        schema_version: MOTE_DEF_SCHEMA_VERSION,
    }
}

/// A PURE Mote, made unique by `seed`, with optional data-edge parents.
#[must_use]
pub fn pure_mote(seed: u8, parent_ids: &[MoteId]) -> Mote {
    let parents: SmallVec<[ParentRef; 4]> = parent_ids
        .iter()
        .map(|id| ParentRef {
            parent_id: *id,
            edge: EdgeMeta::data(),
        })
        .collect();
    Mote::new(
        pure_def(),
        InputDataId::from_bytes([seed; 32]),
        GraphPosition(vec![seed]),
        parents,
    )
}

/// A warrant the embedded gateway worker can run: its `executor_class` is the
/// server's `default_executor_class()`, so the in-process worker leases it.
#[must_use]
pub fn pure_warrant() -> WarrantSpec {
    let mut mounts = BTreeMap::new();
    mounts.insert(PathBuf::from("/tmp/in"), FsMode::ReadOnly);

    WarrantSpec {
        mote_class: MoteClass::Pure,
        nd_class: MoteClass::Pure,
        fs_scope: FsScope { mounts },
        net_scope: NetScope::EgressAllowlist({
            let mut h = BTreeSet::new();
            h.insert(Host("api.example.com:443".into()));
            h
        }),
        syscall_profile_ref: kx_content::ContentRef::from_bytes([4u8; 32]),
        tool_grants: BTreeSet::new(),
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
        environment_ref: Some(kx_content::ContentRef::from_bytes([8u8; 32])),
        executor_class: kx_gateway::default_executor_class(),
        ..Default::default()
    }
}
