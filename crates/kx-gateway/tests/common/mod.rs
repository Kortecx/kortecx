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
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use kx_gateway::{ConsoleMode, GatewayConfig};
use kx_mote::{
    EdgeMeta, EffectPattern, GraphPosition, InferenceParams, InputDataId, LogicRef, ModelId, Mote,
    MoteDef, MoteId, NdClass, ParentRef, PromptTemplateHash, MOTE_DEF_SCHEMA_VERSION,
};
use kx_proto::proto;
use kx_proto::proto::kx_gateway_client::KxGatewayClient;
use kx_warrant::{
    FsMode, FsScope, Host, ModelRoute, MoteClass, NetScope, ResourceCeiling, WarrantSpec,
};
use smallvec::SmallVec;
use tempfile::TempDir;
use tonic::transport::Channel;

/// Connect a gRPC client to `addr`, retrying briefly while the server binds.
pub async fn connect_client(addr: SocketAddr) -> KxGatewayClient<Channel> {
    let endpoint = format!("http://{addr}");
    for _ in 0..100 {
        if let Ok(c) = KxGatewayClient::connect(endpoint.clone()).await {
            return c;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("client connects to the gateway at {endpoint}");
}

/// `SubmitRun` one parentless PURE demo Mote (`seed`); return its journaled
/// 16-byte `instance_id`.
pub async fn submit_pure_run(client: &mut KxGatewayClient<Channel>, seed: u8) -> Vec<u8> {
    client
        .submit_run(proto::SubmitRunRequest {
            recipe_fingerprint: vec![0x5a; 32],
            motes: vec![proto::SubmitMoteSpec {
                mote: Some(pure_mote(seed, &[]).into()),
                warrant: Some(pure_warrant().into()),
                accept_at_least_once: false,
                react_seed: false,
            }],
        })
        .await
        .unwrap()
        .into_inner()
        .instance_id
}

/// Poll `GetProjection` until the run has a `Committed` Mote; return its
/// `(mote_id, result_ref)`. Panics on timeout.
pub async fn await_committed(
    client: &mut KxGatewayClient<Channel>,
    instance_id: &[u8],
) -> ([u8; 32], [u8; 32]) {
    for _ in 0..200 {
        let view = client
            .get_projection(proto::GetProjectionRequest {
                instance_id: instance_id.to_vec(),
                at_seq: None,
            })
            .await
            .unwrap()
            .into_inner();
        if let Some(m) = view
            .motes
            .iter()
            .find(|m| m.state == proto::MoteSnapshotState::Committed as i32)
        {
            let mote_id: [u8; 32] = m.mote_id.clone().try_into().unwrap();
            let result_ref: [u8; 32] = m
                .result_ref
                .clone()
                .expect("a committed Mote carries a result_ref")
                .try_into()
                .unwrap();
            return (mote_id, result_ref);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the submitted Mote never reached Committed");
}

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
        ws_listen: "127.0.0.1:0".parse().unwrap(),
        journal_path: dir.path().join("kx.db"),
        content_root: dir.path().join("blobs"),
        max_lease: 16,
        dev_allow_local,
        auth_tokens,
        catalog_dir: None,
        tls: None,
        cors_origins: Vec::new(),
        console_listen: ConsoleMode::Disabled,
        content_max_bytes: kx_gateway::DEFAULT_CONTENT_MAX_BYTES,
        metrics_listen: None,
        audit_log: None,
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
