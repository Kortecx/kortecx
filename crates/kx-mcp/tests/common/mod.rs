//! Shared fixtures for the kx-mcp integration tests: a Mote that declares the MCP
//! tool in its `tool_contract`, a warrant that grants it (net_scope = None, fs
//! empty — the stdio transport needs no egress), and an `EffectRequest` builder.

#![allow(
    dead_code,
    unreachable_pub,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::doc_markdown
)]

use std::collections::{BTreeMap, BTreeSet};

use kx_capability::EffectRequest;
use kx_mcp::{McpCapability, StdioTransport};
use kx_mote::{
    EffectPattern, GraphPosition, InferenceParams, InputDataId, LogicRef, ModelId, Mote, MoteDef,
    NdClass, PromptTemplateHash, ToolName, ToolVersion, MOTE_DEF_SCHEMA_VERSION,
};
use kx_tool_registry::McpEndpointId;
use kx_warrant::{
    ExecutorClass, FsScope, ModelRoute, MoteClass, NetScope, ResourceCeiling, ToolGrant,
    WarrantSpec,
};

/// An `McpCapability` over the bundled stdio echo server (default mode).
#[must_use]
pub fn echo_capability(name: ToolName, version: ToolVersion) -> McpCapability {
    McpCapability::new(
        name,
        version,
        McpEndpointId("stdio://mock".into()),
        "echo",
        Box::new(StdioTransport::new(MOCK_SERVER)),
    )
}

/// Absolute path to the bundled test stdio MCP server (set by Cargo for this crate's
/// integration tests because the server is one of this crate's `[[bin]]` targets).
pub const MOCK_SERVER: &str = env!("CARGO_BIN_EXE_kx-mcp-mock-stdio");

/// The MCP tool the fixtures use: `mcp-echo@1`.
#[must_use]
pub fn tool() -> (ToolName, ToolVersion) {
    (ToolName("mcp-echo".into()), ToolVersion("1".into()))
}

/// A WorldMutating `StageThenCommit` Mote that declares `(tool_name, tool_version)`
/// in its `tool_contract` (so `LocalCapabilityBroker::precheck` admits the call).
#[must_use]
pub fn sample_mote(tool_name: &ToolName, tool_version: &ToolVersion) -> Mote {
    let mut tool_contract = BTreeMap::new();
    tool_contract.insert(tool_name.clone(), tool_version.clone());
    let def = MoteDef {
        critic_check: None,
        logic_ref: LogicRef::from_bytes([7; 32]),
        model_id: ModelId("m".into()),
        prompt_template_hash: PromptTemplateHash::from_bytes([7; 32]),
        tool_contract,
        nd_class: NdClass::WorldMutating,
        config_subset: BTreeMap::new(),
        effect_pattern: EffectPattern::StageThenCommit,
        critic_for: None,
        is_topology_shaper: false,
        inference_params: InferenceParams::default(),
        schema_version: MOTE_DEF_SCHEMA_VERSION,
    };
    Mote::new(
        def,
        InputDataId::from_bytes([7; 32]),
        GraphPosition(vec![7]),
        smallvec::SmallVec::new(),
    )
}

/// A warrant granting exactly `(tool_name, tool_version)`, no egress, no fs.
#[must_use]
pub fn warrant_granting(tool_name: &ToolName, tool_version: &ToolVersion) -> WarrantSpec {
    let mut tool_grants = BTreeSet::new();
    tool_grants.insert(ToolGrant {
        tool_id: tool_name.clone(),
        tool_version: tool_version.clone(),
    });
    WarrantSpec {
        mote_class: MoteClass::WorldMutating,
        nd_class: MoteClass::WorldMutating,
        fs_scope: FsScope::empty(),
        net_scope: NetScope::None,
        syscall_profile_ref: kx_content::ContentRef::from_bytes([0; 32]),
        tool_grants,
        model_route: ModelRoute {
            model_id: ModelId("m".into()),
            max_input_tokens: 1024,
            max_output_tokens: 256,
            max_calls: 8,
        },
        resource_ceiling: ResourceCeiling {
            cpu_milli: 0,
            mem_bytes: 0,
            wall_clock_ms: 5_000,
            fd_count: 0,
            disk_bytes: 0,
        },
        environment_ref: None,
        executor_class: ExecutorClass::Bwrap,
    }
}

/// An `EffectRequest` carrying `args_json` (the tool arguments) under
/// `StageThenCommit` with no egress / fs.
#[must_use]
pub fn effect(args_json: &str) -> EffectRequest {
    EffectRequest {
        payload: args_json.as_bytes().to_vec(),
        pattern: EffectPattern::StageThenCommit,
        idempotency_key: None,
        net_scope: NetScope::None,
        fs_scope: FsScope::empty(),
    }
}
