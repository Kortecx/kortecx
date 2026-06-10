//! The deterministic cluster driver.
//!
//! Wraps one [`CoordinatorService`] (per seed) and drives it through **direct,
//! sequenced RPC-trait calls** — no gRPC transport, no autonomous worker tasks. A
//! single injected [`FakeClock`] is the only time source; the driver advances it by
//! hand to make a worker cross the liveness timeout (the W-3 death pattern), so a
//! reap + reschedule is triggered by the very next `LeaseWork`. Because each call is
//! awaited before the next is issued, the coordinator's owner thread sees commands in
//! exactly the driver's order — the run is a pure function of the plan.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use kx_capability::{idempotency_token_for, CapabilityBroker, EffectRequest};
use kx_content::ContentRef;
use kx_coordinator::proto::coordinator_server::Coordinator;
use kx_coordinator::{
    proto, Clock, CoordinatorService, InMemoryWorkerRegistry, MoteState, RepudiationReason,
    WorkerRegistry,
};
use kx_journal::InMemoryJournal;
use kx_mote::{Mote, MoteId};
use kx_warrant::{warrant_ref_of, FsScope, NetScope, WarrantSpec};
use tonic::Request;

use crate::broker::ChaosBroker;

/// Logical start time (ms). Arbitrary; the clock only ever moves forward by hand.
const START_MS: u64 = 1_000;
/// The liveness timeout the registry uses; the driver jumps past it to kill a worker.
const TIMEOUT: Duration = Duration::from_secs(6);
/// Lease cap per poll — comfortably above any one scenario's ready set.
const LEASE_MAX: u32 = 64;

/// A hand-advanced clock shared by the registry and the driver.
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

/// A step error — an unexpected coordinator/RPC fault. Gate-invariant violations are
/// reported separately (in `scenario`); this is only for "the infrastructure misbehaved".
pub(crate) type StepResult<T> = Result<T, String>;

/// One per-seed cluster: coordinator + clock + the shared counting broker.
pub(crate) struct Cluster {
    svc: CoordinatorService,
    clock: Arc<FakeClock>,
    broker: ChaosBroker,
    now: u64,
    timeout_ms: u64,
    worker_seq: u64,
}

impl Cluster {
    /// A fresh in-memory cluster (no content store ⇒ no phantom-ref guard ⇒ fully
    /// deterministic, no filesystem). The registry shares the driver's clock.
    pub(crate) fn new() -> Self {
        let clock = FakeClock::new(START_MS);
        let registry: Arc<dyn WorkerRegistry> = Arc::new(
            InMemoryWorkerRegistry::with_clock_and_timeout(clock.clone(), TIMEOUT),
        );
        let svc = CoordinatorService::with_registry(InMemoryJournal::new(), registry);
        let timeout_ms = u64::try_from(TIMEOUT.as_millis()).unwrap_or(6_000);
        Self {
            svc,
            clock,
            broker: ChaosBroker::new(),
            now: START_MS,
            timeout_ms,
            worker_seq: 0,
        }
    }

    /// The shared broker (read its net-effect / dispatch counters after a run).
    pub(crate) fn broker(&self) -> &ChaosBroker {
        &self.broker
    }

    /// Register a worker at the current clock time; returns its assigned id.
    pub(crate) async fn register(&mut self) -> StepResult<u64> {
        self.worker_seq += 1;
        let endpoint = format!("chaos://w{}", self.worker_seq);
        let resp = self
            .svc
            .register_worker(Request::new(proto::RegisterWorkerRequest {
                executor_class: proto::ExecutorClass::MacosSandbox as i32,
                endpoint,
            }))
            .await
            .map_err(|s| format!("register_worker: {s}"))?;
        Ok(resp.into_inner().worker_id)
    }

    /// Submit a Mote + warrant (registers it with the hosted scheduler). M1.3:
    /// the run is registered first (idempotent) so the submit passes the
    /// registration-before-submit gate; the chaos warrants carry empty
    /// `tool_grants` (→ resolution `Resolved([])`, no D66/R-10) and the WM Mote's
    /// `tool_contract` is non-empty (→ R-1 passes), so the gate admits every
    /// chaos Mote.
    pub(crate) async fn submit(&self, mote: &Mote, warrant: &WarrantSpec) -> StepResult<()> {
        self.svc
            .register_run(Request::new(proto::RegisterRunRequest {
                recipe_fingerprint: [0x5au8; 32].to_vec(),
            }))
            .await
            .map_err(|s| format!("register_run: {s}"))?;
        self.svc
            .submit_mote(Request::new(proto::SubmitMoteRequest {
                mote: Some(mote.clone().into()),
                warrant: Some(warrant.clone().into()),
                accept_at_least_once: false,
                react_seed: false,
            }))
            .await
            .map_err(|s| format!("submit_mote: {s}"))?;
        Ok(())
    }

    /// Lease ready work for `worker`, returning the leased Motes (decoded from the
    /// `WorkItem`s). Reaps dead workers first (D57), so this is also what triggers a
    /// reschedule after a death.
    pub(crate) async fn lease(&self, worker: u64) -> StepResult<Vec<Mote>> {
        let resp = self
            .svc
            .lease_work(Request::new(proto::LeaseWorkRequest {
                worker_id: worker,
                executor_class: proto::ExecutorClass::MacosSandbox as i32,
                max_motes: LEASE_MAX,
            }))
            .await
            .map_err(|s| format!("lease_work: {s}"))?;
        let mut motes = Vec::new();
        for item in resp.into_inner().items {
            let pm = item
                .mote
                .ok_or_else(|| "workitem missing mote".to_string())?;
            let mote: Mote = pm
                .try_into()
                .map_err(|_| "workitem mote failed to decode".to_string())?;
            motes.push(mote);
        }
        Ok(motes)
    }

    /// Record a WORLD-MUTATING Mote's staged intent (`EffectStaged` hint) for `worker`.
    pub(crate) async fn stage(&self, worker: u64, mote: &Mote) -> StepResult<()> {
        let id = mote.id.as_bytes().to_vec();
        self.svc
            .report_effect_staged(Request::new(proto::ReportEffectStagedRequest {
                mote_id: id.clone(),
                idempotency_key: id,
                worker_id: worker,
            }))
            .await
            .map_err(|s| format!("report_effect_staged: {s}"))?;
        Ok(())
    }

    /// Fire the effect through the shared broker (counts a dispatch; bumps net effects
    /// only for a never-before-seen idempotency key). Returns the staged ref to commit.
    pub(crate) fn fire(&self, mote: &Mote, warrant: &WarrantSpec) -> StepResult<ContentRef> {
        let request = EffectRequest {
            payload: Vec::new(),
            pattern: mote.def.effect_pattern,
            idempotency_key: Some(idempotency_token_for(mote)),
            net_scope: NetScope::None,
            fs_scope: FsScope::empty(),
            secret_scope: kx_warrant::SecretScope::None,
        };
        let handle = self
            .broker
            .dispatch(mote, warrant, &crate::workflow::world_tool(), request)
            .map_err(|e| format!("broker dispatch: {e:?}"))?;
        Ok(handle.staged_ref)
    }

    /// Propose a commit for `mote` by `worker` with the given `result_ref`. Returns the
    /// commit outcome (`Committed` / `AlreadyCommitted` — the latter is the dedup path).
    pub(crate) async fn commit(
        &self,
        worker: u64,
        mote: &Mote,
        warrant: &WarrantSpec,
        result_ref: ContentRef,
    ) -> StepResult<proto::CommitOutcome> {
        let id = mote.id.as_bytes().to_vec();
        let resp = self
            .svc
            .report_commit(Request::new(proto::ReportCommitRequest {
                mote_id: id.clone(),
                idempotency_key: id,
                result_ref: result_ref.as_bytes().to_vec(),
                warrant_ref: warrant_ref_of(warrant).as_bytes().to_vec(),
                mote_def_hash: mote.def.hash().as_bytes().to_vec(),
                nd_class: proto::NdClass::from(mote.def.nd_class) as i32,
                parents: mote.parents.iter().copied().map(Into::into).collect(),
                worker_id: worker,
            }))
            .await
            .map_err(|s| format!("report_commit: {s}"))?;
        proto::CommitOutcome::try_from(resp.into_inner().outcome)
            .map_err(|e| format!("unknown commit outcome: {e}"))
    }

    /// Jump the clock past the liveness timeout so any worker last seen at the current
    /// time is now `Dead` — the next `lease`/reap re-offers its in-flight Motes.
    pub(crate) fn advance_past_timeout(&mut self) {
        self.now += self.timeout_ms + 1;
        self.clock.set(self.now);
    }

    /// The committed (non-repudiated) Mote count in the coordinator's projection.
    pub(crate) async fn committed_count(&self) -> StepResult<usize> {
        self.svc
            .committed_count()
            .await
            .map_err(|e| format!("committed_count: {e}"))
    }

    /// The current [`MoteState`] of `id`.
    pub(crate) async fn state_of(&self, id: MoteId) -> StepResult<MoteState> {
        self.svc
            .state_of(id)
            .await
            .map_err(|e| format!("state_of: {e}"))
    }

    /// Repudiate `target` and cascade; returns the cascade size (downstream count).
    pub(crate) async fn repudiate(&self, target: MoteId) -> StepResult<usize> {
        self.svc
            .repudiate(target, RepudiationReason::OperatorAction, 0xC0)
            .await
            .map(|o| o.cascade_size)
            .map_err(|e| format!("repudiate: {e}"))
    }
}
