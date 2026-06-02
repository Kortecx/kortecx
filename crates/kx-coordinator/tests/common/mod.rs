//! Shared fixtures for the coordinator integration tests: parameterized Motes
//! (nd_class + data-edge parents + a per-seed unique identity), a representative
//! warrant, and a faithful `ReportCommit` request built from a Mote. Built from
//! the real `kx-mote` / `kx-warrant` types so the proto<->domain mapping is
//! exercised against genuine inputs.

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
use kx_coordinator::proto;
use kx_coordinator::proto::coordinator_server::Coordinator;
use kx_coordinator::CoordinatorService;
use tonic::{Request, Status};

use kx_mote::{
    ConfigKey, ConfigVal, EdgeMeta, EffectPattern, GraphPosition, InferenceParams, InputDataId,
    LogicRef, ModelId, Mote, MoteDef, MoteId, NdClass, ParentRef, PromptTemplateHash, ToolName,
    ToolVersion, MOTE_DEF_SCHEMA_VERSION,
};
use kx_warrant::{
    ExecutorClass, FsMode, FsScope, Host, ModelRoute, MoteClass, NetScope, ResourceCeiling,
    ToolGrant, WarrantSpec,
};
use smallvec::SmallVec;

/// The executor class registered workers run under in the stress/distributed
/// harnesses. `WorkerClient::register_worker` takes the domain `ExecutorClass`
/// (it converts to the proto enum internally); this matches the local Apple-
/// Silicon sandbox class used by the `lease_work` / `reschedule` tests.
pub const WORKER_CLASS: ExecutorClass = ExecutorClass::MacOsSandbox;

#[must_use]
pub fn sample_inference_params() -> InferenceParams {
    InferenceParams {
        max_output_tokens: 256,
        temperature_bps: 0,
        top_p_bps: 9_000,
        top_k: 40,
        seed: 12_345,
        stop_tokens: SmallVec::new(),
        grammar: None,
    }
}

/// A `MoteDef` for the given nd_class. The effect pattern follows the class
/// (WORLD-MUTATING stages-then-commits; PURE / READ-ONLY-NONDET are idempotent).
#[must_use]
pub fn mote_def(nd_class: NdClass) -> MoteDef {
    let effect_pattern = match nd_class {
        NdClass::WorldMutating => EffectPattern::StageThenCommit,
        NdClass::Pure | NdClass::ReadOnlyNondet => EffectPattern::IdempotentByConstruction,
    };
    MoteDef {
        critic_check: None,
        logic_ref: LogicRef::from_bytes([7u8; 32]),
        model_id: ModelId("llama-3.1-8b-instruct-q4_k_m".into()),
        prompt_template_hash: PromptTemplateHash::from_bytes([9u8; 32]),
        tool_contract: BTreeMap::new(),
        nd_class,
        config_subset: {
            let mut c = BTreeMap::new();
            c.insert(ConfigKey("max_depth".into()), ConfigVal(vec![3]));
            c
        },
        effect_pattern,
        critic_for: None,
        is_topology_shaper: false,
        inference_params: sample_inference_params(),
        schema_version: MOTE_DEF_SCHEMA_VERSION,
    }
}

/// Build a Mote with the given nd_class and data-edge parents, made unique by
/// `seed` (distinct `graph_position` + `input_data_id`). The id is derived by
/// `Mote::new` from `def + input_data_id + graph_position`.
#[must_use]
pub fn mote(seed: u8, nd_class: NdClass, parent_ids: &[MoteId]) -> Mote {
    let parents: SmallVec<[ParentRef; 4]> = parent_ids
        .iter()
        .map(|id| ParentRef {
            parent_id: *id,
            edge: EdgeMeta::data(),
        })
        .collect();
    Mote::new(
        mote_def(nd_class),
        InputDataId::from_bytes([seed; 32]),
        GraphPosition(vec![seed]),
        parents,
    )
}

/// A parentless WORLD-MUTATING Mote with an explicit effect pattern (so a test can build a
/// `ValidateThenCommit` / `IdempotentByConstruction` producer that — per D58 — never writes
/// `EffectStaged`). Made unique by `seed`.
#[must_use]
pub fn wm_mote(seed: u8, effect_pattern: EffectPattern) -> Mote {
    let mut def = mote_def(NdClass::WorldMutating);
    def.effect_pattern = effect_pattern;
    // M1.3 R-1: a WORLD-MUTATING `IdempotentByConstruction` Mote with an EMPTY
    // tool_contract is refused at submit (no idempotency-supporting tool to dedup
    // against). Declare the `fs-write` builtin so every `wm_mote` — regardless of
    // pattern — passes the R-1 structural check. (R-10/D66 resolve the WARRANT's
    // grants, not this contract, so this does not affect those predicates.)
    def.tool_contract
        .insert(ToolName("fs-write".into()), ToolVersion("1".into()));
    Mote::new(
        def,
        InputDataId::from_bytes([seed; 32]),
        GraphPosition(vec![seed]),
        SmallVec::new(),
    )
}

/// A WORLD-MUTATING `StageThenCommit` Mote whose `tool_contract` names exactly
/// the given `(tool, version)` — used to exercise the class-aware recovery gate
/// (M2.3b). `StageThenCommit` is not subject to R-1 (which is IBC-only), so the
/// contract may name an `AtLeastOnce` tool without an additional idempotent tool.
#[must_use]
pub fn wm_mote_with_tool(seed: u8, tool: &str, version: &str) -> Mote {
    let mut def = mote_def(NdClass::WorldMutating);
    def.effect_pattern = EffectPattern::StageThenCommit;
    def.tool_contract = BTreeMap::new();
    def.tool_contract
        .insert(ToolName(tool.into()), ToolVersion(version.into()));
    Mote::new(
        def,
        InputDataId::from_bytes([seed; 32]),
        GraphPosition(vec![seed]),
        SmallVec::new(),
    )
}

/// The canonical parentless PURE root Mote.
#[must_use]
pub fn pure_root_mote() -> Mote {
    mote(0, NdClass::Pure, &[])
}

/// A parentless Mote uniquely identified by a `u64` index (for load tests that
/// need more than 256 distinct Motes). Encodes the index into both
/// `input_data_id` and `graph_position` so each id is distinct.
#[must_use]
pub fn mote_indexed(index: u64, nd_class: NdClass) -> Mote {
    let mut input = [0u8; 32];
    input[..8].copy_from_slice(&index.to_le_bytes());
    Mote::new(
        mote_def(nd_class),
        InputDataId::from_bytes(input),
        GraphPosition(index.to_le_bytes().to_vec()),
        SmallVec::new(),
    )
}

/// A valid warrant that round-trips through the proto schema.
#[must_use]
pub fn sample_warrant() -> WarrantSpec {
    let mut mounts = BTreeMap::new();
    mounts.insert(PathBuf::from("/tmp/in"), FsMode::ReadOnly);

    let mut hosts = BTreeSet::new();
    hosts.insert(Host("api.example.com:443".into()));

    // M1.3: the grant must RESOLVE against the OSS built-ins (`fs-read@1`,
    // IdempotencyClass::Readback) so a WORLD-MUTATING Mote submitted under this
    // warrant passes the D66 fail-closed gate (a resolvable, non-AtLeastOnce
    // tool). `fs-read@1` requires `syscall_profile_ref == [0; 32]` (exact match),
    // so the warrant carries that value.
    let mut tool_grants = BTreeSet::new();
    tool_grants.insert(ToolGrant {
        tool_id: ToolName("fs-read".into()),
        tool_version: ToolVersion("1".into()),
    });

    WarrantSpec {
        mote_class: MoteClass::Pure,
        nd_class: MoteClass::Pure,
        fs_scope: FsScope { mounts },
        net_scope: NetScope::EgressAllowlist(hosts),
        syscall_profile_ref: ContentRef::from_bytes([0u8; 32]),
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

/// A faithful `ReportCommit` request proposing the commit of `mote` by `worker`.
/// `idempotency_key == mote_id` (the identity invariant); `nd_class` and `parents`
/// mirror the Mote; `result_ref` is an arbitrary 32-byte value (P2.2 does not
/// re-validate it against content).
#[must_use]
pub fn report_commit_request(mote: &Mote, worker_id: u64) -> proto::ReportCommitRequest {
    let id = mote.id.as_bytes().to_vec();
    proto::ReportCommitRequest {
        mote_id: id.clone(),
        idempotency_key: id,
        result_ref: vec![3u8; 32],
        warrant_ref: vec![4u8; 32],
        mote_def_hash: vec![5u8; 32],
        nd_class: proto::NdClass::from(mote.def.nd_class) as i32,
        parents: mote.parents.iter().cloned().map(Into::into).collect(),
        worker_id,
    }
}

/// A `RegisterWorker` request with a macOS-sandbox executor class.
#[must_use]
pub fn register_request(endpoint: &str) -> proto::RegisterWorkerRequest {
    proto::RegisterWorkerRequest {
        executor_class: proto::ExecutorClass::MacosSandbox as i32,
        endpoint: endpoint.into(),
    }
}

// --- direct-call helpers (in-process, no network) for happy-path test setup ---

/// Register a worker through the service; returns its assigned id.
pub async fn register(service: &CoordinatorService, endpoint: &str) -> u64 {
    service
        .register_worker(Request::new(register_request(endpoint)))
        .await
        .unwrap()
        .into_inner()
        .worker_id
}

/// Send a heartbeat for `worker_id` through the service (liveness tests).
pub async fn heartbeat(
    service: &CoordinatorService,
    worker_id: u64,
    timestamp_ms: u64,
    in_flight: u32,
) -> bool {
    service
        .heartbeat(Request::new(proto::HeartbeatRequest {
            worker_id,
            timestamp_ms,
            in_flight,
        }))
        .await
        .unwrap()
        .into_inner()
        .ack
}

/// A fixed recipe fingerprint for the test helpers' auto-registration. The
/// fingerprint is discovery-only metadata (never identity, D64/D79), so a
/// constant is fine — the per-run `instance_id` still comes from the service's
/// nonce source (random under `new`, fixed under the seam constructors).
pub const TEST_RECIPE_FINGERPRINT: [u8; 32] = [0x5au8; 32];

/// Register the run through the service (M1.1/D64), returning its `instance_id`.
/// Idempotent: the first call writes the seq=1 `RunRegistered` fact; a later call
/// returns the existing id and writes nothing. Call once before submitting (the
/// M1.3 registration-before-submit gate) — or rely on [`submit`], which does.
pub async fn register_run(service: &CoordinatorService, fingerprint: [u8; 32]) -> Vec<u8> {
    service
        .register_run(Request::new(proto::RegisterRunRequest {
            recipe_fingerprint: fingerprint.to_vec(),
        }))
        .await
        .unwrap()
        .into_inner()
        .instance_id
}

/// Submit a Mote + warrant through the service, **auto-registering the run first**
/// (M1.3 forces registration-before-submit). The registration is idempotent, so a
/// test that submits many Motes registers exactly once (seq=1). `accept_at_least_once`
/// defaults `false` — an AtLeastOnce WM tool is fail-closed; use [`submit_accepting`]
/// to opt in. Tests that exercise the un-registered path use [`submit_unregistered`].
pub async fn submit(
    service: &CoordinatorService,
    mote: &Mote,
    warrant: &WarrantSpec,
) -> proto::SubmitMoteResponse {
    let _ = register_run(service, TEST_RECIPE_FINGERPRINT).await;
    service
        .submit_mote(Request::new(proto::SubmitMoteRequest {
            mote: Some(mote.clone().into()),
            warrant: Some(warrant.clone().into()),
            accept_at_least_once: false,
        }))
        .await
        .unwrap()
        .into_inner()
}

/// As [`submit`], but sets `accept_at_least_once = true` (M1.3/D38 §2c — the
/// operator opt-in to dispatch a resolved `AtLeastOnce` WORLD-MUTATING tool).
pub async fn submit_accepting(
    service: &CoordinatorService,
    mote: &Mote,
    warrant: &WarrantSpec,
) -> proto::SubmitMoteResponse {
    let _ = register_run(service, TEST_RECIPE_FINGERPRINT).await;
    service
        .submit_mote(Request::new(proto::SubmitMoteRequest {
            mote: Some(mote.clone().into()),
            warrant: Some(warrant.clone().into()),
            accept_at_least_once: true,
        }))
        .await
        .unwrap()
        .into_inner()
}

/// Submit a Mote + warrant through the service WITHOUT registering the run first
/// and WITHOUT unwrapping — for tests that exercise the M1.3
/// registration-before-submit gate (an un-registered submit is refused with
/// `failed_precondition`). Returns the raw `Result` so the caller asserts the
/// `tonic::Status` (or the REJECTED response).
pub async fn submit_unregistered(
    service: &CoordinatorService,
    mote: &Mote,
    warrant: &WarrantSpec,
) -> Result<proto::SubmitMoteResponse, Status> {
    service
        .submit_mote(Request::new(proto::SubmitMoteRequest {
            mote: Some(mote.clone().into()),
            warrant: Some(warrant.clone().into()),
            accept_at_least_once: false,
        }))
        .await
        .map(tonic::Response::into_inner)
}

/// Report a commit for `mote` by `worker_id` through the service.
pub async fn commit(
    service: &CoordinatorService,
    mote: &Mote,
    worker_id: u64,
) -> proto::ReportCommitResponse {
    service
        .report_commit(Request::new(report_commit_request(mote, worker_id)))
        .await
        .unwrap()
        .into_inner()
}

/// Record a WORLD-MUTATING Mote's staged-intent (`ReportEffectStaged`) for `worker_id` through
/// the service — the durable `EffectStaged` hint the recovery oracle keys on. `idempotency_key
/// == mote_id` (identity invariant). Returns the staged seq.
pub async fn report_effect_staged(
    service: &CoordinatorService,
    mote: &Mote,
    worker_id: u64,
) -> u64 {
    let id = mote.id.as_bytes().to_vec();
    service
        .report_effect_staged(Request::new(proto::ReportEffectStagedRequest {
            mote_id: id.clone(),
            idempotency_key: id,
            worker_id,
        }))
        .await
        .unwrap()
        .into_inner()
        .staged_seq
}

/// Lease ready PURE work for `worker_id` on `executor_class` through the service.
pub async fn lease_work(
    service: &CoordinatorService,
    worker_id: u64,
    executor_class: proto::ExecutorClass,
    max_motes: u32,
) -> Vec<proto::WorkItem> {
    service
        .lease_work(Request::new(proto::LeaseWorkRequest {
            worker_id,
            executor_class: executor_class as i32,
            max_motes,
        }))
        .await
        .unwrap()
        .into_inner()
        .items
}

/// Read committed journal entries after `since_seq` through the service.
pub async fn read_entries(
    service: &CoordinatorService,
    since_seq: u64,
    max: u32,
) -> proto::ReadEntriesResponse {
    service
        .read_entries(Request::new(proto::ReadEntriesRequest { since_seq, max }))
        .await
        .unwrap()
        .into_inner()
}

/// The `mote_id` + `result_ref` of a committed entry (test convenience).
#[must_use]
pub fn committed_view(entry: &proto::JournalEntry) -> (Vec<u8>, Vec<u8>, u64) {
    match entry.kind.as_ref().unwrap() {
        proto::journal_entry::Kind::Committed(c) => {
            (c.mote_id.clone(), c.result_ref.clone(), c.seq)
        }
    }
}
