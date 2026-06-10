//! PR-2b — LIVE shaper-child materialization + dispatch routing (T1.1, the core gap).
//!
//! The §2.149 gap: the live coordinator folds a committed shaper but its children — which
//! a client never submitted — never enter the dispatch admission set (`Dispatch.defs`), so
//! `lease_ready` skips them and they never reach a worker. This file proves the splice:
//! a committed shaper's children become leasable, with IDENTITIES that match the canonical
//! `DefaultTopologyMaterializer` (one source of truth), and SURVIVE a coordinator restart
//! (recovery re-derives them from the committed `TopologyDecision` fact — R49, never
//! re-sampled). The shaper executor that PRODUCES the decision is exercised at the gateway
//! layer (PR-2b Step 4); here the decision is staged directly so the test is deterministic.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use kx_content::{ContentRef, ContentStore, LocalFsContentStore};
use kx_coordinator::proto::coordinator_server::Coordinator;
use kx_coordinator::proto::{CommitOutcome, ExecutorClass as ProtoExecutorClass};
use kx_coordinator::{CoordinatorService, InMemoryWorkerRegistry, MoteState, WorkerRegistry};
use kx_journal::SqliteJournal;
use kx_mote::{
    ChildDescriptor, ConfigVal, EffectPattern, GraphPosition, InputDataId, LogicRef, ModelId, Mote,
    MoteDef, NdClass, PromptTemplateHash, RoleId, TopologyDecision, MOTE_DEF_SCHEMA_VERSION,
};
use kx_projection::{derive_child_identity, InheritFromShaperResolver};
use kx_warrant::{
    warrant_ref_of, ExecutorClass, FsScope, InMemoryRoleRegistry, ModelRoute, MoteClass, NetScope,
    ResourceCeiling, Role, RoleRegistry, WarrantSpec,
};
use smallvec::SmallVec;
use tempfile::TempDir;
use tonic::Request;

const MAC: ProtoExecutorClass = ProtoExecutorClass::MacosSandbox;

/// A ROND topology-shaper MoteDef (the model that proposes a fan-out).
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

/// A permissive warrant the shaper runs under (and, via the role registry below, every
/// child inherits so `intersect` is a no-op). `MacosSandbox` so the test worker leases it.
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

/// A role registry that grants every proposed role the shaper's warrant (the harness
/// `InheritShaperWarrantRoles` model — `intersect` is an identity narrow, SN-8-safe).
fn role_registry(spec: &WarrantSpec) -> Arc<dyn RoleRegistry> {
    let reg = InMemoryRoleRegistry::new();
    let role = Role {
        name: "worker".into(),
        version: 1,
        spec: spec.clone(),
        description: String::new(),
    };
    // The decision below proposes role-10 / role-20; register both.
    reg.register(RoleId("role-10".into()), role.clone());
    reg.register(RoleId("role-20".into()), role);
    Arc::new(reg)
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

/// The shaper Mote the client submits (root: no parents). Returns the Mote + its id.
fn shaper_mote(def: &MoteDef) -> Mote {
    Mote::new(
        def.clone(),
        InputDataId::from_bytes([0u8; 32]),
        GraphPosition(vec![0u8]),
        SmallVec::new(),
    )
}

/// Commit `mote` through the service with an explicit `result_ref` (the staged decision)
/// and `warrant_ref`. Mirrors `common::commit` but lets the test point the commit at the
/// TopologyDecision it staged in the shared store.
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

async fn register_run(svc: &CoordinatorService) {
    let _ = svc
        .register_run(Request::new(kx_coordinator::proto::RegisterRunRequest {
            recipe_fingerprint: vec![0x5au8; 32],
        }))
        .await;
}

async fn submit(svc: &CoordinatorService, mote: &Mote, w: &WarrantSpec) {
    svc.submit_mote(Request::new(kx_coordinator::proto::SubmitMoteRequest {
        mote: Some(mote.clone().into()),
        warrant: Some(w.clone().into()),
        accept_at_least_once: false,
        react_seed: false,
    }))
    .await
    .unwrap();
}

/// Build a coordinator wired for live shaper materialization over a durable journal + a
/// shared on-disk store (so a "restart" can reopen the same journal/store).
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

/// The expected materialized child ids — the SAME derivation a `DefaultTopologyMaterializer`
/// performs (one source of truth).
fn expected_children(
    shaper_id: kx_mote::MoteId,
    def: &MoteDef,
    td_ref: ContentRef,
    td: &TopologyDecision,
) -> Vec<kx_mote::MoteId> {
    td.children
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let (id, _h, _nd, _ep) =
                derive_child_identity(shaper_id, def, td_ref, d, i, &InheritFromShaperResolver);
            id
        })
        .collect()
}

/// Flagship: a committed shaper's children become leasable with materializer-identical ids.
#[tokio::test]
async fn committed_shaper_children_are_materialized_and_leasable() {
    let dir = TempDir::new().unwrap();
    let w = warrant();
    let (svc, store) = coordinator(&dir, &w);

    let def = shaper_def();
    let shaper = shaper_mote(&def);
    let td = TopologyDecision {
        children: vec![descriptor(10), descriptor(20)],
    };
    let td_ref = store.put(&td.encode()).unwrap();
    assert_eq!(
        td_ref,
        ContentRef::from_bytes(td.hash()),
        "staged ref == decision hash"
    );

    register_run(&svc).await;
    submit(&svc, &shaper, &w).await;
    let worker = common::register(&svc, "w").await;

    // The shaper is the only ready Mote; lease + commit it pointing at the staged decision.
    let leased = common::lease_work(&svc, worker, MAC, 16).await;
    assert_eq!(leased.len(), 1, "the shaper is ready");
    let outcome = commit_with_result(&svc, &shaper, warrant_ref_of(&w), td_ref, worker).await;
    assert_eq!(outcome, CommitOutcome::Committed as i32);
    assert_eq!(svc.state_of(shaper.id).await.unwrap(), MoteState::Committed);

    // The gap, closed: both children are now leasable (materialized into dispatch.defs).
    let expected = expected_children(shaper.id, &def, td_ref, &td);
    assert_eq!(expected.len(), 2);
    let child_items = common::lease_work(&svc, worker, MAC, 16).await;
    let leased_ids: BTreeSet<_> = child_items
        .iter()
        .map(|it| {
            let m: Mote = it.mote.clone().unwrap().try_into().unwrap();
            m.id
        })
        .collect();
    for child in &expected {
        assert!(
            leased_ids.contains(child),
            "materialized child {child:?} must be leasable (== DefaultTopologyMaterializer id)"
        );
    }
}

/// Crash mid-shaper-commit → resume re-materializes children from the committed fact
/// (R49: served from the journal, never re-derived against a different decision).
#[tokio::test]
async fn shaper_children_survive_a_coordinator_restart() {
    let dir = TempDir::new().unwrap();
    let w = warrant();
    let def = shaper_def();
    let shaper = shaper_mote(&def);
    let td = TopologyDecision {
        children: vec![descriptor(10), descriptor(20)],
    };
    let expected: BTreeSet<_>;

    // First coordinator: drive the shaper to Committed, then DROP it (simulated crash —
    // the in-memory dispatch.defs + materialized children are lost; the journal persists).
    let td_ref = {
        let (svc, store) = coordinator(&dir, &w);
        let td_ref = store.put(&td.encode()).unwrap();
        register_run(&svc).await;
        submit(&svc, &shaper, &w).await;
        let worker = common::register(&svc, "w").await;
        common::lease_work(&svc, worker, MAC, 16).await;
        let outcome = commit_with_result(&svc, &shaper, warrant_ref_of(&w), td_ref, worker).await;
        assert_eq!(outcome, CommitOutcome::Committed as i32);
        expected = expected_children(shaper.id, &def, td_ref, &td)
            .into_iter()
            .collect();
        td_ref
        // svc dropped here → coordinator gone
    };

    // Restart over the SAME journal + store. Recovery folds the committed shaper; the
    // client re-invokes (idempotent re-submit), which re-materializes the children.
    let (svc2, _store2) = coordinator(&dir, &w);
    assert_eq!(
        svc2.state_of(shaper.id).await.unwrap(),
        MoteState::Committed,
        "the shaper's commit survived the restart"
    );
    register_run(&svc2).await;
    submit(&svc2, &shaper, &w).await; // idempotent (already committed) → triggers re-materialization
    let worker2 = common::register(&svc2, "w2").await;

    let child_items = common::lease_work(&svc2, worker2, MAC, 16).await;
    let leased_ids: BTreeSet<_> = child_items
        .iter()
        .map(|it| {
            let m: Mote = it.mote.clone().unwrap().try_into().unwrap();
            m.id
        })
        .collect();
    assert_eq!(
        leased_ids, expected,
        "after restart the SAME children (R49) re-materialize and are leasable"
    );
    let _ = td_ref;
}
