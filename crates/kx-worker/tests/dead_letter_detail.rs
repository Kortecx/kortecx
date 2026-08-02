//! The journal-v18 model-facing DETAIL, across the worker→coordinator RPC boundary.
//!
//! ## The incident
//!
//! `JournalEntry::Failed` carries a `detail`: the diagnostic the FAILING SUBSYSTEM
//! itself produced, so a tool that named its own failure can have those words reach
//! the model. The coordinator renders it into the next ReAct turn's re-prompt. The
//! executor's in-process commit protocol populates it. **The distributed worker path
//! does not** — `Worker::dead_letter` takes only a `MoteId`, so the `&WorkerError`
//! that is still in scope one frame up in `handle_execution_failure` is discarded
//! before the report is sent, and the coordinator writes `detail: String::new()`.
//!
//! That path is not an exotic deployment: `kx serve` hosts the embedded worker, and
//! `kx-gateway` does not depend on `kx-runtime` at all — so EVERY served run's
//! terminal failure loses its diagnosis here.
//!
//! ## Why this is an integration test over a real port and not a unit test
//!
//! An accessor that returns the right string proves nothing about what survives the
//! RPC — the drop is AT the boundary, in a request message that has no field for it.
//! So this drives the real `Worker` against a real `CoordinatorService` over loopback
//! gRPC and reads the durable `Failed` entry back out of the journal file.
//!
//! ## The three arms are a consistency check on each other
//!
//! They differ in ONE variable — what the broker refuses with — and therefore in
//! whether the failure has a model-facing detail at all:
//!
//! | arm | broker refusal | `model_facing_detail` | `is_permanent` |
//! |---|---|---|---|
//! | `other`   | `CapabilityFailure(Other(..))` | the downstream system's own words | false ⇒ retried to the budget |
//! | `closed`  | `CapabilityFailure(AuthDenied)` | a fixed actionable sentence      | true  ⇒ dead-letters at once  |
//! | `control` | `UnknownCapability`             | `""` (runtime-side)              | true  ⇒ dead-letters at once  |
//!
//! The `control` arm is the ACCEPTING control: it must stay green through the fix. A
//! repair that rendered `format!("{e:?}")` unconditionally would satisfy the first two
//! and redden this one, which is precisely the failure mode worth catching — the
//! runtime's own operator-facing text is not model-facing, and the split between
//! `reason` and `model_detail` exists to keep it out of a prompt.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::sync::Arc;
use std::time::Duration;

use kx_capability::{
    BrokerError, BrokerHandle, CapabilityBroker, CapabilityFailureReason, EffectRequest,
};
use kx_content::{ContentStore, LocalFsContentStore};
use kx_coordinator::proto::coordinator_server::{Coordinator, CoordinatorServer};
use kx_coordinator::{CoordinatorService, MoteState};
use kx_executor::{LocalResourceManager, MoteExecutor, TestMoteExecutor};
use kx_journal::{Journal, JournalEntry, SqliteJournal};
use kx_mote::{EffectPattern, Mote, MoteId, ToolName};
use kx_warrant::WarrantSpec;
use kx_worker::{Worker, WorkerClient};
use tempfile::TempDir;
use tonic::transport::Server;
use tonic::Request;

/// The downstream system's own words — an MCP server naming the argument the model got
/// wrong. Chosen because it is the case the class-derived steer actively contradicts:
/// "it failed to run — do not call it again with the same arguments" is the opposite of
/// the fix, which is to change ONE argument.
const DOWNSTREAM_DETAIL: &str = "MCP error -32602: unknown field `cursr`, did you mean `cursor`?";

/// What `BrokerError::model_facing_detail` renders for the closed `AuthDenied` arm.
/// Pinned as a literal rather than computed from the enum: deriving the expectation
/// from the code under test would compare a thing to itself.
const AUTH_DENIED_DETAIL: &str = "the external system denied authentication";

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// What the broker under test refuses with. One knob, so the arms differ in exactly
/// one variable.
#[derive(Clone, Copy)]
enum Refusal {
    /// A downstream failure carrying the server's own free-text diagnostic.
    DownstreamOther,
    /// A downstream failure in the CLOSED vocabulary (permanent).
    DownstreamAuthDenied,
    /// A RUNTIME-side refusal: this machine's own state, never model-facing.
    RuntimeUnknownCapability,
}

impl Refusal {
    fn error(self, capability: &ToolName) -> BrokerError {
        match self {
            Self::DownstreamOther => BrokerError::CapabilityFailure {
                capability: capability.clone(),
                reason: CapabilityFailureReason::Other(DOWNSTREAM_DETAIL.to_string()),
            },
            Self::DownstreamAuthDenied => BrokerError::CapabilityFailure {
                capability: capability.clone(),
                reason: CapabilityFailureReason::AuthDenied,
            },
            Self::RuntimeUnknownCapability => BrokerError::UnknownCapability {
                name: capability.clone(),
            },
        }
    }
}

/// A broker that refuses every dispatch with a chosen [`Refusal`]. The refusal is the
/// only behaviour — the effect never fires, so there is nothing to make idempotent.
struct RefusingBroker {
    refusal: Refusal,
}

impl CapabilityBroker for RefusingBroker {
    fn dispatch(
        &self,
        _mote: &Mote,
        _warrant: &WarrantSpec,
        capability: &ToolName,
        _request: EffectRequest,
    ) -> Result<BrokerHandle, BrokerError> {
        Err(self.refusal.error(capability))
    }

    fn probe_readback(
        &self,
        _mote: &Mote,
        _warrant: &WarrantSpec,
        capability: &ToolName,
        _probe: EffectRequest,
    ) -> Result<Option<BrokerHandle>, BrokerError> {
        Err(self.refusal.error(capability))
    }
}

fn storing_executor(store: Arc<LocalFsContentStore>) -> Arc<dyn MoteExecutor> {
    Arc::new(TestMoteExecutor::new(move |mote, _warrant| {
        let mut v = b"kx-result:".to_vec();
        v.extend_from_slice(mote.id.as_bytes());
        store.put(&v).expect("publish result bytes")
    }))
}

fn serve<S: Coordinator>(svc: S) -> String {
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
    panic!("worker connects to the coordinator");
}

async fn submit(svc: &CoordinatorService, mote: &Mote, warrant: &WarrantSpec) {
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

/// The `detail` on the durable `Failed` entry for `mote`, read from the journal FILE —
/// the same bytes the coordinator's projection folds and `failure_detail_of` serves.
/// A second reader on the SQLite WAL, exactly as `react_live.rs` reads react facts.
fn failed_detail(dir: &TempDir, mote: MoteId) -> String {
    let journal = SqliteJournal::open(dir.path().join("journal.db")).unwrap();
    let head = journal.current_seq().unwrap();
    let found = journal
        .read_entries_by_seq(0..head + 1)
        .unwrap()
        .find_map(|e| match e {
            JournalEntry::Failed {
                mote_id, detail, ..
            } if mote_id == mote => Some(detail),
            _ => None,
        });
    found.expect("a terminal Failed entry for the dead-lettered Mote")
}

/// Submit one WORLD-MUTATING Mote, run a worker whose broker refuses with `refusal`
/// until the Mote is terminal, and return the `detail` on its `Failed` entry.
///
/// The loop is bounded rather than fixed-count on purpose: a TRANSIENT refusal
/// dead-letters only after the worker's retry budget, a PERMANENT one on the first
/// attempt, and the arms deliberately span both. The bound is the assertion that it
/// terminates at all.
async fn dead_letter_detail_for(refusal: Refusal) -> String {
    let dir = TempDir::new().unwrap();
    let store = Arc::new(LocalFsContentStore::open(dir.path().join("content")).unwrap());
    let journal = SqliteJournal::open(dir.path().join("journal.db")).unwrap();
    let svc = CoordinatorService::with_store(journal, store.clone());
    let endpoint = serve(svc.clone());

    let wm = common::wm_mote(11, EffectPattern::StageThenCommit, &[]);
    submit(&svc, &wm, &common::wm_warrant()).await;

    let mut worker = Worker::register(
        connect(&endpoint).await,
        common::WORKER_CLASS,
        "inproc://worker-detail",
        storing_executor(store.clone()),
        LocalResourceManager::dev_defaults(),
        store,
        Arc::new(RefusingBroker { refusal }),
        16,
    )
    .await
    .unwrap();

    for _ in 0..8 {
        // Never an Err: a refused dispatch dead-letters, it does not abort the batch.
        worker
            .run_once()
            .await
            .expect("a refused dispatch must not abort the worker batch");
        if svc.state_of(wm.id).await.unwrap() == MoteState::Failed {
            return failed_detail(&dir, wm.id);
        }
    }
    panic!("the refused Mote never reached a terminal Failed state");
}

// ---------------------------------------------------------------------------
// The arms
// ---------------------------------------------------------------------------

/// ★ THE INCIDENT. An MCP server that says WHICH argument is wrong must have those
/// words survive the worker→coordinator RPC onto the durable `Failed` fact — that
/// fact is the only thing the coordinator can render into the model's next prompt.
#[tokio::test(flavor = "multi_thread")]
async fn a_downstream_systems_own_diagnostic_survives_the_report_to_the_coordinator() {
    let detail = dead_letter_detail_for(Refusal::DownstreamOther).await;
    assert_eq!(
        detail, DOWNSTREAM_DETAIL,
        "the failing subsystem's own words must reach the durable Failed fact; \
         an empty detail here is the model being told nothing it can act on"
    );
}

/// The CLOSED-vocabulary sibling: a permanent credential refusal dead-letters on the
/// first attempt and still carries its (fixed, actionable) detail. Distinct from the
/// arm above in BOTH the permanence and the text, so a fix that only handled the
/// retried path would redden here.
#[tokio::test(flavor = "multi_thread")]
async fn a_permanent_credential_refusal_carries_its_detail_on_the_first_attempt() {
    let detail = dead_letter_detail_for(Refusal::DownstreamAuthDenied).await;
    assert_eq!(
        detail, AUTH_DENIED_DETAIL,
        "a closed-vocabulary downstream refusal renders an actionable sentence"
    );
}

/// ★ THE ACCEPTING CONTROL — one variable changed, and it must stay green through the
/// fix. `UnknownCapability` is the RUNTIME talking about ITSELF (this machine's
/// registry), so `model_facing_detail` is `""` and the model keeps the unchanged
/// class-derived steer. A repair that forwarded the operator-facing `reason` (which
/// can name host paths and local configuration) instead of the model-facing subset
/// would pass the two arms above and fail this one.
#[tokio::test(flavor = "multi_thread")]
async fn a_runtime_side_refusal_contributes_no_model_facing_detail() {
    let detail = dead_letter_detail_for(Refusal::RuntimeUnknownCapability).await;
    assert_eq!(
        detail, "",
        "a refusal about THIS machine's own state is not model-facing; \
         the model keeps the class-derived steer"
    );
}
