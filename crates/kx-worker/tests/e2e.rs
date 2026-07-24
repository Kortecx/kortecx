//! End-to-end distributed witnesses over real gRPC + a shared content-addressed store.
//!
//! - **P2.4** (`committed_result_is_readable_by_coordinator_and_peer`): a result
//!   committed via one worker is readable by the coordinator and by another worker;
//!   plus `phantom_result_ref_is_rejected` (D55 store verification).
//! - **P2.5 / P2 EXIT GATE** (`two_workers_share_the_workload_via_placement`): two
//!   workers of the same class run a workflow distributed, placement v2 (D56) balances
//!   the Motes across them, all commit exactly once.
//!
//! Topology: one in-process `CoordinatorService` built `with_store` (so it VERIFIES each
//! committed `result_ref` against the shared store — D55) on loopback; workers run PURE
//! Motes through a **storing** executor (real bytes land in the shared store before they
//! propose). The store is one `LocalFsContentStore` root all nodes share (single host
//! now; the S3 backend is the cross-host impl at P5.5).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use kx_content::{ContentRef, ContentStore, LocalFsContentStore};
use kx_coordinator::proto::coordinator_server::{Coordinator, CoordinatorServer};
use kx_coordinator::{CoordinatorService, MoteState, WorkerId, WorkerStatus};
use kx_executor::{LocalResourceManager, MoteExecutor, TestMoteExecutor};
use kx_journal::InMemoryJournal;
use kx_mote::Mote;
use kx_work_cache::{SqliteWorkCache, WorkCache};
use kx_worker::{Worker, WorkerClient};
use tempfile::TempDir;
use tonic::transport::Server;
use tonic::Request;

/// The deterministic result payload worker A's executor publishes for a Mote.
fn result_bytes(mote: &Mote) -> Vec<u8> {
    let mut v = b"kx-result:".to_vec();
    v.extend_from_slice(mote.id.as_bytes());
    v
}

/// A `MoteExecutor` that PUBLISHES its result bytes to the shared store and returns
/// the ref — the correct producer for the PURE path (no R-11 gate; content-addressed
/// so the committed ref == the stored object). Built via the existing public
/// `TestMoteExecutor::new` constructor — kx-executor source is untouched.
fn storing_executor(store: Arc<LocalFsContentStore>) -> Arc<dyn MoteExecutor> {
    Arc::new(TestMoteExecutor::new(move |mote, _warrant| {
        store
            .put(&result_bytes(mote))
            .expect("publish result bytes")
    }))
}

async fn submit(svc: &CoordinatorService, mote: &Mote, warrant: &kx_warrant::WarrantSpec) {
    // M1.3: register the run (idempotent) so the submit passes the
    // registration-before-submit gate.
    let _ = svc
        .register_run(Request::new(kx_coordinator::proto::RegisterRunRequest {
            recipe_fingerprint: vec![0x5au8; 32],
        }))
        .await;
    svc.submit_mote(Request::new(kx_coordinator::proto::SubmitMoteRequest {
        mote: Some(mote.clone().into()),
        warrant: Some(warrant.clone().into()),
        accept_at_least_once: false,
        react_seed: false,
    }))
    .await
    .unwrap();
}

async fn connect(endpoint: &str) -> WorkerClient {
    for _ in 0..100 {
        if let Ok(c) = WorkerClient::connect(endpoint.to_string()).await {
            return c;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("worker connects to the coordinator");
}

/// Spawn an in-process coordinator (built `with_store`) on loopback; return its
/// service clone (for submission + assertions) and the endpoint URL.
async fn spawn_coordinator(store: Arc<LocalFsContentStore>) -> (CoordinatorService, String) {
    let svc = CoordinatorService::with_store(InMemoryJournal::new(), store);
    let server_svc = svc.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    tokio::spawn(async move {
        Server::builder()
            .add_service(CoordinatorServer::new(server_svc))
            .serve(addr)
            .await
            .unwrap();
    });
    (svc, format!("http://{addr}"))
}

#[tokio::test]
async fn committed_result_is_readable_by_coordinator_and_peer() {
    let dir = TempDir::new().unwrap();
    let store = Arc::new(LocalFsContentStore::open(dir.path()).unwrap());
    let (svc, endpoint) = spawn_coordinator(store.clone()).await;

    // root -> child PURE DAG.
    let root = common::pure_mote(1, &[]);
    let child = common::pure_mote(2, &[root.id]);
    let warrant = common::pure_warrant();
    submit(&svc, &root, &warrant).await;
    submit(&svc, &child, &warrant).await;

    // Worker A runs + proposes through the storing executor; the coordinator
    // verifies each result_ref is present in the store before committing.
    let mut worker_a = Worker::register(
        connect(&endpoint).await,
        common::WORKER_CLASS,
        "inproc://worker-a",
        storing_executor(store.clone()),
        LocalResourceManager::dev_defaults(),
        store.clone(),
        common::noop_broker(),
        16,
    )
    .await
    .unwrap();

    assert_eq!(worker_a.run_once().await.unwrap(), 1, "root committed");
    assert_eq!(
        worker_a.run_once().await.unwrap(),
        1,
        "child committed once parent is"
    );
    assert_eq!(svc.committed_count().await.unwrap(), 2);
    assert_eq!(svc.state_of(child.id).await.unwrap(), MoteState::Committed);

    // (a) Readable by the coordinator: it serves the committed result_ref over
    //     ReadEntries, and the bytes resolve from the store it was built with (the
    //     same store.contains check it ran at commit time).
    let mut observer = connect(&endpoint).await;
    let (entries, _next) = observer.read_entries(0, 16).await.unwrap();
    let root_ref = entries
        .iter()
        .find_map(|e| match e.kind.as_ref().unwrap() {
            kx_coordinator::proto::journal_entry::Kind::Committed(c)
                if c.mote_id == root.id.as_bytes().to_vec() =>
            {
                Some(ContentRef::from_bytes(
                    c.result_ref.clone().try_into().unwrap(),
                ))
            }
            _ => None,
        })
        .expect("coordinator serves the root's committed result_ref");
    assert_eq!(
        store.get(&root_ref).unwrap().to_vec(),
        result_bytes(&root),
        "coordinator-visible result_ref resolves to the bytes A published"
    );

    // (b) Readable by another worker: peer B folds the log + reads from the store.
    let mut worker_b = Worker::register(
        connect(&endpoint).await,
        common::WORKER_CLASS,
        "inproc://worker-b",
        storing_executor(store.clone()), // unused by B; B only reads
        LocalResourceManager::dev_defaults(),
        store.clone(),
        common::noop_broker(),
        16,
    )
    .await
    .unwrap();
    assert_eq!(
        worker_b.peer_read(root.id).await.unwrap(),
        result_bytes(&root),
        "peer worker reads the exact bytes worker A produced"
    );
    assert_eq!(
        worker_b.peer_read(child.id).await.unwrap(),
        result_bytes(&child),
    );
}

#[tokio::test]
async fn phantom_result_ref_is_rejected() {
    let dir = TempDir::new().unwrap();
    let store = Arc::new(LocalFsContentStore::open(dir.path()).unwrap());
    let (svc, endpoint) = spawn_coordinator(store).await;

    let mote = common::pure_mote(9, &[]);
    let warrant = common::pure_warrant();
    submit(&svc, &mote, &warrant).await;

    // A registered worker proposes a result_ref whose bytes were never stored.
    let mut client = connect(&endpoint).await;
    let worker_id = client
        .register_worker(common::WORKER_CLASS, "inproc://phantom")
        .await
        .unwrap();
    let id = mote.id.as_bytes().to_vec();
    let err = client
        .report_commit(kx_coordinator::proto::ReportCommitRequest {
            mote_id: id.clone(),
            idempotency_key: id,
            result_ref: vec![0xAB; 32], // never published to the store
            warrant_ref: vec![4; 32],
            mote_def_hash: vec![5; 32],
            nd_class: kx_coordinator::proto::NdClass::Pure as i32,
            parents: vec![],
            worker_id,
        })
        .await
        .map(|_| ())
        .unwrap_err();
    // ResultRefAbsent maps to INVALID_ARGUMENT (see kx-coordinator error.rs).
    match err {
        kx_worker::WorkerError::Rpc(status) => {
            assert_eq!(status.code(), tonic::Code::InvalidArgument);
        }
        other => panic!("expected an RPC status error, got {other:?}"),
    }
    assert_eq!(
        svc.committed_count().await.unwrap(),
        0,
        "the phantom commit was not recorded"
    );
}

/// P2 EXIT GATE: a two-node (coordinator + ≥2 worker) setup runs the workflow
/// distributed — placement v2 (D56) balances ready Motes across workers of the same
/// class, all commit exactly once, and the executor/inference/scheduler crates are
/// unchanged (thesis test, asserted by the CI diff job, not here).
#[tokio::test]
async fn two_workers_share_the_workload_via_placement() {
    let dir = TempDir::new().unwrap();
    let store = Arc::new(LocalFsContentStore::open(dir.path()).unwrap());
    let (svc, endpoint) = spawn_coordinator(store.clone()).await;

    // Eight independent ready PURE Motes.
    let warrant = common::pure_warrant();
    let motes: Vec<Mote> = (10u8..18).map(|s| common::pure_mote(s, &[])).collect();
    for m in &motes {
        submit(&svc, m, &warrant).await;
    }

    // Two workers of the same class, each leasing small batches.
    let register = |ep: String, tag: &'static str| {
        let store = store.clone();
        async move {
            Worker::register(
                connect(&ep).await,
                common::WORKER_CLASS,
                tag,
                storing_executor(store.clone()),
                LocalResourceManager::dev_defaults(),
                store,
                common::noop_broker(),
                2,
            )
            .await
            .unwrap()
        }
    };
    let mut worker_a = register(endpoint.clone(), "inproc://a").await;
    let mut worker_b = register(endpoint.clone(), "inproc://b").await;

    // Interleave bounded polls until the DAG drains. Placement routes each worker its
    // sharded Motes first; fill-to-max keeps a poller busy if its shard is empty, so
    // neither starves and both make progress.
    let mut a_total = 0usize;
    let mut b_total = 0usize;
    for _ in 0..16 {
        a_total += worker_a.run_once().await.unwrap();
        b_total += worker_b.run_once().await.unwrap();
        if svc.committed_count().await.unwrap() >= motes.len() {
            break;
        }
    }

    assert_eq!(
        svc.committed_count().await.unwrap(),
        motes.len(),
        "every Mote committed exactly once across the two workers"
    );
    assert!(
        a_total > 0 && b_total > 0,
        "placement shared work across both workers (a={a_total}, b={b_total})"
    );
    assert_eq!(
        a_total + b_total,
        motes.len(),
        "no Mote double-committed (dedup holds)"
    );

    // Load reporting reached the coordinator (run_once heartbeats in_flight).
    let rec_a = svc.registry().get(WorkerId(worker_a.worker_id())).unwrap();
    let rec_b = svc.registry().get(WorkerId(worker_b.worker_id())).unwrap();
    assert!(
        rec_a.last_heartbeat_ms > 0 || rec_b.last_heartbeat_ms > 0,
        "at least one worker reported load via heartbeat during run"
    );

    // Cross-worker read still holds (P2.4): B reads a result regardless of who ran it.
    assert_eq!(
        worker_b.peer_read(motes[0].id).await.unwrap(),
        result_bytes(&motes[0]),
    );
}

/// A `MoteExecutor` that PUBLISHES result bytes to the shared store AND counts every
/// physical run — so a cross-run cache HIT (which skips `run`) is observable as a flat
/// counter.
fn counting_storing_executor(
    store: Arc<LocalFsContentStore>,
    counter: Arc<AtomicUsize>,
) -> Arc<dyn MoteExecutor> {
    Arc::new(TestMoteExecutor::new(move |mote, _warrant| {
        counter.fetch_add(1, Ordering::SeqCst);
        store
            .put(&result_bytes(mote))
            .expect("publish result bytes")
    }))
}

/// The shared MoteDef both cross-run children use (identical ⇒ identical `mote_def_hash`).
fn shared_child_def() -> kx_mote::MoteDef {
    use std::collections::BTreeMap;

    use kx_mote::{
        EffectPattern, InferenceParams, LogicRef, ModelId, MoteDef, NdClass, PromptTemplateHash,
        MOTE_DEF_SCHEMA_VERSION,
    };
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

/// A PURE **child** mote that does byte-identical work regardless of `graph_seed`:
/// identical `def` and a fixed `input_data_id`, so its `work_fingerprint` is constant,
/// while `graph_seed` varies `graph_position` (⇒ a distinct `MoteId`) — the exact shape
/// of the same PURE sub-task appearing in two different runs. `parent` only gates
/// scheduling (it is not part of the identity or the fingerprint).
fn shared_work_child(graph_seed: u8, parent: kx_mote::MoteId) -> Mote {
    use kx_mote::{EdgeMeta, GraphPosition, InputDataId, ParentRef};
    use smallvec::SmallVec;

    let mut parents: SmallVec<[ParentRef; 4]> = SmallVec::new();
    parents.push(ParentRef {
        parent_id: parent,
        edge: EdgeMeta::data(),
    });
    Mote::new(
        shared_child_def(),
        InputDataId::from_bytes([42u8; 32]),
        GraphPosition(vec![graph_seed]),
        parents,
    )
}

/// **Cross-run work cache (the live distributed proof).** Two SEPARATE runs — two
/// coordinators, each its own journal — share ONE `SqliteWorkCache` and ONE content
/// store. A PURE child that does byte-identical work runs in run A (populating the
/// cache) and then appears in run B; the worker in run B serves it FROM THE CACHE
/// through the real gRPC lease→run→propose-commit path, computing it exactly once
/// across the two runs. The in-run memoizer could not dedup these (distinct `MoteId`s);
/// only the run-independent `work_fingerprint` does.
#[tokio::test]
async fn cross_run_work_cache_computes_a_pure_child_once_across_two_runs() {
    let dir = TempDir::new().unwrap();
    let store = Arc::new(LocalFsContentStore::open(dir.path()).unwrap());
    let cache: Arc<dyn WorkCache> = Arc::new(SqliteWorkCache::open_in_memory().unwrap());
    let computes = Arc::new(AtomicUsize::new(0));
    let warrant = common::pure_warrant();

    // Two independent runs, each a coordinator with its own journal, sharing the store.
    let (svc_a, ep_a) = spawn_coordinator(store.clone()).await;
    let (svc_b, ep_b) = spawn_coordinator(store.clone()).await;

    // Run A: a root parent + a shared-work child.
    let parent_a = common::pure_mote(1, &[]);
    let child_a = shared_work_child(10, parent_a.id);
    submit(&svc_a, &parent_a, &warrant).await;
    submit(&svc_a, &child_a, &warrant).await;

    // Run B: a DIFFERENT root parent + a child that does byte-identical work to child A.
    let parent_b = common::pure_mote(2, &[]);
    let child_b = shared_work_child(11, parent_b.id);
    submit(&svc_b, &parent_b, &warrant).await;
    submit(&svc_b, &child_b, &warrant).await;

    // Distinct identities, identical work.
    assert_ne!(
        child_a.id, child_b.id,
        "distinct MoteIds (different graph_position)"
    );

    // A worker per run, both sharing the ONE work cache.
    let mut worker_a = Worker::register(
        connect(&ep_a).await,
        common::WORKER_CLASS,
        "inproc://run-a",
        counting_storing_executor(store.clone(), computes.clone()),
        LocalResourceManager::dev_defaults(),
        store.clone(),
        common::noop_broker(),
        16,
    )
    .await
    .unwrap()
    .with_work_cache(Some(cache.clone()));

    let mut worker_b = Worker::register(
        connect(&ep_b).await,
        common::WORKER_CLASS,
        "inproc://run-b",
        counting_storing_executor(store.clone(), computes.clone()),
        LocalResourceManager::dev_defaults(),
        store.clone(),
        common::noop_broker(),
        16,
    )
    .await
    .unwrap()
    .with_work_cache(Some(cache.clone()));

    // Run A: parent then child. The child MISSES (cold cache) and is computed +
    // populated. 2 physical computes (parent_a + child_a).
    assert_eq!(worker_a.run_once().await.unwrap(), 1, "parent A committed");
    assert_eq!(worker_a.run_once().await.unwrap(), 1, "child A committed");
    assert_eq!(
        computes.load(Ordering::SeqCst),
        2,
        "run A computed parent + child"
    );

    // Run B: parent (a distinct entrypoint, so it computes) then the shared-work child.
    assert_eq!(worker_b.run_once().await.unwrap(), 1, "parent B committed");
    assert_eq!(
        worker_b.run_once().await.unwrap(),
        1,
        "child B committed — served from the cross-run cache"
    );

    // THE PROOF: exactly 3 physical computes total (parent_a, child_a, parent_b) —
    // child_b was served from the cache, NOT recomputed.
    assert_eq!(
        computes.load(Ordering::SeqCst),
        3,
        "child B was served from the cross-run work cache (not recomputed)"
    );

    // And child B's committed result IS child A's output (the cached ref), read back
    // through the coordinator's committed log + the shared store.
    assert_eq!(
        svc_b.state_of(child_b.id).await.unwrap(),
        MoteState::Committed
    );
    assert_eq!(
        worker_b.peer_read(child_b.id).await.unwrap(),
        result_bytes(&child_a),
        "run B's committed child result is exactly run A's computed output"
    );
}

/// P3.1: an **idle** worker (leasing no work) stays live in the coordinator's
/// registry because its background heartbeat keeps reporting in. Without it, an idle
/// worker would send nothing and be falsely declared dead.
#[tokio::test]
async fn background_heartbeat_keeps_an_idle_worker_live() {
    let dir = TempDir::new().unwrap();
    let store = Arc::new(LocalFsContentStore::open(dir.path()).unwrap());
    let (svc, endpoint) = spawn_coordinator(store.clone()).await;

    // Register a worker but never lease/run anything — it is purely idle.
    let worker = Worker::register(
        connect(&endpoint).await,
        common::WORKER_CLASS,
        "inproc://idle",
        storing_executor(store.clone()),
        LocalResourceManager::dev_defaults(),
        store.clone(),
        common::noop_broker(),
        16,
    )
    .await
    .unwrap();
    let id = worker.worker_id();

    // At registration the worker has not heartbeated yet (advisory timestamp == 0).
    assert_eq!(
        svc.registry().get(WorkerId(id)).unwrap().last_heartbeat_ms,
        0,
        "no heartbeat before the background task starts"
    );

    // Start the background heartbeat at a short cadence; let it tick several times.
    let hb = worker.spawn_heartbeat(Duration::from_millis(20));
    tokio::time::sleep(Duration::from_millis(150)).await;
    hb.abort();

    // The idle worker reached the coordinator on its own (advisory timestamp now set
    // by the background loop — the only heartbeat source, since nothing was leased)
    // and is still live.
    let rec = svc.registry().get(WorkerId(id)).unwrap();
    assert!(
        rec.last_heartbeat_ms > 0,
        "the background heartbeat reached the coordinator while idle"
    );
    assert_eq!(
        svc.registry().status(WorkerId(id)),
        Some(WorkerStatus::Live),
        "the idle worker is kept live by its background heartbeat"
    );
}
