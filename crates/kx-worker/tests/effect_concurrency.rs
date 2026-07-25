//! EFFECT-QUEUE — a slow effect must not head-of-line-block the rest of a lease batch,
//! and the per-Mote wall-clock deadline must actually fire.
//!
//! Every test here is written to be **RED against the pre-queue worker** (a synchronous
//! `broker.dispatch` driven by a sequential `for item in items` loop) and GREEN after —
//! a safety test that passes either way is decoration. What each one breaks:
//!
//! - **G6** `slow_effect_does_not_block_a_ready_sibling` — two ready Motes must be in
//!   `invoke` *at the same time*. Sequential dispatch admits exactly one.
//! - **G5** `tool_deadline_fires_on_a_hung_effect` — `tokio::time::timeout` polls its
//!   inner future first and returns `Ok` when it is `Ready`, so a synchronous dispatch
//!   with no await point completes in ONE poll and the deadline never fires. Only a
//!   real await point (`spawn_blocking`) makes the guard live.
//! - **G3** `concurrent_dispatch_carries_a_distinct_idempotency_key_per_mote` —
//!   concurrency must not weaken the D38 §1 tool-boundary dedup, in either direction:
//!   a dropped key, or one key shared across two different effects.
//!
//! Determinism: no wall-clock in the gating path. The capabilities rendezvous over
//! channels (enter → park → release), so "concurrent" is proven by two entries observed
//! before either release, never by timing.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use kx_capability::{BrokerError, BrokerHandle, CapabilityBroker, EffectRequest, INSTANCE_ID_LEN};
use kx_content::{ContentRef, ContentStore, LocalFsContentStore};
use kx_coordinator::proto::coordinator_server::{Coordinator, CoordinatorServer};
use kx_coordinator::{CoordinatorService, MoteState};
use kx_executor::{LocalResourceManager, MoteExecutor, TestMoteExecutor};
use kx_journal::InMemoryJournal;
use kx_mote::{EffectPattern, Mote, MoteId, ToolName};
use kx_warrant::WarrantSpec;
use kx_worker::{Worker, WorkerClient};
use tempfile::TempDir;
use tonic::transport::Server;

/// How long a rendezvous may wait before the test declares the effect absent. A PASS
/// never spends it (a concurrent entry arrives in milliseconds); a FAIL is a real
/// absence, not a slow machine.
const RENDEZVOUS: Duration = Duration::from_secs(4);

/// The hard ceiling on how long a parked effect blocks, even if the test never releases
/// it. Bounded on purpose: a test that FAILS its rendezvous still has to let the parked
/// effect return, or dropping the tokio runtime would block forever and the failure
/// would present as a hang instead of a diagnosis.
const PARK_MAX: Duration = Duration::from_secs(8);

// ---------------------------------------------------------------------------
// Harness (mirrors tests/wm_dispatch.rs)
// ---------------------------------------------------------------------------

fn storing_executor(store: Arc<LocalFsContentStore>) -> Arc<dyn MoteExecutor> {
    Arc::new(TestMoteExecutor::new(move |mote, _warrant| {
        let mut v = b"kx-result:".to_vec();
        v.extend_from_slice(mote.id.as_bytes());
        store.put(&v).expect("publish result bytes")
    }))
}

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

async fn connect(endpoint: &str) -> WorkerClient {
    for _ in 0..100 {
        if let Ok(c) = WorkerClient::connect(endpoint.to_string()).await {
            return c;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("worker client could not reach the coordinator at {endpoint}");
}

async fn submit(svc: &CoordinatorService, mote: &Mote, warrant: &WarrantSpec) {
    use kx_coordinator::proto;
    // M1.3: register the run (idempotent) so the submit passes the
    // registration-before-submit gate.
    let _ = svc
        .register_run(tonic::Request::new(proto::RegisterRunRequest {
            recipe_fingerprint: vec![0x5au8; 32],
        }))
        .await;
    svc.submit_mote(tonic::Request::new(proto::SubmitMoteRequest {
        mote: Some(mote.clone().into()),
        warrant: Some(warrant.clone().into()),
        accept_at_least_once: false,
        react_seed: false,
    }))
    .await
    .unwrap();
}

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

// ---------------------------------------------------------------------------
// A broker whose effect announces itself, then parks until the test releases it
// ---------------------------------------------------------------------------

/// Announces `mote.id` on `entered` when the effect begins, then blocks on `release`
/// until the test lets it finish. This is the *whole* mechanism behind G6: with a
/// sequential batch loop only one Mote can ever be parked at a time.
struct ParkingBroker {
    store: Arc<LocalFsContentStore>,
    entered: std::sync::mpsc::SyncSender<MoteId>,
    release: Arc<Mutex<std::sync::mpsc::Receiver<()>>>,
    dispatches: Arc<AtomicUsize>,
    /// Net world effects: a dispatch whose D38 §1 idempotency key was already applied is
    /// a no-op at the world boundary, so it does NOT bump this (G3).
    net_effects: Arc<AtomicUsize>,
    applied_keys: Arc<Mutex<std::collections::BTreeSet<[u8; 32]>>>,
}

impl CapabilityBroker for ParkingBroker {
    fn dispatch(
        &self,
        mote: &Mote,
        _warrant: &WarrantSpec,
        capability: &ToolName,
        request: EffectRequest,
    ) -> Result<BrokerHandle, BrokerError> {
        self.dispatches.fetch_add(1, Ordering::SeqCst);
        let _ = self.entered.send(mote.id);
        // Park: the test holds every parked effect until it has observed as many
        // concurrent entries as it expects. Bounded by `PARK_MAX` so a FAILING test
        // still terminates (see the constant).
        let _ = self.release.lock().unwrap().recv_timeout(PARK_MAX);

        if let Some(key) = request.idempotency_key {
            if self.applied_keys.lock().unwrap().insert(key) {
                self.net_effects.fetch_add(1, Ordering::SeqCst);
            }
        } else {
            self.net_effects.fetch_add(1, Ordering::SeqCst);
        }

        let mut bytes = b"wm-effect:".to_vec();
        bytes.extend_from_slice(mote.id.as_bytes());
        // Return a typed failure rather than panicking if the store has gone away.
        //
        // Two tests here ABANDON an effect on purpose (the deadline and the orphan-guard
        // cases), so a parked dispatch legitimately outlives the test that started it —
        // by which point the `TempDir` backing this store is gone. Panicking on that
        // would put a dying thread inside a passing test: noise at best, and at worst a
        // future flake or a poisoned lock. The caller has already stopped waiting for
        // this result, so a refusal is both honest and inert.
        let staged_ref = self
            .store
            .put(&bytes)
            .map_err(|e| BrokerError::StageWriteFailed {
                capability: capability.clone(),
                diagnostic: format!("{e}"),
            })?;
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
// G6 — a slow effect must not head-of-line-block a ready sibling
// ---------------------------------------------------------------------------

/// Two independent ready WM Motes are leased in ONE batch (the coordinator's ready set
/// is "Pending, parents committed", so a batch never contains a parent and its child —
/// its items are mutually independent). Both must reach `invoke` **before either is
/// released**.
///
/// RED on the pre-queue worker: `for item in items` awaits each Mote's whole
/// stage→fire→commit chain, so the second `invoke` cannot begin until the first
/// returns — the test observes ONE entry and times out on the second.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn slow_effect_does_not_block_a_ready_sibling() {
    let dir = TempDir::new().unwrap();
    let store = Arc::new(LocalFsContentStore::open(dir.path()).unwrap());
    let svc = CoordinatorService::with_store(InMemoryJournal::new(), store.clone());
    let endpoint = serve(svc.clone());

    let a = common::wm_mote(11, EffectPattern::StageThenCommit, &[]);
    let b = common::wm_mote(12, EffectPattern::StageThenCommit, &[]);
    submit(&svc, &a, &common::wm_warrant()).await;
    submit(&svc, &b, &common::wm_warrant()).await;

    let (entered_tx, entered_rx) = std::sync::mpsc::sync_channel(8);
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let broker = Arc::new(ParkingBroker {
        store: store.clone(),
        entered: entered_tx,
        release: Arc::new(Mutex::new(release_rx)),
        dispatches: Arc::new(AtomicUsize::new(0)),
        net_effects: Arc::new(AtomicUsize::new(0)),
        applied_keys: Arc::new(Mutex::new(std::collections::BTreeSet::new())),
    });

    let mut worker = register_worker(&endpoint, store.clone(), broker, "hol").await;
    let driver = tokio::spawn(async move { worker.run_once().await });

    // BOTH effects must be in flight before either is released. This is the assertion
    // that fails on a sequential batch loop.
    let first = entered_rx
        .recv_timeout(RENDEZVOUS)
        .expect("the first effect must enter invoke");
    let second = entered_rx.recv_timeout(RENDEZVOUS).expect(
        "a second ready Mote must reach its effect while the first is still in flight \
         (one slow tool must not head-of-line-block the batch)",
    );
    assert_ne!(first, second, "two DISTINCT Motes must be in flight");
    assert!(
        [a.id, b.id].contains(&first) && [a.id, b.id].contains(&second),
        "the in-flight effects are the two submitted Motes"
    );

    // Let both finish and drain the batch.
    release_tx.send(()).unwrap();
    release_tx.send(()).unwrap();
    let committed = driver.await.unwrap().unwrap();
    assert_eq!(committed, 2, "both Motes commit");
    assert_eq!(svc.state_of(a.id).await.unwrap(), MoteState::Committed);
    assert_eq!(svc.state_of(b.id).await.unwrap(), MoteState::Committed);
}

// ---------------------------------------------------------------------------
// G5 — the per-Mote wall-clock deadline must actually fire
// ---------------------------------------------------------------------------

/// A capability that hangs must be abandoned at `KX_SERVE_TOOL_DEADLINE_SECS` so it
/// cannot pin a worker slot forever.
///
/// RED on the pre-queue worker: `broker.dispatch` is a synchronous call with no await
/// point, so `tokio::time::timeout` — which polls its inner future FIRST and returns
/// `Ok` the moment it is `Ready` — never observes the elapsed timer. The hung effect
/// runs to completion and the Mote commits, which is exactly what the deadline exists
/// to prevent. There is no other coverage of `ExecutionTimedOut` in the workspace.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tool_deadline_fires_on_a_hung_effect() {
    let dir = TempDir::new().unwrap();
    let store = Arc::new(LocalFsContentStore::open(dir.path()).unwrap());
    let svc = CoordinatorService::with_store(InMemoryJournal::new(), store.clone());
    let endpoint = serve(svc.clone());

    let hung = common::wm_mote(21, EffectPattern::StageThenCommit, &[]);
    submit(&svc, &hung, &common::wm_warrant()).await;

    let (entered_tx, entered_rx) = std::sync::mpsc::sync_channel(8);
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let broker = Arc::new(ParkingBroker {
        store: store.clone(),
        entered: entered_tx,
        release: Arc::new(Mutex::new(release_rx)),
        dispatches: Arc::new(AtomicUsize::new(0)),
        net_effects: Arc::new(AtomicUsize::new(0)),
        applied_keys: Arc::new(Mutex::new(std::collections::BTreeSet::new())),
    });

    let mut worker = register_worker(&endpoint, store.clone(), broker, "deadline")
        .await
        .with_tool_deadline(Some(Duration::from_millis(300)));

    // The batch must DRAIN while the effect is still parked — that is the deadline
    // firing. On the pre-queue worker this future does not resolve until the effect is
    // released, so the `timeout` below expires.
    let committed = tokio::time::timeout(RENDEZVOUS, worker.run_once())
        .await
        .expect(
            "run_once must return while the hung effect is still parked \
             (the per-Mote deadline must abandon it, not wait for it)",
        )
        .unwrap();

    entered_rx
        .recv_timeout(RENDEZVOUS)
        .expect("the effect did enter invoke");
    assert_eq!(committed, 0, "a timed-out effect commits nothing");
    assert_ne!(
        svc.state_of(hung.id).await.unwrap(),
        MoteState::Committed,
        "a Mote whose effect blew the deadline must NOT be committed"
    );

    release_tx.send(()).unwrap();
}

// ---------------------------------------------------------------------------
// G3 — concurrency must not weaken exactly-once
// ---------------------------------------------------------------------------

/// Every concurrently-dispatched Mote must still carry its OWN D38 §1 tool-boundary key.
///
/// That key is the whole basis of exactly-once at the world boundary: a re-dispatch after
/// a crash or a timeout is a no-op only because the tool recognises the key. A concurrent
/// pipeline can break it in two ways, and this pins both — a DROPPED key (nothing dedups
/// on a later re-fire) and a SHARED key (two genuinely different effects collapse into
/// one, silently losing work). Hoisting the request out of the per-item path would do
/// either, which is what makes this worth asserting here.
///
/// Exactly-once ACROSS a re-dispatch is proven separately, and serially, by
/// `wm_dispatch::w3_worker_death_after_stage_is_exactly_once`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_dispatch_carries_a_distinct_idempotency_key_per_mote() {
    let dir = TempDir::new().unwrap();
    let store = Arc::new(LocalFsContentStore::open(dir.path()).unwrap());
    let svc = CoordinatorService::with_store(InMemoryJournal::new(), store.clone());
    let endpoint = serve(svc.clone());

    let a = common::wm_mote(31, EffectPattern::StageThenCommit, &[]);
    let b = common::wm_mote(32, EffectPattern::StageThenCommit, &[]);
    submit(&svc, &a, &common::wm_warrant()).await;
    submit(&svc, &b, &common::wm_warrant()).await;

    let (entered_tx, _entered_rx) = std::sync::mpsc::sync_channel(64);
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    // Pre-arm the release channel so nothing parks: this test is about the keys, not
    // about rendezvous.
    for _ in 0..64 {
        release_tx.send(()).unwrap();
    }
    let net_effects = Arc::new(AtomicUsize::new(0));
    let dispatches = Arc::new(AtomicUsize::new(0));
    let applied_keys = Arc::new(Mutex::new(std::collections::BTreeSet::new()));
    let broker = Arc::new(ParkingBroker {
        store: store.clone(),
        entered: entered_tx,
        release: Arc::new(Mutex::new(release_rx)),
        dispatches: dispatches.clone(),
        net_effects: net_effects.clone(),
        applied_keys: applied_keys.clone(),
    });

    let mut worker = register_worker(&endpoint, store.clone(), broker, "keys").await;
    assert_eq!(worker.run_once().await.unwrap(), 2, "both Motes commit");

    assert_eq!(
        dispatches.load(Ordering::SeqCst),
        2,
        "both effects fired exactly once"
    );
    // The ParkingBroker records a key only when the request carried one, so a dropped key
    // shows up here as 0 and a shared key as 1.
    assert_eq!(
        applied_keys.lock().unwrap().len(),
        2,
        "each concurrently-dispatched Mote carried its OWN idempotency key \
         (0 = the key was dropped, 1 = the two Motes SHARED one)"
    );
    assert_eq!(
        net_effects.load(Ordering::SeqCst),
        2,
        "two distinct Motes are two distinct world effects — a shared key would \
         silently collapse them into one"
    );
    assert_eq!(svc.state_of(a.id).await.unwrap(), MoteState::Committed);
    assert_eq!(svc.state_of(b.id).await.unwrap(), MoteState::Committed);
}

// ---------------------------------------------------------------------------
// G7 — the deadline must not open a double-fire path
// ---------------------------------------------------------------------------

/// `spawn_blocking` cannot be cancelled: a timed-out dispatch is abandoned by its caller
/// but keeps running. `ExecutionTimedOut` is TRANSIENT, so the coordinator re-offers the
/// Mote — and the worker must REFUSE to fire it again while the abandoned effect is
/// still live.
///
/// This hazard did not exist before the effect queue only because the deadline never
/// fired. Making the guard real is what creates the obligation, so it ships with its own
/// guard.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_abandoned_effect_is_not_re_fired_while_it_is_still_running() {
    let dir = TempDir::new().unwrap();
    let store = Arc::new(LocalFsContentStore::open(dir.path()).unwrap());
    let svc = CoordinatorService::with_store(InMemoryJournal::new(), store.clone());
    let endpoint = serve(svc.clone());

    let m = common::wm_mote(41, EffectPattern::StageThenCommit, &[]);
    submit(&svc, &m, &common::wm_warrant()).await;

    let (entered_tx, entered_rx) = std::sync::mpsc::sync_channel(8);
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let dispatches = Arc::new(AtomicUsize::new(0));
    let broker = Arc::new(ParkingBroker {
        store: store.clone(),
        entered: entered_tx,
        release: Arc::new(Mutex::new(release_rx)),
        dispatches: dispatches.clone(),
        net_effects: Arc::new(AtomicUsize::new(0)),
        applied_keys: Arc::new(Mutex::new(std::collections::BTreeSet::new())),
    });

    let mut worker = register_worker(&endpoint, store.clone(), broker, "orphan")
        .await
        .with_tool_deadline(Some(Duration::from_millis(200)));

    // Round 1: the effect enters and the deadline abandons it — still running.
    worker.run_once().await.unwrap();
    entered_rx
        .recv_timeout(RENDEZVOUS)
        .expect("the effect entered invoke");
    assert_eq!(dispatches.load(Ordering::SeqCst), 1);

    // Rounds 2 and 3: the Mote is re-offered, and the worker must refuse it. A second
    // `invoke` here would be a genuine double-fire of a world-mutating effect.
    for round in 2..=3 {
        worker.run_once().await.unwrap();
        assert_eq!(
            dispatches.load(Ordering::SeqCst),
            1,
            "round {round} re-fired an effect whose abandoned dispatch is still running"
        );
    }
    assert_ne!(
        svc.state_of(m.id).await.unwrap(),
        MoteState::Committed,
        "nothing commits while the effect is unresolved"
    );

    release_tx.send(()).unwrap();
}

// ---------------------------------------------------------------------------
// The escape hatch — concurrency 1 is the pre-queue sequential batch
// ---------------------------------------------------------------------------

/// `KX_SERVE_EFFECT_CONCURRENCY=1` must genuinely serialize the batch, so an operator who
/// needs the old behaviour can have it. Proven the same way G6 proves the opposite: at
/// width 1 a second ready Mote must NOT reach its effect while the first is parked.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrency_of_one_is_the_sequential_batch() {
    let dir = TempDir::new().unwrap();
    let store = Arc::new(LocalFsContentStore::open(dir.path()).unwrap());
    let svc = CoordinatorService::with_store(InMemoryJournal::new(), store.clone());
    let endpoint = serve(svc.clone());

    let a = common::wm_mote(51, EffectPattern::StageThenCommit, &[]);
    let b = common::wm_mote(52, EffectPattern::StageThenCommit, &[]);
    submit(&svc, &a, &common::wm_warrant()).await;
    submit(&svc, &b, &common::wm_warrant()).await;

    let (entered_tx, entered_rx) = std::sync::mpsc::sync_channel(8);
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let broker = Arc::new(ParkingBroker {
        store: store.clone(),
        entered: entered_tx,
        release: Arc::new(Mutex::new(release_rx)),
        dispatches: Arc::new(AtomicUsize::new(0)),
        net_effects: Arc::new(AtomicUsize::new(0)),
        applied_keys: Arc::new(Mutex::new(std::collections::BTreeSet::new())),
    });

    let mut worker = register_worker(&endpoint, store.clone(), broker, "serial")
        .await
        .with_effect_concurrency(1);
    let driver = tokio::spawn(async move { worker.run_once().await });

    entered_rx
        .recv_timeout(RENDEZVOUS)
        .expect("the first effect enters");
    assert!(
        entered_rx.recv_timeout(Duration::from_millis(500)).is_err(),
        "at width 1 the second Mote must WAIT — this is the opt-out from the effect queue"
    );

    release_tx.send(()).unwrap();
    release_tx.send(()).unwrap();
    assert_eq!(driver.await.unwrap().unwrap(), 2, "both still commit");
}

/// The run-scoped token length is part of the contract the concurrent path must carry
/// through unchanged (a task that rebuilt the request would silently drop it).
#[test]
fn instance_id_len_is_the_documented_16() {
    assert_eq!(INSTANCE_ID_LEN, 16);
}

/// Referenced so the harness's unused-import lint stays honest about `ContentRef`.
#[test]
fn content_ref_is_32_bytes() {
    assert_eq!(ContentRef::from_bytes([0u8; 32]).as_bytes().len(), 32);
}
