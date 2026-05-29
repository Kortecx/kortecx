//! Workflow fixtures the scenarios submit: parameterized PURE / WORLD-MUTATING /
//! topology-shaper Motes and the warrants that admit them, built from the real
//! `kx-mote` / `kx-warrant` types. Identities are seed-salted so a run's Motes are
//! visibly distinct; each seed has its own coordinator + journal, so cross-seed id
//! reuse is harmless.
//!
//! The topology helpers wrap `kx-runtime`'s real `derive_child_motes`, so the shaper
//! scenario checks the *production* child-identity derivation for determinism under a
//! shaper death — not a re-implementation.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use kx_content::ContentRef;
use kx_mote::{
    EdgeMeta, EffectPattern, GraphPosition, InferenceParams, InputDataId, LogicRef, ModelId, Mote,
    MoteDef, MoteId, NdClass, ParentRef, PromptTemplateHash, ToolName, ToolVersion,
    TopologyDecision, MOTE_DEF_SCHEMA_VERSION,
};
use kx_runtime::topology::{demo_topology_decision, derive_child_motes, encode_topology_decision};
use kx_warrant::{
    ExecutorClass, FsMode, FsScope, Host, ModelRoute, MoteClass, NetScope, ResourceCeiling,
    WarrantSpec,
};
use smallvec::SmallVec;

use crate::plan::WmPattern;

/// The executor backend the harness's workers register as and the warrants require.
pub(crate) const WORKER_CLASS: ExecutorClass = ExecutorClass::MacOsSandbox;

/// The capability a WORLD-MUTATING Mote names in its `tool_contract`. The chaos broker
/// ignores the contract (it is not the real `LocalCapabilityBroker`), and the
/// coordinator's `SubmitMote` runs no refusal predicates, so no warrant grant is needed.
pub(crate) fn world_tool() -> ToolName {
    ToolName("kx-chaos-effect".into())
}

/// The version pinned beside [`world_tool`].
pub(crate) fn world_tool_version() -> ToolVersion {
    ToolVersion("0.1.0".into())
}

/// Map the plan's [`WmPattern`] onto the domain effect pattern.
pub(crate) fn effect_pattern_of(p: WmPattern) -> EffectPattern {
    match p {
        WmPattern::StageThenCommit => EffectPattern::StageThenCommit,
        WmPattern::ValidateThenCommit => EffectPattern::ValidateThenCommit,
        WmPattern::IdempotentByConstruction => EffectPattern::IdempotentByConstruction,
    }
}

/// A distinct 32-byte identity base from `(salt, index)`.
fn id_bytes(salt: u8, index: u8) -> [u8; 32] {
    let mut b = [0u8; 32];
    b[0] = salt;
    b[1] = index;
    b
}

fn parents_of(parent_ids: &[MoteId]) -> SmallVec<[ParentRef; 4]> {
    parent_ids
        .iter()
        .map(|id| ParentRef {
            parent_id: *id,
            edge: EdgeMeta::data(),
        })
        .collect()
}

fn base_def(nd_class: NdClass, effect_pattern: EffectPattern) -> MoteDef {
    MoteDef {
        logic_ref: LogicRef::from_bytes([7u8; 32]),
        model_id: ModelId("llama-3.1-8b-instruct-q4_k_m".into()),
        prompt_template_hash: PromptTemplateHash::from_bytes([9u8; 32]),
        tool_contract: BTreeMap::new(),
        nd_class,
        config_subset: BTreeMap::new(),
        effect_pattern,
        critic_for: None,
        is_topology_shaper: false,
        inference_params: InferenceParams::default(),
        schema_version: MOTE_DEF_SCHEMA_VERSION,
    }
}

/// A PURE Mote, made unique by `(salt, index)`, with optional data-edge parents.
pub(crate) fn pure_mote(salt: u8, index: u8, parent_ids: &[MoteId]) -> Mote {
    Mote::new(
        base_def(NdClass::Pure, EffectPattern::IdempotentByConstruction),
        InputDataId::from_bytes(id_bytes(salt, index)),
        GraphPosition(vec![salt, index]),
        parents_of(parent_ids),
    )
}

/// A WORLD-MUTATING Mote with the given effect pattern, made unique by `(salt, index)`.
/// Its `tool_contract` names [`world_tool`] (faithful shape; the broker ignores it).
pub(crate) fn wm_mote(salt: u8, index: u8, pattern: EffectPattern) -> Mote {
    let mut def = base_def(NdClass::WorldMutating, pattern);
    def.tool_contract.insert(world_tool(), world_tool_version());
    Mote::new(
        def,
        InputDataId::from_bytes(id_bytes(salt, index)),
        GraphPosition(vec![salt, index]),
        SmallVec::new(),
    )
}

/// The topology shaper: READ-ONLY-NONDET, `is_topology_shaper`, made unique by `salt`.
/// Its committed decision spawns `DEMO_WORKER_COUNT` PURE children.
pub(crate) fn shaper_mote(salt: u8) -> Mote {
    let mut def = base_def(
        NdClass::ReadOnlyNondet,
        EffectPattern::IdempotentByConstruction,
    );
    def.is_topology_shaper = true;
    Mote::new(
        def,
        InputDataId::from_bytes(id_bytes(salt, 0xF0)),
        GraphPosition(vec![salt, 0xF0]),
        SmallVec::new(),
    )
}

/// The shaper's topology decision (`DEMO_WORKER_COUNT` PURE workers).
pub(crate) fn topology_decision() -> TopologyDecision {
    demo_topology_decision()
}

/// The deterministic `result_ref` the shaper commits — content-addressed on the encoded
/// decision, so any worker (the original or a post-death replacement) commits the *same*
/// ref. Falls back to a fixed ref if encoding ever fails (it cannot for this decision).
pub(crate) fn shaper_result_ref(td: &TopologyDecision) -> ContentRef {
    match encode_topology_decision(td) {
        Ok(bytes) => ContentRef::from_bytes(*blake3::hash(&bytes).as_bytes()),
        Err(_) => ContentRef::from_bytes([0xEE; 32]),
    }
}

/// Re-derive the shaper's child Motes from its committed `result_ref` + decision via the
/// **production** `kx_runtime::topology::derive_child_motes`. A pure function of the
/// committed decision: a death-and-replay of the shaper cannot fork this set.
pub(crate) fn derive_children(shaper: &Mote, td: &TopologyDecision) -> Vec<Mote> {
    let child_warrant = pure_warrant();
    let capability = world_tool();
    derive_child_motes(
        shaper,
        shaper_result_ref(td),
        td,
        &child_warrant,
        &capability,
    )
    .into_iter()
    .map(|wm| wm.mote)
    .collect()
}

/// A deterministic `result_ref` for a non-world-mutating Mote — content-addressed on
/// its id, so any worker (original or replacement) proposes the identical ref and the
/// journal's dedup-by-key collapses a racing double-commit to one fact.
pub(crate) fn pure_result_ref(mote: &Mote) -> ContentRef {
    let mut tagged = b"kx-chaos-result:".to_vec();
    tagged.extend_from_slice(mote.id.as_bytes());
    ContentRef::from_bytes(*blake3::hash(&tagged).as_bytes())
}

/// A warrant a [`WORKER_CLASS`] worker can run a PURE Mote under.
pub(crate) fn pure_warrant() -> WarrantSpec {
    let mut mounts = BTreeMap::new();
    mounts.insert(PathBuf::from("/tmp/in"), FsMode::ReadOnly);
    let mut hosts = BTreeSet::new();
    hosts.insert(Host("api.example.com:443".into()));
    WarrantSpec {
        mote_class: MoteClass::Pure,
        nd_class: MoteClass::Pure,
        fs_scope: FsScope { mounts },
        net_scope: NetScope::EgressAllowlist(hosts),
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

/// A warrant a [`WORKER_CLASS`] worker can run a WORLD-MUTATING Mote under.
pub(crate) fn wm_warrant() -> WarrantSpec {
    WarrantSpec {
        mote_class: MoteClass::WorldMutating,
        nd_class: MoteClass::WorldMutating,
        ..pure_warrant()
    }
}
