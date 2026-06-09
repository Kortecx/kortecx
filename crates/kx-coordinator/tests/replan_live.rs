//! PR-2c-2 — LIVE re-plan-on-failure inside the coordinator.
//!
//! Proves the runtime-side capability: when a topology shaper's children settle with a
//! failure, the sole-writer coordinator durably drives the NEXT re-plan round — it commits
//! a `ReplanRound` fact and materializes a round-namespaced correction shaper carrying the
//! failure-corrected prompt — and that the chain SURVIVES a coordinator restart (re-derived
//! from committed facts alone). The shaper executor that PRODUCES each round's decision is a
//! gateway concern; here decisions are staged directly so the test is deterministic + model-free.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use kx_content::{ContentRef, ContentStore, LocalFsContentStore};
use kx_coordinator::proto::coordinator_server::Coordinator;
use kx_coordinator::proto::FailureReason as ProtoFailureReason;
use kx_coordinator::proto::{CommitOutcome, ExecutorClass as ProtoExecutorClass};
use kx_coordinator::{CoordinatorService, InMemoryWorkerRegistry, MoteState, WorkerRegistry};
use kx_journal::SqliteJournal;
use kx_mote::{
    ChildDescriptor, ConfigKey, ConfigVal, EffectPattern, GraphPosition, InputDataId, LogicRef,
    ModelId, Mote, MoteDef, NdClass, PromptTemplateHash, RoleId, TopologyDecision,
    MOTE_DEF_SCHEMA_VERSION, PROMPT_KEY,
};
use kx_warrant::{
    warrant_ref_of, ExecutorClass, FsScope, InMemoryRoleRegistry, ModelRoute, MoteClass, NetScope,
    ResourceCeiling, Role, RoleRegistry, WarrantSpec,
};
use smallvec::SmallVec;
use tempfile::TempDir;
use tonic::Request;

const MAC: ProtoExecutorClass = ProtoExecutorClass::MacosSandbox;
const BASE_PROMPT: &str = "Plan the run.";
const MODEL: &str = "planner-v1";

fn shaper_def() -> MoteDef {
    // The round-0 shaper carries the run's base planning prompt — the coordinator
    // auto-anchors it (writes the round-0 ReplanRound) at submit, enabling re-plan.
    let mut config_subset = BTreeMap::new();
    config_subset.insert(
        ConfigKey(PROMPT_KEY.to_string()),
        ConfigVal(BASE_PROMPT.as_bytes().to_vec()),
    );
    MoteDef {
        critic_check: None,
        logic_ref: LogicRef([1u8; 32]),
        model_id: ModelId(MODEL.into()),
        prompt_template_hash: PromptTemplateHash([3u8; 32]),
        tool_contract: BTreeMap::new(),
        nd_class: NdClass::ReadOnlyNondet,
        config_subset,
        effect_pattern: EffectPattern::IdempotentByConstruction,
        critic_for: None,
        is_topology_shaper: true,
        inference_params: kx_mote::InferenceParams::default(),
        schema_version: MOTE_DEF_SCHEMA_VERSION,
    }
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
            model_id: ModelId(MODEL.into()),
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

/// Drive a round-0 shaper to commit + materialize TWO children, then return the two leased
/// child `(Mote, WarrantSpec)` plus the worker — the shared setup for the settle tests.
async fn drive_round0_children(
    svc: &CoordinatorService,
    store: &LocalFsContentStore,
    shaper: &Mote,
    w: &WarrantSpec,
) -> (u64, Vec<(Mote, WarrantSpec)>) {
    let td = TopologyDecision {
        children: vec![descriptor(10), descriptor(20)],
    };
    let td_ref = store.put(&td.encode()).unwrap();

    common::register_run(svc, [0x5a; 32]).await;
    let worker = common::register(svc, "w").await;
    common::submit(svc, shaper, w).await;

    let leased = common::lease_work(svc, worker, MAC, 16).await;
    assert_eq!(leased.len(), 1, "the round-0 shaper is ready");
    let outcome = commit_with_result(svc, shaper, warrant_ref_of(w), td_ref, worker).await;
    assert_eq!(outcome, CommitOutcome::Committed as i32);

    let child_items = common::lease_work(svc, worker, MAC, 16).await;
    assert_eq!(
        child_items.len(),
        2,
        "both children materialized + leasable"
    );
    let children: Vec<(Mote, WarrantSpec)> = child_items
        .into_iter()
        .map(|it| {
            let m: Mote = it.mote.unwrap().try_into().unwrap();
            let warr: WarrantSpec = it.warrant.unwrap().try_into().unwrap();
            (m, warr)
        })
        .collect();
    (worker, children)
}

/// The corrected prompt carried by a leased Mote's `config_subset`, if any.
fn prompt_of(mote: &Mote) -> Option<String> {
    mote.def
        .config_subset
        .get(&ConfigKey(PROMPT_KEY.to_string()))
        .map(|v| String::from_utf8_lossy(&v.0).into_owned())
}

/// Flagship: a round-0 child dead-letters ⇒ the coordinator durably drives round 1 — a NEW
/// topology shaper, leasable, carrying the failure-corrected prompt.
#[tokio::test]
async fn child_failure_drives_a_live_replan_round() {
    let dir = TempDir::new().unwrap();
    let w = warrant();
    let def = shaper_def();
    let shaper = shaper_mote(&def);

    let (svc, store) = coordinator(&dir, &w);
    let (worker, children) = drive_round0_children(&svc, &store, &shaper, &w).await;

    // Settle the round: child[0] dead-letters, child[1] commits.
    let resp = common::report_failure(
        &svc,
        &children[0].0,
        worker,
        ProtoFailureReason::DeadLettered,
    )
    .await
    .unwrap();
    assert!(resp.ack);
    let child1_result = store.put(b"child1-result").unwrap();
    let c1 = commit_with_result(
        &svc,
        &children[1].0,
        warrant_ref_of(&children[1].1),
        child1_result,
        worker,
    )
    .await;
    assert_eq!(c1, CommitOutcome::Committed as i32);
    assert_eq!(
        svc.state_of(children[0].0.id).await.unwrap(),
        MoteState::Failed
    );

    // The coordinator drove round 1: a NEW topology shaper is now leasable, distinct from the
    // round-0 shaper, carrying the failure-corrected prompt (base + the low-entropy token).
    let round1 = common::lease_work(&svc, worker, MAC, 16).await;
    assert_eq!(round1.len(), 1, "the round-1 correction shaper is leasable");
    let r1: Mote = round1[0].mote.clone().unwrap().try_into().unwrap();
    assert!(r1.def.is_topology_shaper, "round-1 is a topology shaper");
    assert_ne!(r1.id, shaper.id, "round 1 is a distinct shaper");
    assert!(r1.parents.is_empty(), "the re-plan shaper is edge-free");
    let prompt = prompt_of(&r1).expect("round-1 shaper carries a corrected prompt");
    assert!(prompt.starts_with(BASE_PROMPT), "preserves the base prompt");
    assert!(
        prompt.contains("dead-lettered"),
        "renders the low-entropy failure token (SN-8: no result bytes): {prompt}"
    );
}

/// Every child commits ⇒ the round succeeds ⇒ NO re-plan round is driven (PR-2 parity).
#[tokio::test]
async fn all_children_commit_drives_no_replan() {
    let dir = TempDir::new().unwrap();
    let w = warrant();
    let def = shaper_def();
    let shaper = shaper_mote(&def);

    let (svc, store) = coordinator(&dir, &w);
    let (worker, children) = drive_round0_children(&svc, &store, &shaper, &w).await;

    for (i, (child, warr)) in children.iter().enumerate() {
        let r = store.put(format!("result-{i}").as_bytes()).unwrap();
        let o = commit_with_result(&svc, child, warrant_ref_of(warr), r, worker).await;
        assert_eq!(o, CommitOutcome::Committed as i32);
    }
    // No failures ⇒ no new shaper; a further lease yields nothing.
    let after = common::lease_work(&svc, worker, MAC, 16).await;
    assert!(after.is_empty(), "success ⇒ no re-plan round driven");
}

/// Commit `shaper` with a single-child decision, fail that child, and return the NEXT
/// round's correction shaper if the coordinator drove one (`None` at budget exhaustion).
async fn settle_one_round(
    svc: &CoordinatorService,
    store: &LocalFsContentStore,
    shaper: &Mote,
    w: &WarrantSpec,
    worker: u64,
) -> Option<Mote> {
    let td = TopologyDecision {
        children: vec![descriptor(10)],
    };
    let td_ref = store.put(&td.encode()).unwrap();
    // Lease + commit the shaper pointing at its staged decision.
    let leased = common::lease_work(svc, worker, MAC, 16).await;
    assert_eq!(leased.len(), 1, "the round shaper is leasable");
    let s: Mote = leased[0].mote.clone().unwrap().try_into().unwrap();
    assert_eq!(s.id, shaper.id);
    let o = commit_with_result(svc, shaper, warrant_ref_of(w), td_ref, worker).await;
    assert_eq!(o, CommitOutcome::Committed as i32);
    // Lease the child + dead-letter it → the round settles with a failure.
    let child_items = common::lease_work(svc, worker, MAC, 16).await;
    assert_eq!(child_items.len(), 1, "the single child materialized");
    let child: Mote = child_items[0].mote.clone().unwrap().try_into().unwrap();
    common::report_failure(svc, &child, worker, ProtoFailureReason::DeadLettered)
        .await
        .unwrap();
    // The next round's shaper, if the coordinator drove one (and only one).
    let next = common::lease_work(svc, worker, MAC, 16).await;
    next.into_iter()
        .next()
        .map(|it| it.mote.unwrap().try_into().unwrap())
}

/// Every round fails ⇒ the chain is bounded at `MAX_SHAPER_ROUNDS` (4 total: round 0 + 3
/// corrective rounds), then quiesces — no runaway re-plan / unbounded journal growth.
#[tokio::test]
async fn replan_chain_is_bounded_by_the_round_budget() {
    let dir = TempDir::new().unwrap();
    let w = warrant();
    let def = shaper_def();
    let shaper = shaper_mote(&def);

    let (svc, store) = coordinator(&dir, &w);
    common::register_run(&svc, [0x5a; 32]).await;
    let worker = common::register(&svc, "w").await;
    common::submit(&svc, &shaper, &w).await;

    // Round 0 → 1 → 2 → 3 each fail and drive the next; round 3's failure drives NOTHING.
    let mut current = shaper.clone();
    let mut driven = 0u32; // corrective rounds beyond round 0
    for _ in 0..6 {
        match settle_one_round(&svc, &store, &current, &w, worker).await {
            Some(next) => {
                assert!(next.def.is_topology_shaper);
                assert_ne!(next.id, current.id, "each round is a distinct shaper");
                current = next;
                driven += 1;
            }
            None => break,
        }
    }
    assert_eq!(
        driven, 3,
        "round 0 + exactly 3 corrective rounds (MAX_SHAPER_ROUNDS=4), then quiesce"
    );
}

/// Crash after the round-0 children settle with a failure (before the coordinator drives
/// round 1) ⇒ recovery completes the interrupted round: round 1 is driven from committed
/// facts alone, surviving the restart.
#[tokio::test]
async fn settled_round_survives_restart_and_drives_replan() {
    let dir = TempDir::new().unwrap();
    let w = warrant();
    let def = shaper_def();
    let shaper = shaper_mote(&def);

    // First coordinator: settle the round (child[0] fails, child[1] commits), then DROP it
    // WITHOUT giving the settle drain a chance to surface round 1 to a new lease.
    {
        let (svc, store) = coordinator(&dir, &w);
        let (worker, children) = drive_round0_children(&svc, &store, &shaper, &w).await;
        common::report_failure(
            &svc,
            &children[0].0,
            worker,
            ProtoFailureReason::DeadLettered,
        )
        .await
        .unwrap();
        let r = store.put(b"c1").unwrap();
        commit_with_result(
            &svc,
            &children[1].0,
            warrant_ref_of(&children[1].1),
            r,
            worker,
        )
        .await;
        assert_eq!(
            svc.state_of(children[0].0.id).await.unwrap(),
            MoteState::Failed
        );
        // svc dropped here → simulated crash. The journal + store persist on disk.
    }

    // Restart over the SAME journal + store: recovery re-derives the chain and drives round 1.
    let (svc, _store) = coordinator(&dir, &w);
    let worker = common::register(&svc, "w2").await;
    let round1 = common::lease_work(&svc, worker, MAC, 16).await;
    assert_eq!(
        round1.len(),
        1,
        "round 1 is re-derived + leasable after restart"
    );
    let r1: Mote = round1[0].mote.clone().unwrap().try_into().unwrap();
    assert!(r1.def.is_topology_shaper);
    assert_ne!(r1.id, shaper.id);
    let prompt = prompt_of(&r1).expect("recovered round-1 shaper carries a corrected prompt");
    assert!(prompt.contains("dead-lettered"), "{prompt}");
}
