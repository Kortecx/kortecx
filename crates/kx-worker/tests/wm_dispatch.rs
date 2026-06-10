//! P3.6b — WORLD-MUTATING distributed dispatch (D58 §8, the W-1…W-5 obligations).
//!
//! A worker stages its intent (`ReportEffectStaged`) through the coordinator (sole
//! writer, D40) BEFORE firing the effect through its `CapabilityBroker`, then PROPOSES
//! the staged ref (`ReportCommit`). Exactly-once holds across worker death by composing
//! the staged-intent recovery hint (D58 §2) with reschedule (D57) and the tool-boundary
//! idempotency key (D38 §1).
//!
//! - **W-1** happy stage→fire→commit.
//! - **W-2** ordering: the worker does not fire before the stage ack.
//! - **W-3** worker-death-after-stage → exactly-once (≤1 net world effect despite a
//!   re-dispatch, proven by a counting idempotent broker).
//! - **W-4** validate-then-commit distributed (plumbing + repudiation cascade; the
//!   promote/unpromote gate is a P1-default stub, activation deferred — see the test).
//! - **W-5** dedupe-on-late-commit (the dead worker's late commit → `AlreadyCommitted`).
//!
//! Determinism: an injected `Clock` drives death (W-3/W-5); the test brokers are
//! deterministic (fixed bytes → stable `ContentRef`). No wall-clock in the gating path.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use kx_capability::{
    idempotency_token_for, run_scoped_token, BrokerError, BrokerHandle, CapabilityBroker,
    EffectRequest, INSTANCE_ID_LEN,
};
use kx_content::{ContentRef, ContentStore, LocalFsContentStore};
use kx_coordinator::proto::coordinator_server::{Coordinator, CoordinatorServer};
use kx_coordinator::{
    Clock, CoordinatorService, InMemoryWorkerRegistry, MoteState, RepudiationReason, WorkerRegistry,
};
use kx_executor::{LocalResourceManager, MoteExecutor, TestMoteExecutor};
use kx_journal::InMemoryJournal;
use kx_mote::{EffectPattern, Mote, ToolName};
use kx_warrant::WarrantSpec;
use kx_worker::{Worker, WorkerClient};
use tempfile::TempDir;
use tonic::transport::Server;
use tonic::{Request, Response, Status};

const TIMEOUT: Duration = Duration::from_secs(6);

/// The deterministic effect bytes a test broker stages for a Mote.
fn wm_bytes(mote: &Mote) -> Vec<u8> {
    let mut v = b"wm-effect:".to_vec();
    v.extend_from_slice(mote.id.as_bytes());
    v
}

/// A `MoteExecutor` for the (unused) PURE path on a WM worker — and the real PURE path
/// for W-4's critic. Publishes bytes to the shared store, returning the ref.
fn storing_executor(store: Arc<LocalFsContentStore>) -> Arc<dyn MoteExecutor> {
    Arc::new(TestMoteExecutor::new(move |mote, _warrant| {
        let mut v = b"kx-result:".to_vec();
        v.extend_from_slice(mote.id.as_bytes());
        store.put(&v).expect("publish result bytes")
    }))
}

// ---------------------------------------------------------------------------
// Test brokers
// ---------------------------------------------------------------------------

/// Stages deterministic effect bytes into the shared store and returns the ref —
/// the minimal faithful WM dispatch (the executor's own WM integration tests use the
/// same shape). Content-addressed, so re-staging the same Mote yields the same ref.
struct StagingBroker {
    store: Arc<LocalFsContentStore>,
}
impl CapabilityBroker for StagingBroker {
    fn dispatch(
        &self,
        mote: &Mote,
        _warrant: &WarrantSpec,
        capability: &ToolName,
        _request: EffectRequest,
    ) -> Result<BrokerHandle, BrokerError> {
        let staged_ref = self.store.put(&wm_bytes(mote)).expect("stage effect bytes");
        Ok(BrokerHandle {
            staged_ref,
            capability: capability.clone(),
            capability_version: common::world_tool_version(),
        })
    }
    fn probe_readback(
        &self,
        _mote: &Mote,
        _warrant: &WarrantSpec,
        _capability: &ToolName,
        _probe: EffectRequest,
    ) -> Result<Option<BrokerHandle>, BrokerError> {
        Ok(None)
    }
}

/// Counts dispatches and **net** world effects: a dispatch with an already-seen
/// idempotency key (D38 §1 token) is a no-op at the world boundary (the tool dedupes),
/// so it does NOT bump `net_effects`. Proves "≤1 net effect despite a re-dispatch" (W-3).
#[derive(Clone)]
struct CountingBroker {
    store: Arc<LocalFsContentStore>,
    dispatch_calls: Arc<AtomicUsize>,
    net_effects: Arc<AtomicUsize>,
    applied_keys: Arc<Mutex<std::collections::BTreeSet<[u8; 32]>>>,
}
impl CountingBroker {
    fn new(store: Arc<LocalFsContentStore>) -> Self {
        Self {
            store,
            dispatch_calls: Arc::new(AtomicUsize::new(0)),
            net_effects: Arc::new(AtomicUsize::new(0)),
            applied_keys: Arc::new(Mutex::new(std::collections::BTreeSet::new())),
        }
    }
}
impl CapabilityBroker for CountingBroker {
    fn dispatch(
        &self,
        mote: &Mote,
        _warrant: &WarrantSpec,
        capability: &ToolName,
        request: EffectRequest,
    ) -> Result<BrokerHandle, BrokerError> {
        self.dispatch_calls.fetch_add(1, Ordering::SeqCst);
        // The tool-boundary idempotency key (== mote_id) makes a re-dispatch a no-op:
        // only a never-before-seen key is a real world effect.
        let key = request
            .idempotency_key
            .expect("WM dispatch carries a token");
        if self.applied_keys.lock().unwrap().insert(key) {
            self.net_effects.fetch_add(1, Ordering::SeqCst);
        }
        let staged_ref = self.store.put(&wm_bytes(mote)).expect("stage effect bytes");
        Ok(BrokerHandle {
            staged_ref,
            capability: capability.clone(),
            capability_version: common::world_tool_version(),
        })
    }
    fn probe_readback(
        &self,
        _mote: &Mote,
        _warrant: &WarrantSpec,
        _capability: &ToolName,
        _probe: EffectRequest,
    ) -> Result<Option<BrokerHandle>, BrokerError> {
        Ok(None)
    }
}

/// Records `"dispatched"` into a shared ordering log when it fires (W-2). Paired with
/// [`RecordingCoordinator`], which records `"staged"` — the log proves the worker stages
/// before it fires.
struct RecordingBroker {
    store: Arc<LocalFsContentStore>,
    log: Arc<Mutex<Vec<&'static str>>>,
}
impl CapabilityBroker for RecordingBroker {
    fn dispatch(
        &self,
        mote: &Mote,
        _warrant: &WarrantSpec,
        capability: &ToolName,
        _request: EffectRequest,
    ) -> Result<BrokerHandle, BrokerError> {
        self.log.lock().unwrap().push("dispatched");
        let staged_ref = self.store.put(&wm_bytes(mote)).expect("stage effect bytes");
        Ok(BrokerHandle {
            staged_ref,
            capability: capability.clone(),
            capability_version: common::world_tool_version(),
        })
    }
    fn probe_readback(
        &self,
        _mote: &Mote,
        _warrant: &WarrantSpec,
        _capability: &ToolName,
        _probe: EffectRequest,
    ) -> Result<Option<BrokerHandle>, BrokerError> {
        Ok(None)
    }
}

// ---------------------------------------------------------------------------
// Coordinator harness
// ---------------------------------------------------------------------------

/// A deterministic clock the test advances by hand (mirrors `tests/reschedule.rs`).
#[derive(Debug)]
struct FakeClock(AtomicU64);
impl FakeClock {
    fn new(ms: u64) -> Arc<Self> {
        Arc::new(Self(AtomicU64::new(ms)))
    }
    fn set(&self, ms: u64) {
        self.0.store(ms, Ordering::Relaxed);
    }
}
impl Clock for FakeClock {
    fn now_ms(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
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

async fn submit(svc: &CoordinatorService, mote: &Mote, warrant: &WarrantSpec) {
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

/// Serve `svc` over loopback gRPC; return the endpoint URL (the retrying [`connect`]
/// tolerates the brief bind→serve gap, mirroring the e2e harness).
fn serve<S>(svc: S) -> String
where
    S: Coordinator,
{
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    tokio::spawn(async move {
        Server::builder()
            .add_service(CoordinatorServer::new(svc))
            .serve(addr)
            .await
            .unwrap();
    });
    format!("http://{addr}")
}

/// Register a worker whose WM path uses `broker` (PURE path is the unused storing
/// executor). One round of `run_once` drives lease → stage → fire → propose.
async fn register_worker(
    endpoint: &str,
    store: Arc<LocalFsContentStore>,
    broker: Arc<dyn CapabilityBroker>,
    tag: &str,
) -> Worker {
    Worker::register(
        connect(endpoint).await,
        common::WORKER_CLASS,
        tag.to_string(),
        storing_executor(store.clone()),
        LocalResourceManager::dev_defaults(),
        store,
        broker,
        16,
    )
    .await
    .unwrap()
}

/// The committed `result_ref` of `mote` as the coordinator serves it over `ReadEntries`.
async fn committed_ref(client: &mut WorkerClient, mote: &Mote) -> ContentRef {
    let (entries, _next) = client.read_entries(0, 256).await.unwrap();
    entries
        .iter()
        .find_map(|e| match e.kind.as_ref().unwrap() {
            kx_coordinator::proto::journal_entry::Kind::Committed(c)
                if c.mote_id == mote.id.as_bytes().to_vec() =>
            {
                Some(ContentRef::from_bytes(
                    c.result_ref.clone().try_into().unwrap(),
                ))
            }
            _ => None,
        })
        .expect("coordinator serves the committed result_ref")
}

// ---------------------------------------------------------------------------
// W-1 — happy stage→fire→commit
// ---------------------------------------------------------------------------

#[tokio::test]
async fn w1_stage_then_commit_happy_path() {
    let dir = TempDir::new().unwrap();
    let store = Arc::new(LocalFsContentStore::open(dir.path()).unwrap());
    let svc = CoordinatorService::with_store(InMemoryJournal::new(), store.clone());
    let endpoint = serve(svc.clone());

    let wm = common::wm_mote(7, EffectPattern::StageThenCommit, &[]);
    submit(&svc, &wm, &common::wm_warrant()).await;

    let broker = Arc::new(StagingBroker {
        store: store.clone(),
    });
    let mut worker = register_worker(&endpoint, store.clone(), broker, "w1").await;

    assert_eq!(worker.run_once().await.unwrap(), 1, "the WM Mote committed");
    assert_eq!(svc.committed_count().await.unwrap(), 1);
    assert_eq!(svc.state_of(wm.id).await.unwrap(), MoteState::Committed);

    // The effect bytes the broker staged are in the shared store under the committed
    // result_ref (D55 data plane; the coordinator's phantom-ref guard accepted them).
    let mut observer = connect(&endpoint).await;
    let r = committed_ref(&mut observer, &wm).await;
    assert_eq!(store.get(&r).unwrap().to_vec(), wm_bytes(&wm));
}

// ---------------------------------------------------------------------------
// W-2 — ordering: stage ack BEFORE fire
// ---------------------------------------------------------------------------

/// Delegates every RPC to an inner [`CoordinatorService`], recording `"staged"` into a
/// shared log once a `ReportEffectStaged` is durably recorded. Paired with
/// [`RecordingBroker`] (records `"dispatched"`), the log order proves the worker awaits
/// the stage ack before firing.
#[derive(Clone)]
struct RecordingCoordinator {
    inner: CoordinatorService,
    log: Arc<Mutex<Vec<&'static str>>>,
}

#[tonic::async_trait]
impl Coordinator for RecordingCoordinator {
    async fn register_worker(
        &self,
        request: Request<kx_coordinator::proto::RegisterWorkerRequest>,
    ) -> Result<Response<kx_coordinator::proto::RegisterWorkerResponse>, Status> {
        self.inner.register_worker(request).await
    }
    async fn heartbeat(
        &self,
        request: Request<kx_coordinator::proto::HeartbeatRequest>,
    ) -> Result<Response<kx_coordinator::proto::HeartbeatResponse>, Status> {
        self.inner.heartbeat(request).await
    }
    async fn submit_mote(
        &self,
        request: Request<kx_coordinator::proto::SubmitMoteRequest>,
    ) -> Result<Response<kx_coordinator::proto::SubmitMoteResponse>, Status> {
        self.inner.submit_mote(request).await
    }
    async fn report_commit(
        &self,
        request: Request<kx_coordinator::proto::ReportCommitRequest>,
    ) -> Result<Response<kx_coordinator::proto::ReportCommitResponse>, Status> {
        self.inner.report_commit(request).await
    }
    async fn report_effect_staged(
        &self,
        request: Request<kx_coordinator::proto::ReportEffectStagedRequest>,
    ) -> Result<Response<kx_coordinator::proto::ReportEffectStagedResponse>, Status> {
        let resp = self.inner.report_effect_staged(request).await?;
        // Recorded only after the inner sole writer durably appended the EffectStaged.
        self.log.lock().unwrap().push("staged");
        Ok(resp)
    }
    async fn lease_work(
        &self,
        request: Request<kx_coordinator::proto::LeaseWorkRequest>,
    ) -> Result<Response<kx_coordinator::proto::LeaseWorkResponse>, Status> {
        self.inner.lease_work(request).await
    }
    async fn read_entries(
        &self,
        request: Request<kx_coordinator::proto::ReadEntriesRequest>,
    ) -> Result<Response<kx_coordinator::proto::ReadEntriesResponse>, Status> {
        self.inner.read_entries(request).await
    }
    async fn register_run(
        &self,
        request: Request<kx_coordinator::proto::RegisterRunRequest>,
    ) -> Result<Response<kx_coordinator::proto::RegisterRunResponse>, Status> {
        self.inner.register_run(request).await
    }
    async fn report_failure(
        &self,
        request: Request<kx_coordinator::proto::ReportFailureRequest>,
    ) -> Result<Response<kx_coordinator::proto::ReportFailureResponse>, Status> {
        self.inner.report_failure(request).await
    }
}

#[tokio::test]
async fn w2_does_not_fire_before_stage_ack() {
    let dir = TempDir::new().unwrap();
    let store = Arc::new(LocalFsContentStore::open(dir.path()).unwrap());
    let svc = CoordinatorService::with_store(InMemoryJournal::new(), store.clone());
    let log = Arc::new(Mutex::new(Vec::new()));
    let endpoint = serve(RecordingCoordinator {
        inner: svc.clone(),
        log: log.clone(),
    });

    let wm = common::wm_mote(8, EffectPattern::StageThenCommit, &[]);
    submit(&svc, &wm, &common::wm_warrant()).await;

    let broker = Arc::new(RecordingBroker {
        store: store.clone(),
        log: log.clone(),
    });
    let mut worker = register_worker(&endpoint, store.clone(), broker, "w2").await;

    assert_eq!(worker.run_once().await.unwrap(), 1);
    assert_eq!(
        log.lock().unwrap().as_slice(),
        ["staged", "dispatched"],
        "the worker recorded the durable EffectStaged BEFORE firing the effect (D58 §2)"
    );
}

// ---------------------------------------------------------------------------
// W-3 — worker-death-after-stage → exactly-once
// ---------------------------------------------------------------------------

#[tokio::test]
async fn w3_worker_death_after_stage_is_exactly_once() {
    let dir = TempDir::new().unwrap();
    let store = Arc::new(LocalFsContentStore::open(dir.path()).unwrap());
    let clock = FakeClock::new(1_000);
    let registry: Arc<dyn WorkerRegistry> = Arc::new(
        InMemoryWorkerRegistry::with_clock_and_timeout(clock.clone(), TIMEOUT),
    );
    let svc = CoordinatorService::with_store_and_registry(
        InMemoryJournal::new(),
        store.clone(),
        registry,
    );
    let endpoint = serve(svc.clone());

    let wm = common::wm_mote(11, EffectPattern::StageThenCommit, &[]);
    let warrant = common::wm_warrant();
    submit(&svc, &wm, &warrant).await;

    // One counting broker shared by the dying-worker simulation and the live worker, so
    // the applied-key set (tool-boundary idempotency) spans both dispatches.
    let broker = CountingBroker::new(store.clone());

    // --- The dying worker: register, lease, STAGE, FIRE once, then die (no commit). ---
    let mut dying = connect(&endpoint).await;
    let dying_id = dying
        .register_worker(common::WORKER_CLASS, "dying")
        .await
        .unwrap();
    let (leased, instance_id) = dying
        .lease_work(dying_id, common::WORKER_CLASS, 16)
        .await
        .unwrap();
    assert_eq!(leased.len(), 1, "dying worker leased the WM Mote");
    // M1.3: the run is registered, so the live worker derives a RUN-SCOPED
    // idempotency token from the leased `instance_id` (`run_scoped_token`). The
    // dying worker's manual fire must use the SAME token, or the broker won't
    // dedupe the re-fire and the world effect would happen twice.
    let iid: [u8; INSTANCE_ID_LEN] = instance_id
        .as_slice()
        .try_into()
        .expect("a registered run surfaces a 16-byte instance_id on lease");
    let id = *wm.id.as_bytes();
    dying.report_effect_staged(id, id, dying_id).await.unwrap();
    // It fired the effect (net effect #1) before crashing — the exact stage→fire→{die}
    // window the EffectStaged hint exists to make recoverable.
    let cap = common::world_tool();
    let req = EffectRequest {
        payload: Vec::new(),
        pattern: wm.effect_pattern(),
        idempotency_key: Some(run_scoped_token(&iid, &wm)),
        net_scope: kx_warrant::NetScope::None,
        fs_scope: kx_warrant::FsScope::empty(),
        secret_scope: kx_warrant::SecretScope::None,
    };
    broker.dispatch(&wm, &warrant, &cap, req).unwrap();
    assert_eq!(broker.net_effects.load(Ordering::SeqCst), 1);
    // dying worker is now silent forever.

    // --- Time advances past the liveness timeout; a live worker recovers it. ---
    clock.set(1_000 + 6_001);
    let mut worker =
        register_worker(&endpoint, store.clone(), Arc::new(broker.clone()), "live").await;
    // run_once: reap the dead lease → re-lease (D57) → re-stage (dedupe) → re-fire
    // (idempotent: key already applied) → commit.
    assert_eq!(
        worker.run_once().await.unwrap(),
        1,
        "the replacement committed"
    );

    assert_eq!(
        svc.committed_count().await.unwrap(),
        1,
        "exactly one commit"
    );
    assert_eq!(svc.state_of(wm.id).await.unwrap(), MoteState::Committed);
    assert_eq!(
        broker.dispatch_calls.load(Ordering::SeqCst),
        2,
        "the effect was re-dispatched after the death (so dedupe is what bounds it)"
    );
    assert_eq!(
        broker.net_effects.load(Ordering::SeqCst),
        1,
        "≤1 NET world effect despite two dispatches — exactly-once at the world boundary"
    );
}

// ---------------------------------------------------------------------------
// W-4 — validate-then-commit distributed (plumbing + cascade)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn w4_validate_then_commit_distributed_and_cascade() {
    let dir = TempDir::new().unwrap();
    let store = Arc::new(LocalFsContentStore::open(dir.path()).unwrap());
    let svc = CoordinatorService::with_store(InMemoryJournal::new(), store.clone());
    let endpoint = serve(svc.clone());

    // A VTC producer + its sibling critic (a PURE child gated on the producer).
    let producer = common::vtc_producer(20, &[]);
    let critic = common::critic(21, producer.id);
    submit(&svc, &producer, &common::wm_warrant()).await;
    submit(&svc, &critic, &common::pure_warrant()).await;

    let broker = Arc::new(StagingBroker {
        store: store.clone(),
    });
    let mut worker = register_worker(&endpoint, store.clone(), broker, "w4").await;

    // Round 1: the producer (WM) is ready → staged+fired+committed via the broker.
    assert_eq!(worker.run_once().await.unwrap(), 1, "producer committed");
    assert_eq!(
        svc.state_of(producer.id).await.unwrap(),
        MoteState::Committed
    );

    // Round 2: the critic is now ready (producer committed) → runs the PURE path
    // (run_pure through the executor) and commits its verdict. The worker did NOT
    // schedule it — the coordinator's ready_set did (D58 §6).
    assert_eq!(worker.run_once().await.unwrap(), 1, "critic committed");
    assert_eq!(svc.state_of(critic.id).await.unwrap(), MoteState::Committed);

    // The producer is NOT auto-failed by the verdict (P0.8): it stays Committed. The
    // promote/unpromote GATE (`promotion_state`) is a P1-default stub (always
    // NotApplicable) — its verdict-reading activation is a kx-projection change deferred
    // to its own ticket; here we assert the distributed plumbing + that no auto-fail
    // occurs.
    assert_eq!(
        svc.state_of(producer.id).await.unwrap(),
        MoteState::Committed
    );

    // Repudiating the producer cascades the poison-invalidation to its committed
    // downstream lineage (D22 / P3.5) — the critic, a committed consumer.
    let outcome = svc
        .repudiate(producer.id, RepudiationReason::OperatorAction, 42)
        .await
        .unwrap();
    assert_eq!(outcome.target, producer.id);
    assert_eq!(
        outcome.cascade_size, 1,
        "the critic is downstream of the producer"
    );
    assert_eq!(
        svc.state_of(producer.id).await.unwrap(),
        MoteState::Repudiated
    );
    assert_eq!(
        svc.state_of(critic.id).await.unwrap(),
        MoteState::Repudiated,
        "the cascade reaches the committed consumer"
    );
}

// ---------------------------------------------------------------------------
// W-5 — dedupe-on-late-commit
// ---------------------------------------------------------------------------

#[tokio::test]
async fn w5_dead_workers_late_commit_dedupes() {
    let dir = TempDir::new().unwrap();
    let store = Arc::new(LocalFsContentStore::open(dir.path()).unwrap());
    let clock = FakeClock::new(1_000);
    let registry: Arc<dyn WorkerRegistry> = Arc::new(
        InMemoryWorkerRegistry::with_clock_and_timeout(clock.clone(), TIMEOUT),
    );
    let svc = CoordinatorService::with_store_and_registry(
        InMemoryJournal::new(),
        store.clone(),
        registry,
    );
    let endpoint = serve(svc.clone());

    let wm = common::wm_mote(13, EffectPattern::StageThenCommit, &[]);
    let warrant = common::wm_warrant();
    submit(&svc, &wm, &warrant).await;
    let broker = CountingBroker::new(store.clone());

    // The dying worker stages + fires, then dies.
    let mut dying = connect(&endpoint).await;
    let dying_id = dying
        .register_worker(common::WORKER_CLASS, "dying")
        .await
        .unwrap();
    dying
        .lease_work(dying_id, common::WORKER_CLASS, 16)
        .await
        .unwrap();
    let id = *wm.id.as_bytes();
    dying.report_effect_staged(id, id, dying_id).await.unwrap();
    let staged = broker
        .dispatch(
            &wm,
            &warrant,
            &common::world_tool(),
            EffectRequest {
                payload: Vec::new(),
                pattern: wm.effect_pattern(),
                idempotency_key: Some(idempotency_token_for(&wm)),
                net_scope: kx_warrant::NetScope::None,
                fs_scope: kx_warrant::FsScope::empty(),
                secret_scope: kx_warrant::SecretScope::None,
            },
        )
        .unwrap();

    // The live worker recovers and commits.
    clock.set(1_000 + 6_001);
    let mut worker =
        register_worker(&endpoint, store.clone(), Arc::new(broker.clone()), "live").await;
    assert_eq!(worker.run_once().await.unwrap(), 1);
    assert_eq!(svc.committed_count().await.unwrap(), 1);

    // The "dead" worker was only slow: its late ReportCommit for the same Mote dedupes
    // (first-wins, D54) → AlreadyCommitted, committed count unchanged.
    let late = dying
        .report_commit(kx_coordinator::proto::ReportCommitRequest {
            mote_id: id.to_vec(),
            idempotency_key: id.to_vec(),
            result_ref: staged.staged_ref.as_bytes().to_vec(),
            warrant_ref: kx_warrant::warrant_ref_of(&warrant).as_bytes().to_vec(),
            mote_def_hash: wm.def.hash().as_bytes().to_vec(),
            nd_class: kx_coordinator::proto::NdClass::from(wm.nd_class()) as i32,
            parents: vec![],
            worker_id: dying_id,
        })
        .await
        .unwrap();
    assert_eq!(
        late.outcome,
        kx_coordinator::proto::CommitOutcome::AlreadyCommitted as i32,
        "the dead worker's late commit dedupes to the first"
    );
    assert_eq!(svc.committed_count().await.unwrap(), 1, "no double commit");
}
