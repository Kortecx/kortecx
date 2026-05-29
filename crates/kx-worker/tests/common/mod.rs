//! Compact PURE fixtures for the worker e2e: a parentless PURE root + a PURE
//! child, and a warrant whose `executor_class` matches the worker the test
//! registers. Built from the real `kx-mote` / `kx-warrant` types.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::pedantic,
    dead_code,
    unreachable_pub
)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;

use kx_capability::{BrokerError, BrokerHandle, CapabilityBroker, EffectRequest};
use kx_content::ContentRef;
use kx_mote::{
    EdgeMeta, EffectPattern, GraphPosition, InferenceParams, InputDataId, LogicRef, ModelId, Mote,
    MoteDef, MoteId, NdClass, ParentRef, PromptTemplateHash, ToolName, ToolVersion,
    MOTE_DEF_SCHEMA_VERSION,
};
use kx_warrant::{
    ExecutorClass, FsMode, FsScope, Host, ModelRoute, MoteClass, NetScope, ResourceCeiling,
    WarrantSpec,
};
use smallvec::SmallVec;

/// The executor backend the worker registers as and the warrant requires.
pub const WORKER_CLASS: ExecutorClass = ExecutorClass::MacOsSandbox;

/// The capability a WORLD-MUTATING test Mote declares in its `tool_contract` (so the
/// worker's `resolve_capability` finds it). The custom test brokers below ignore the
/// per-call contract (they are not the `LocalCapabilityBroker`), so no warrant grant is
/// needed — the coordinator's `SubmitMote` runs no refusal predicates (it hosts the
/// scheduler, which only registers the Mote).
pub fn world_tool() -> ToolName {
    ToolName("kx-test-effect".into())
}

/// The version pinned alongside [`world_tool`] in a WM Mote's `tool_contract`.
pub fn world_tool_version() -> ToolVersion {
    ToolVersion("0.1.0".into())
}

fn pure_def() -> MoteDef {
    MoteDef {
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

/// A warrant a [`WORKER_CLASS`] worker can run.
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
        syscall_profile_ref: ContentRef::from_bytes([4u8; 32]),
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
        environment_ref: Some(ContentRef::from_bytes([8u8; 32])),
        executor_class: WORKER_CLASS,
    }
}

fn wm_def(pattern: EffectPattern, critic_for: Option<MoteId>) -> MoteDef {
    let mut tool_contract = BTreeMap::new();
    // A critic is non-WM (R-7) and dispatches nothing; only the WM producer needs a
    // capability in its contract for `resolve_capability` to pick.
    let nd_class = if critic_for.is_some() {
        NdClass::Pure
    } else {
        tool_contract.insert(world_tool(), world_tool_version());
        NdClass::WorldMutating
    };
    MoteDef {
        logic_ref: LogicRef::from_bytes([7u8; 32]),
        model_id: ModelId("llama-3.1-8b-instruct-q4_k_m".into()),
        prompt_template_hash: PromptTemplateHash::from_bytes([9u8; 32]),
        tool_contract,
        nd_class,
        config_subset: BTreeMap::new(),
        effect_pattern: pattern,
        critic_for,
        is_topology_shaper: false,
        inference_params: InferenceParams::default(),
        schema_version: MOTE_DEF_SCHEMA_VERSION,
    }
}

/// A WORLD-MUTATING Mote with the given effect pattern, made unique by `seed`, with
/// optional data-edge parents. Its `tool_contract` names [`world_tool`] so the worker's
/// `resolve_capability` succeeds.
#[must_use]
pub fn wm_mote(seed: u8, pattern: EffectPattern, parent_ids: &[MoteId]) -> Mote {
    Mote::new(
        wm_def(pattern, None),
        InputDataId::from_bytes([seed; 32]),
        GraphPosition(vec![seed]),
        parents(parent_ids),
    )
}

/// A `ValidateThenCommit` WORLD-MUTATING producer (D58 §6) — its sibling critic gates
/// promotion; distributed, the critic is an ordinary ready DAG Mote (see [`critic`]).
#[must_use]
pub fn vtc_producer(seed: u8, parent_ids: &[MoteId]) -> Mote {
    wm_mote(seed, EffectPattern::ValidateThenCommit, parent_ids)
}

/// The PURE critic for `producer` (R-7: a critic is non-WM). It carries `producer` as a
/// data-edge parent so the projection's `ready_set` only offers it once the producer is
/// `Committed` — distributed, the coordinator schedules it by dependency (the worker has
/// no scheduler authority, D58 §6).
#[must_use]
pub fn critic(seed: u8, producer: MoteId) -> Mote {
    Mote::new(
        wm_def(EffectPattern::IdempotentByConstruction, Some(producer)),
        InputDataId::from_bytes([seed; 32]),
        GraphPosition(vec![seed]),
        parents(&[producer]),
    )
}

fn parents(parent_ids: &[MoteId]) -> SmallVec<[ParentRef; 4]> {
    parent_ids
        .iter()
        .map(|id| ParentRef {
            parent_id: *id,
            edge: EdgeMeta::data(),
        })
        .collect()
}

/// A warrant a [`WORKER_CLASS`] worker can run a WORLD-MUTATING Mote under. Same as
/// [`pure_warrant`] but with a WORLD-MUTATING class (the custom test brokers skip the
/// per-call grant check, so `tool_grants` stays empty).
#[must_use]
pub fn wm_warrant() -> WarrantSpec {
    WarrantSpec {
        mote_class: MoteClass::WorldMutating,
        nd_class: MoteClass::WorldMutating,
        ..pure_warrant()
    }
}

/// A broker that never dispatches — for PURE-only workers (the WM path is never taken,
/// so this is never invoked). Panics if it ever is, surfacing a wiring mistake.
#[must_use]
pub fn noop_broker() -> Arc<dyn CapabilityBroker> {
    struct NoopBroker;
    impl CapabilityBroker for NoopBroker {
        fn dispatch(
            &self,
            _mote: &Mote,
            _warrant: &WarrantSpec,
            _capability: &ToolName,
            _request: EffectRequest,
        ) -> Result<BrokerHandle, BrokerError> {
            panic!("a PURE-only worker must never dispatch through the broker");
        }
        fn probe_readback(
            &self,
            _mote: &Mote,
            _warrant: &WarrantSpec,
            _capability: &ToolName,
            _probe: EffectRequest,
        ) -> Result<Option<BrokerHandle>, BrokerError> {
            panic!("a PURE-only worker must never probe through the broker");
        }
    }
    Arc::new(NoopBroker)
}
