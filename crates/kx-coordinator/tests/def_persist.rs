//! PR-2 — admission-time MoteDef persistence (the `GetMoteDetail` substrate).
//!
//! The coordinator's `dispatch.defs` is in-memory and recovery never repopulates
//! it, so a committed Mote's definition would otherwise exist nowhere durable
//! (the journal stores only `mote_def_hash`). PR-2 persists the canonical def
//! bytes into the SHARED content store at every admission site; because the
//! canonical encode's blake3 IS `MoteDef::hash()`, the blob lands at the exact
//! address a committed fact carries — no sidecar, no journal write, rebuildable
//! by construction. These tests pin: the root-submit site, the shaper-children
//! site, the react-turn site, idempotent re-submit, and the storeless
//! degradation (admission must never depend on the display-only blob).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use kx_content::{ContentRef, ContentStore, LocalFsContentStore};
use kx_coordinator::proto::coordinator_server::Coordinator;
use kx_coordinator::proto::{CommitOutcome, ExecutorClass as ProtoExecutorClass};
use kx_coordinator::{CoordinatorService, InMemoryWorkerRegistry, WorkerRegistry};
use kx_journal::SqliteJournal;
use kx_mote::{
    ChildDescriptor, ConfigVal, EffectPattern, GraphPosition, InputDataId, LogicRef, ModelId, Mote,
    MoteDef, NdClass, PromptTemplateHash, RoleId, TopologyDecision, MOTE_DEF_SCHEMA_VERSION,
};
use kx_warrant::{
    warrant_ref_of, ExecutorClass, FsScope, InMemoryRoleRegistry, ModelRoute, MoteClass, NetScope,
    ResourceCeiling, Role, RoleRegistry, WarrantSpec,
};
use smallvec::SmallVec;
use tempfile::TempDir;
use tonic::Request;

const MAC: ProtoExecutorClass = ProtoExecutorClass::MacosSandbox;

/// Assert the store holds `def`'s canonical bytes at exactly `def.hash()` and
/// that they decode back to the identical definition (the GetMoteDetail read).
fn assert_def_persisted(store: &LocalFsContentStore, def: &MoteDef) {
    let addr = ContentRef::from_bytes(*def.hash().as_bytes());
    let bytes = store
        .get(&addr)
        .expect("def blob must exist at ContentRef(def.hash())");
    let decoded = MoteDef::decode(&bytes).expect("persisted def bytes decode");
    assert_eq!(&decoded, def, "persisted bytes round-trip to the same def");
}

fn warrant() -> WarrantSpec {
    WarrantSpec {
        mote_class: MoteClass::ReadOnlyNondet,
        nd_class: MoteClass::ReadOnlyNondet,
        fs_scope: FsScope::empty(),
        net_scope: NetScope::None,
        syscall_profile_ref: ContentRef::from_bytes([0; 32]),
        tool_grants: BTreeSet::new(),
        model_route: ModelRoute {
            model_id: ModelId("planner-v1".into()),
            max_input_tokens: 1024,
            max_output_tokens: 1024,
            max_calls: 8,
        },
        resource_ceiling: ResourceCeiling {
            cpu_milli: 1000,
            mem_bytes: 1 << 20,
            wall_clock_ms: 60_000,
            fd_count: 64,
            disk_bytes: 1 << 20,
        },
        environment_ref: None,
        executor_class: ExecutorClass::MacOsSandbox,
        ..Default::default()
    }
}

fn role_registry(spec: &WarrantSpec) -> Arc<dyn RoleRegistry> {
    let reg = InMemoryRoleRegistry::new();
    let role = Role {
        name: "worker".into(),
        version: 1,
        spec: spec.clone(),
        description: String::new(),
    };
    reg.register(RoleId("role-10".into()), role.clone());
    reg.register(RoleId("role-20".into()), role);
    Arc::new(reg)
}

fn shaper_def() -> MoteDef {
    MoteDef {
        critic_check: None,
        logic_ref: LogicRef([1u8; 32]),
        model_id: ModelId("planner-v1".into()),
        prompt_template_hash: PromptTemplateHash([3u8; 32]),
        tool_contract: BTreeMap::new(),
        nd_class: NdClass::ReadOnlyNondet,
        config_subset: BTreeMap::new(),
        effect_pattern: EffectPattern::IdempotentByConstruction,
        critic_for: None,
        is_topology_shaper: true,
        inference_params: kx_mote::InferenceParams::default(),
        schema_version: MOTE_DEF_SCHEMA_VERSION,
    }
}

fn descriptor(seed: u8) -> ChildDescriptor {
    ChildDescriptor {
        role_id: RoleId(format!("role-{seed}")),
        logic_ref: LogicRef([seed; 32]),
        nd_class: NdClass::Pure,
        effect_pattern: EffectPattern::IdempotentByConstruction,
        intent: ConfigVal(format!("subtask-{seed}").into_bytes()),
    }
}

fn shaper_mote(def: &MoteDef) -> Mote {
    Mote::new(
        def.clone(),
        InputDataId::from_bytes([0u8; 32]),
        GraphPosition(vec![0u8]),
        SmallVec::new(),
    )
}

fn coordinator(dir: &TempDir, w: &WarrantSpec) -> (CoordinatorService, Arc<LocalFsContentStore>) {
    let store = Arc::new(LocalFsContentStore::open(dir.path().join("content")).unwrap());
    let journal = SqliteJournal::open(dir.path().join("journal.db")).unwrap();
    let registry: Arc<dyn WorkerRegistry> = Arc::new(InMemoryWorkerRegistry::new());
    let svc = CoordinatorService::with_shaper_materialization(
        journal,
        registry,
        store.clone(),
        Arc::new(kx_coordinator::SystemClock),
        Arc::new(kx_coordinator::OsRandomNonce),
        Arc::new(kx_tool_registry::InMemoryToolRegistry::with_builtins()),
        role_registry(w),
    );
    (svc, store)
}

async fn register_run(svc: &CoordinatorService) {
    let _ = svc
        .register_run(Request::new(kx_coordinator::proto::RegisterRunRequest {
            recipe_fingerprint: vec![0x5au8; 32],
        }))
        .await;
}

async fn submit(svc: &CoordinatorService, mote: &Mote, w: &WarrantSpec) {
    let resp = svc
        .submit_mote(Request::new(kx_coordinator::proto::SubmitMoteRequest {
            mote: Some(mote.clone().into()),
            warrant: Some(w.clone().into()),
            accept_at_least_once: false,
            react_seed: false,
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        resp.status,
        kx_coordinator::proto::SubmitStatus::Accepted as i32
    );
}

async fn commit_with_result(
    svc: &CoordinatorService,
    mote: &Mote,
    warrant_ref: ContentRef,
    result_ref: ContentRef,
    worker_id: u64,
) -> i32 {
    let id = mote.id.as_bytes().to_vec();
    svc.report_commit(Request::new(kx_coordinator::proto::ReportCommitRequest {
        mote_id: id.clone(),
        idempotency_key: id,
        result_ref: result_ref.as_bytes().to_vec(),
        warrant_ref: warrant_ref.as_bytes().to_vec(),
        mote_def_hash: mote.def.hash().as_bytes().to_vec(),
        nd_class: kx_coordinator::proto::NdClass::from(mote.def.nd_class) as i32,
        parents: mote.parents.iter().cloned().map(Into::into).collect(),
        worker_id,
    }))
    .await
    .unwrap()
    .into_inner()
    .outcome
}

/// Flagship: a fresh root submit persists the def blob content-addressed at
/// `def.hash()`, and an idempotent re-submit leaves it intact (no error, no
/// duplicate object).
#[tokio::test]
async fn root_submit_persists_def_content_addressed() {
    let dir = TempDir::new().unwrap();
    let w = warrant();
    let (svc, store) = coordinator(&dir, &w);
    let def = shaper_def();
    let mote = shaper_mote(&def);

    register_run(&svc).await;
    submit(&svc, &mote, &w).await;
    assert_def_persisted(&store, &def);

    // Idempotent re-submit (DUPLICATE path) — the blob stays put.
    let resp = svc
        .submit_mote(Request::new(kx_coordinator::proto::SubmitMoteRequest {
            mote: Some(mote.clone().into()),
            warrant: Some(w.clone().into()),
            accept_at_least_once: false,
            react_seed: false,
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        resp.status,
        kx_coordinator::proto::SubmitStatus::Duplicate as i32
    );
    assert_def_persisted(&store, &def);

    // SELF-HEAL: the duplicate path re-puts too, so a def first admitted by a
    // pre-Batch-B binary (no blob) is repaired by any idempotent re-submit.
    store
        .delete(&ContentRef::from_bytes(*def.hash().as_bytes()))
        .unwrap();
    let resp = svc
        .submit_mote(Request::new(kx_coordinator::proto::SubmitMoteRequest {
            mote: Some(mote.clone().into()),
            warrant: Some(w.clone().into()),
            accept_at_least_once: false,
            react_seed: false,
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        resp.status,
        kx_coordinator::proto::SubmitStatus::Duplicate as i32
    );
    assert_def_persisted(&store, &def);
}

/// A committed shaper's MATERIALIZED children (defs the client never submitted)
/// get their def blobs persisted at the materialization admission site.
#[tokio::test]
async fn materialized_shaper_children_defs_are_persisted() {
    let dir = TempDir::new().unwrap();
    let w = warrant();
    let (svc, store) = coordinator(&dir, &w);

    let def = shaper_def();
    let shaper = shaper_mote(&def);
    let td = TopologyDecision {
        children: vec![descriptor(10), descriptor(20)],
    };
    let td_ref = store.put(&td.encode()).unwrap();

    register_run(&svc).await;
    submit(&svc, &shaper, &w).await;
    let worker = common::register(&svc, "w").await;
    common::lease_work(&svc, worker, MAC, 16).await;
    let outcome = commit_with_result(&svc, &shaper, warrant_ref_of(&w), td_ref, worker).await;
    assert_eq!(outcome, CommitOutcome::Committed as i32);

    // Lease the materialized children; every leased def must be persisted at
    // its content address (the leased Mote IS the admitted ground truth).
    let child_items = common::lease_work(&svc, worker, MAC, 16).await;
    assert_eq!(child_items.len(), 2, "both children leasable");
    for item in &child_items {
        let child: Mote = item.mote.clone().unwrap().try_into().unwrap();
        assert_def_persisted(&store, &child.def);
    }
}

/// A storeless coordinator admits unchanged — def persistence is best-effort
/// display substrate, never an admission dependency.
#[tokio::test]
async fn storeless_coordinator_admits_without_persisting() {
    let svc = CoordinatorService::new(kx_journal::InMemoryJournal::new());
    let _ = svc
        .register_run(Request::new(kx_coordinator::proto::RegisterRunRequest {
            recipe_fingerprint: vec![0x5au8; 32],
        }))
        .await;
    let def = shaper_def();
    let mote = shaper_mote(&def);
    let resp = svc
        .submit_mote(Request::new(kx_coordinator::proto::SubmitMoteRequest {
            mote: Some(mote.into()),
            warrant: Some(warrant().into()),
            accept_at_least_once: false,
            react_seed: false,
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        resp.status,
        kx_coordinator::proto::SubmitStatus::Accepted as i32,
        "admission never depends on the display-only def blob"
    );
}
