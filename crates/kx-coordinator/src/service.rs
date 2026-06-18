//! [`CoordinatorService`] — the gRPC `Coordinator` server implementation.
//!
//! Holds the worker registry (behind a trait) and a handle to the single
//! orchestration core thread (which owns the journal + projection + hosted
//! scheduler). The four RPCs are thin adapters: convert at the untrusted boundary,
//! route to the registry or the core, map errors to [`tonic::Status`].

use std::sync::Arc;

use kx_audit::{AuditEvent, AuditSink, DispatchKind};
use kx_content::LocalFsContentStore;
use kx_journal::{FailureReason, Journal, JournalEntry};
use kx_mote::{Mote, MoteId, NdClass};
use kx_projection::MoteState;
use kx_proto::proto;
use kx_proto::proto::coordinator_server::Coordinator;
use kx_scheduler::WorkerId;
use kx_tool_registry::{InMemoryToolRegistry, ToolRegistry};
use kx_warrant::WarrantSpec;
use tonic::{Request, Response, Status};

use crate::clock::{Clock, SystemClock};
use crate::commit;
use crate::error::CoordinatorError;
use crate::nonce::{OsRandomNonce, RunNonceSource};
use crate::registry::{InMemoryWorkerRegistry, RegistryError, WorkerRegistry};
use crate::state::CoreHandle;

/// The coordinator gRPC service: hosts the scheduler, owns the worker registry,
/// and is the sole journal writer per run.
#[derive(Clone)]
pub struct CoordinatorService {
    core: CoreHandle,
    registry: Arc<dyn WorkerRegistry>,
    /// W1a (T-OBS1): the OPTIONAL off-truth-path operator audit sink. `None`
    /// (every constructor's default) ⇒ no audit emit, byte-identical to today.
    /// Set by the gateway via [`CoordinatorService::with_audit_sink`] for the
    /// long-running serve. Best-effort: `record` is infallible + never gates a run.
    audit: Option<Arc<dyn AuditSink>>,
}

impl CoordinatorService {
    /// Build a coordinator over `journal` with the default in-memory worker
    /// registry. Takes sole ownership of the journal (the single-writer handle).
    pub fn new<J: Journal + Send + 'static>(journal: J) -> Self {
        Self::build(journal, Arc::new(InMemoryWorkerRegistry::new()), None)
    }

    /// Build a coordinator over `journal` with a caller-supplied worker registry.
    pub fn with_registry<J: Journal + Send + 'static>(
        journal: J,
        registry: Arc<dyn WorkerRegistry>,
    ) -> Self {
        Self::build(journal, registry, None)
    }

    /// Build a coordinator that shares the content data plane with its workers:
    /// it **verifies each committed `result_ref` against `store`** before recording
    /// the commit (D55 phantom-ref guard — a worker cannot record a result it never
    /// published), and the same content-addressed store is where peers read results.
    pub fn with_store<J: Journal + Send + 'static>(
        journal: J,
        store: Arc<LocalFsContentStore>,
    ) -> Self {
        Self::build(
            journal,
            Arc::new(InMemoryWorkerRegistry::new()),
            Some(store),
        )
    }

    /// As [`with_store`](Self::with_store), but injects the live [`ToolRegistry`]
    /// the coordinator resolves a warrant's `tool_grants` against at the D66
    /// admission gate (PR-9a). The MODEL-FREE serve arms use this so a tool that
    /// was DIALED from an external MCP server, registered via `RegisterTool`, or
    /// bundled (echo) is resolvable against the SAME durable registry the broker +
    /// the `RegisterTool` admin write into — closing the D66-on-a-model-free-serve
    /// gap (the inference-shaper arm already shares the live registry via
    /// [`with_store_shaper_and_tools`]). The topology/materialization path stays
    /// inert (no shaper roles), byte-identical to [`with_store`](Self::with_store).
    ///
    /// [`with_store_shaper_and_tools`]: CoordinatorService::with_store_shaper_and_tools
    pub fn with_store_and_tools<J: Journal + Send + 'static>(
        journal: J,
        store: Arc<LocalFsContentStore>,
        tool_registry: Arc<dyn ToolRegistry>,
    ) -> Self {
        Self::with_tool_registry_and_seams(
            journal,
            Arc::new(InMemoryWorkerRegistry::new()),
            Some(store),
            Arc::new(SystemClock),
            Arc::new(OsRandomNonce),
            tool_registry,
        )
    }

    /// Build a coordinator with **both** a caller-supplied worker registry and the
    /// shared content store (the `with_registry` + `with_store` combination). Used
    /// where a test needs a clock-injected registry (worker-death detection, P3.1)
    /// *and* the store's phantom-ref guard.
    pub fn with_store_and_registry<J: Journal + Send + 'static>(
        journal: J,
        store: Arc<LocalFsContentStore>,
        registry: Arc<dyn WorkerRegistry>,
    ) -> Self {
        Self::build(journal, registry, Some(store))
    }

    /// Build a coordinator with **injected run-registration seams** (M1.1, D64):
    /// the run-id nonce source + the wall clock that stamps the `RunRegistered`
    /// timestamp. Tests inject a fixed nonce + clock so the registered run
    /// identity is deterministic; production uses [`OsRandomNonce`] +
    /// [`SystemClock`] via the other constructors.
    pub fn with_seams<J: Journal + Send + 'static>(
        journal: J,
        registry: Arc<dyn WorkerRegistry>,
        store: Option<Arc<LocalFsContentStore>>,
        clock: Arc<dyn Clock>,
        nonce: Arc<dyn RunNonceSource>,
    ) -> Self {
        // Default tool registry = the OSS built-ins (M1.2 resolve-at-submit
        // capture, D79). Use [`with_tool_registry_and_seams`] to inject a custom
        // registry (e.g. a test that registers its own tools to assert capture).
        Self::with_tool_registry_and_seams(
            journal,
            registry,
            store,
            clock,
            nonce,
            Arc::new(InMemoryToolRegistry::with_builtins()),
        )
    }

    /// As [`with_seams`], but injects the [`ToolRegistry`] the coordinator
    /// resolves the warrant's `tool_grants` against at submit (M1.2/D79). The
    /// resolved versions are captured as off-DAG run metadata (a
    /// `RunVersionsResolved` journal fact). Tests inject a custom registry to
    /// assert the captured tuples; production uses the built-ins default.
    ///
    /// [`with_seams`]: CoordinatorService::with_seams
    pub fn with_tool_registry_and_seams<J: Journal + Send + 'static>(
        journal: J,
        registry: Arc<dyn WorkerRegistry>,
        store: Option<Arc<LocalFsContentStore>>,
        clock: Arc<dyn Clock>,
        nonce: Arc<dyn RunNonceSource>,
        tool_registry: Arc<dyn ToolRegistry>,
    ) -> Self {
        Self {
            core: CoreHandle::spawn(
                journal,
                store,
                registry.clone(),
                clock,
                nonce,
                tool_registry,
                // No shaper materialization on the default path — byte-identical to the
                // pre-PR-2b behavior (`kx run`, non-inference serve: no model fan-out).
                None,
            ),
            registry,
            audit: None,
        }
    }

    /// Build a coordinator that runs the LIVE model-driven agentic loop (PR-2b/T1.1): a
    /// committed topology shaper's children are materialized into the projection + dispatch
    /// admission set so they actually reach a worker. `shaper_roles` is the role registry
    /// the materializer narrows each child's warrant against (SN-8 — the model proposes a
    /// role name, the registry maps it to a vetted warrant, `intersect` narrows). Requires
    /// `store` (the shared content plane the committed `TopologyDecision` lives in). Used by
    /// the gateway under `--features inference`; every other constructor passes no role
    /// registry, leaving the topology path inert.
    #[allow(clippy::too_many_arguments)]
    pub fn with_shaper_materialization<J: Journal + Send + 'static>(
        journal: J,
        registry: Arc<dyn WorkerRegistry>,
        store: Arc<LocalFsContentStore>,
        clock: Arc<dyn Clock>,
        nonce: Arc<dyn RunNonceSource>,
        tool_registry: Arc<dyn ToolRegistry>,
        shaper_roles: Arc<dyn kx_warrant::RoleRegistry>,
    ) -> Self {
        Self {
            core: CoreHandle::spawn(
                journal,
                Some(store),
                registry.clone(),
                clock,
                nonce,
                tool_registry,
                Some(shaper_roles),
            ),
            registry,
            audit: None,
        }
    }

    /// [`with_store`](Self::with_store) + live shaper materialization (PR-2b): the
    /// store's phantom-ref guard plus the `shaper_roles` registry, with the default
    /// in-memory worker registry / system clock / OS nonce / built-in tool registry. The
    /// constructor the gateway uses under `--features inference`.
    pub fn with_store_and_shaper_materialization<J: Journal + Send + 'static>(
        journal: J,
        store: Arc<LocalFsContentStore>,
        shaper_roles: Arc<dyn kx_warrant::RoleRegistry>,
    ) -> Self {
        Self::with_shaper_materialization(
            journal,
            Arc::new(InMemoryWorkerRegistry::new()),
            store,
            Arc::new(SystemClock),
            Arc::new(OsRandomNonce),
            Arc::new(InMemoryToolRegistry::with_builtins()),
            shaper_roles,
        )
    }

    /// As [`with_store_and_shaper_materialization`], but injects the
    /// [`ToolRegistry`] (PR-2d-2): the live `kx serve` registers the bundled
    /// stdio tool's `ToolDef` so the settle's validate-at-freeze and the
    /// lease-time args re-derivation resolve it. Every other caller keeps the
    /// built-ins default.
    ///
    /// [`with_store_and_shaper_materialization`]: CoordinatorService::with_store_and_shaper_materialization
    pub fn with_store_shaper_and_tools<J: Journal + Send + 'static>(
        journal: J,
        store: Arc<LocalFsContentStore>,
        shaper_roles: Arc<dyn kx_warrant::RoleRegistry>,
        tool_registry: Arc<dyn ToolRegistry>,
    ) -> Self {
        Self::with_shaper_materialization(
            journal,
            Arc::new(InMemoryWorkerRegistry::new()),
            store,
            Arc::new(SystemClock),
            Arc::new(OsRandomNonce),
            tool_registry,
            shaper_roles,
        )
    }

    fn build<J: Journal + Send + 'static>(
        journal: J,
        registry: Arc<dyn WorkerRegistry>,
        store: Option<Arc<LocalFsContentStore>>,
    ) -> Self {
        Self::with_seams(
            journal,
            registry,
            store,
            Arc::new(SystemClock),
            Arc::new(OsRandomNonce),
        )
    }

    /// W1a (T-OBS1): attach an OFF-TRUTH-PATH operator audit sink (builder style,
    /// like the runtime's `RuntimeAuditSink`). The serve coordinator then emits the
    /// admitted / committed / failed lifecycle as best-effort [`AuditEvent`]s. The
    /// gateway supplies a `JsonlAuditSink` from `--audit-log`; every other caller
    /// leaves it `None` (no emit). NEVER gates a run — `record` is infallible.
    #[must_use]
    pub fn with_audit_sink(mut self, sink: Arc<dyn AuditSink>) -> Self {
        self.audit = Some(sink);
        self
    }

    /// Emit one lifecycle event to the audit sink, if one is attached. A no-op when
    /// `None`, so the off-truth-path guarantee holds (no time/identity recomputation;
    /// the sink only echoes already-derived join keys).
    fn audit(&self, event: AuditEvent) {
        if let Some(sink) = &self.audit {
            sink.record(event);
        }
    }

    /// Read-side accessor: the current [`MoteState`] of `mote_id` in the
    /// coordinator's projection (the journal's folded read view).
    pub async fn state_of(&self, mote_id: MoteId) -> Result<MoteState, CoordinatorError> {
        self.core.state_of(mote_id).await
    }

    /// Read-side accessor: the number of `Committed` (non-repudiated) Motes.
    pub async fn committed_count(&self) -> Result<usize, CoordinatorError> {
        self.core.committed_count().await
    }

    /// Read-side accessor: the current ready set — submitted Motes whose parents
    /// are all `Committed`-and-not-`Repudiated`. The dispatch surface P2.3 consumes.
    pub async fn ready_set(&self) -> Result<Vec<MoteId>, CoordinatorError> {
        self.core.ready_set().await
    }

    /// Borrow the worker registry (diagnostics / operator queries).
    #[must_use]
    pub fn registry(&self) -> &dyn WorkerRegistry {
        self.registry.as_ref()
    }

    /// Read-side accessor: the registered run identity (D64) as
    /// `(instance_id, recipe_fingerprint)`, or `None` if the run has not been
    /// registered. Read from the folded projection — on recovery this returns the
    /// journaled fact, never a recomputed value (the run-resume handle M2 builds on).
    pub async fn run_registration(&self) -> Result<Option<([u8; 16], [u8; 32])>, CoordinatorError> {
        self.core.run_registration().await
    }

    /// Read-side accessor: the resolved-version run metadata captured at submit
    /// (M1.2, D79) — one [`RunResolvedVersions`] record per resolved capability
    /// (a zero-grant warrant contributes one with no capability). Audit/lineage
    /// only; off the truth path. (The observability query M11 builds on this.)
    ///
    /// [`RunResolvedVersions`]: kx_projection::RunResolvedVersions
    pub async fn run_resolved_versions(
        &self,
    ) -> Result<Vec<kx_projection::RunResolvedVersions>, CoordinatorError> {
        self.core.run_resolved_versions().await
    }

    /// Repudiate `target` and cascade the poison-invalidation to its committed downstream
    /// consumers (D22 / P0.7): one `Repudiated` entry per Mote (the target with `reason`,
    /// the cascade with `UpstreamCascade`), written atomically through the sole writer, so
    /// the next `LeaseWork` no longer offers any of them. Returns how many downstream Motes
    /// the cascade repudiated. The operator-facing RPC / CLI is P4.5; this is the in-process
    /// mechanism (the "distributed cascade mechanics" P0.7 deferred to P3.5).
    pub async fn repudiate(
        &self,
        target: MoteId,
        reason: crate::RepudiationReason,
        repudiator_id: u128,
    ) -> Result<crate::RepudiationOutcome, crate::RepudiationError> {
        self.core.repudiate(target, reason, repudiator_id).await
    }
}

#[tonic::async_trait]
impl Coordinator for CoordinatorService {
    #[tracing::instrument(skip_all)]
    async fn register_worker(
        &self,
        request: Request<proto::RegisterWorkerRequest>,
    ) -> Result<Response<proto::RegisterWorkerResponse>, Status> {
        let req = request.into_inner();
        let proto_class = proto::ExecutorClass::try_from(req.executor_class).map_err(|_| {
            Status::invalid_argument(format!("unknown executor_class {}", req.executor_class))
        })?;
        let executor_class =
            kx_warrant::ExecutorClass::try_from(proto_class).map_err(CoordinatorError::from)?;
        let id = self.registry.register(executor_class, req.endpoint);
        tracing::info!(worker_id = id.0, ?executor_class, "worker registered");
        Ok(Response::new(proto::RegisterWorkerResponse {
            worker_id: id.0,
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn heartbeat(
        &self,
        request: Request<proto::HeartbeatRequest>,
    ) -> Result<Response<proto::HeartbeatResponse>, Status> {
        let req = request.into_inner();
        self.registry
            .heartbeat(WorkerId(req.worker_id), req.timestamp_ms, req.in_flight)
            .map_err(|RegistryError::UnknownWorker(worker)| {
                CoordinatorError::UnknownWorker(worker)
            })?;
        Ok(Response::new(proto::HeartbeatResponse { ack: true }))
    }

    #[tracing::instrument(skip_all)]
    async fn submit_mote(
        &self,
        request: Request<proto::SubmitMoteRequest>,
    ) -> Result<Response<proto::SubmitMoteResponse>, Status> {
        let req = request.into_inner();
        // M1.3/D38 §2c: per-Mote opt-in to dispatch an AtLeastOnce WM tool.
        let accept_at_least_once = req.accept_at_least_once;
        // PR-2d-1: seed a live ReAct chain (the coordinator swaps in the
        // run-salted turn 0 + anchors a durable ReactRound fact).
        let react_seed = req.react_seed;
        // IDENTITY INVARIANT (D53): `TryFrom<proto::Mote>` re-derives the MoteId
        // Rust-side; the wire `mote_id` is advisory and never trusted.
        let mote: Mote = req
            .mote
            .ok_or_else(|| Status::invalid_argument("SubmitMote.mote is required"))?
            .try_into()
            .map_err(CoordinatorError::from)?;
        let warrant: WarrantSpec = req
            .warrant
            .ok_or_else(|| Status::invalid_argument("SubmitMote.warrant is required"))?
            .try_into()
            .map_err(CoordinatorError::from)?;
        // The coordinator-derived identity (D53) — captured before the move so
        // it can ride a REJECTED response when the submit is refused (M1.3).
        let mote_id = mote.id.as_bytes().to_vec();
        // W1a (T-OBS1): the nd_class for the admitted-Mote audit line (captured
        // before the move into `submit`). Off the truth path — echoed, never recomputed.
        let nd_class = mote.nd_class();

        match self
            .core
            .submit(mote, warrant, accept_at_least_once, react_seed)
            .await
        {
            Ok(outcome) => {
                let status = if outcome.duplicate {
                    proto::SubmitStatus::Duplicate
                } else {
                    proto::SubmitStatus::Accepted
                };
                // W1a (T-OBS1): a genuinely-new admission is an operator-relevant
                // lifecycle event (a duplicate re-submit is not — the journal already
                // has it). Mapped to MoteDispatched (the Mote entered the runtime for
                // execution); the kind is derived from nd_class (no recovery context
                // at submit). Best-effort + off the digest. NOTE: this is the
                // CLIENT-submission admission line; coordinator-MATERIALIZED agentic
                // children (shaper children, ReAct/re-plan turns) are spliced onto the
                // sole-writer thread (state.rs materialize_*) and so are audited via
                // their report_commit/report_failure (MoteCommitted/MoteFailed) only —
                // every durable outcome is captured; a per-child dispatch line for the
                // agentic loop is an additive follow-on (threads the sink into core).
                if !outcome.duplicate {
                    let kind = if matches!(nd_class, NdClass::Pure) {
                        DispatchKind::Pure
                    } else {
                        DispatchKind::WmFresh
                    };
                    self.audit(AuditEvent::MoteDispatched {
                        mote_id: outcome.mote_id,
                        nd_class,
                        kind,
                    });
                }
                Ok(Response::new(proto::SubmitMoteResponse {
                    mote_id: outcome.mote_id.as_bytes().to_vec(),
                    status: status as i32,
                    detail: String::new(),
                    // M1.2/D64: the registered run this Mote was admitted under
                    // (the resume key). M1.3 forces registration-before-submit,
                    // so an accepted Mote always carries a real instance_id.
                    instance_id: outcome
                        .instance_id
                        .map(|id| id.to_vec())
                        .unwrap_or_default(),
                    refusal_code: String::new(),
                }))
            }
            // M1.3: a submission-refusal predicate fired on a WELL-FORMED request
            // (R-1/R-7/R-8/R-14/R-15/R-10/D66). The proto contract is a structured
            // SUBMIT_STATUS_REJECTED response carrying the refusal detail — not a
            // transport error. instance_id is empty (nothing was admitted/written).
            Err(CoordinatorError::SubmissionRefused(refusal)) => {
                Ok(Response::new(proto::SubmitMoteResponse {
                    mote_id,
                    status: proto::SubmitStatus::Rejected as i32,
                    detail: refusal.to_string(),
                    instance_id: Vec::new(),
                    // PR-2: the STRUCTURED code alongside the prose — the
                    // gateway forwards it as `kx-refusal-code` metadata so no
                    // client ever parses the detail string.
                    refusal_code: refusal.code().to_string(),
                }))
            }
            // RunNotRegistered → failed_precondition (an ordering violation, sibling
            // of RunAlreadyStarted); CoreUnavailable → unavailable; durable faults →
            // internal — all via the CoordinatorError → Status mapping.
            Err(other) => Err(other.into()),
        }
    }

    #[tracing::instrument(skip_all)]
    async fn report_commit(
        &self,
        request: Request<proto::ReportCommitRequest>,
    ) -> Result<Response<proto::ReportCommitResponse>, Status> {
        let req = request.into_inner();
        // D40 admission: only a registered worker may propose a commit.
        let worker = WorkerId(req.worker_id);
        if self.registry.get(worker).is_none() {
            return Err(CoordinatorError::UnknownWorker(worker).into());
        }
        let proposal = commit::assemble(req)?;
        // W1a (T-OBS1): capture the commit's join keys before the proposal moves
        // into the sole-writer core, so a NEWLY-committed Mote can be audited below.
        let (audit_mote_id, audit_result_ref, audit_nd) =
            (proposal.mote_id, proposal.result_ref, proposal.nd_class);
        let applied = self.core.commit(proposal).await?;
        let outcome = if applied.already_committed {
            proto::CommitOutcome::AlreadyCommitted
        } else {
            proto::CommitOutcome::Committed
        };
        // W1a (T-OBS1): emit the durable-effect audit line only for a genuinely new
        // commit (an already-committed re-report is not a new fact). Off the digest.
        if !applied.already_committed {
            self.audit(AuditEvent::MoteCommitted {
                mote_id: audit_mote_id,
                result_ref: audit_result_ref,
                nd_class: audit_nd,
            });
        }
        tracing::info!(
            seq = applied.committed_seq,
            already_committed = applied.already_committed,
            "commit recorded"
        );
        Ok(Response::new(proto::ReportCommitResponse {
            committed_seq: applied.committed_seq,
            outcome: outcome as i32,
            detail: String::new(),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn report_effect_staged(
        &self,
        request: Request<proto::ReportEffectStagedRequest>,
    ) -> Result<Response<proto::ReportEffectStagedResponse>, Status> {
        let req = request.into_inner();
        // D40 admission: only a registered worker may stage an effect (mirrors report_commit).
        let worker = WorkerId(req.worker_id);
        if self.registry.get(worker).is_none() {
            return Err(CoordinatorError::UnknownWorker(worker).into());
        }
        let mote_id_bytes: [u8; 32] = req
            .mote_id
            .as_slice()
            .try_into()
            .map_err(|_| Status::invalid_argument("mote_id must be 32 bytes"))?;
        let idempotency_key: [u8; 32] = req
            .idempotency_key
            .as_slice()
            .try_into()
            .map_err(|_| Status::invalid_argument("idempotency_key must be 32 bytes"))?;
        let mote_id = MoteId::from_bytes(mote_id_bytes);
        let staged_seq = self
            .core
            .report_effect_staged(mote_id, idempotency_key)
            .await?;
        tracing::info!(seq = staged_seq, ?mote_id, "effect staged");
        Ok(Response::new(proto::ReportEffectStagedResponse {
            staged_seq,
            ack: true,
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn report_failure(
        &self,
        request: Request<proto::ReportFailureRequest>,
    ) -> Result<Response<proto::ReportFailureResponse>, Status> {
        let req = request.into_inner();
        // D40 admission: only a registered worker may report a failure (mirrors
        // report_commit/report_effect_staged). The per-Mote lease check is enforced
        // deeper, on the owner thread (`dead_letter_failure`).
        let worker = WorkerId(req.worker_id);
        if self.registry.get(worker).is_none() {
            return Err(CoordinatorError::UnknownWorker(worker).into());
        }
        let mote_id_bytes: [u8; 32] = req
            .mote_id
            .as_slice()
            .try_into()
            .map_err(|_| Status::invalid_argument("mote_id must be 32 bytes"))?;
        let idempotency_key: [u8; 32] = req
            .idempotency_key
            .as_slice()
            .try_into()
            .map_err(|_| Status::invalid_argument("idempotency_key must be 32 bytes"))?;
        let mote_id = MoteId::from_bytes(mote_id_bytes);
        // Fail-closed reason boundary: a worker may self-report ONLY a terminal-logic
        // verdict. A pre-commit-crash reason (TimedOut/WorkerCrashed) is coordinator-
        // observed, never worker-self-reported — accepting one here would (under an
        // EffectStaged) leave the Mote re-dispatchable forever (the F4 hang). The proto
        // enum's UNSPECIFIED=0 sentinel is rejected by `worker_reportable_reason`.
        let reason_class = worker_reportable_reason(req.reason_class)?;
        let (failed_seq, appended) = self
            .core
            .report_failure(mote_id, idempotency_key, reason_class, worker)
            .await?;
        // W1a (T-OBS1): a freshly-appended terminal failure is an operator-relevant
        // event (a deduped re-report is not). Off the truth path; never gates the run.
        if appended {
            self.audit(AuditEvent::MoteFailed { mote_id });
        }
        tracing::info!(
            seq = failed_seq,
            appended,
            ?mote_id,
            "worker dead-letter recorded"
        );
        Ok(Response::new(proto::ReportFailureResponse {
            failed_seq,
            ack: true,
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn lease_work(
        &self,
        request: Request<proto::LeaseWorkRequest>,
    ) -> Result<Response<proto::LeaseWorkResponse>, Status> {
        let req = request.into_inner();
        // Admission: only a registered worker may lease (mirrors report_commit).
        let worker = WorkerId(req.worker_id);
        if self.registry.get(worker).is_none() {
            return Err(CoordinatorError::UnknownWorker(worker).into());
        }
        let proto_class = proto::ExecutorClass::try_from(req.executor_class).map_err(|_| {
            Status::invalid_argument(format!("unknown executor_class {}", req.executor_class))
        })?;
        let executor_class =
            kx_warrant::ExecutorClass::try_from(proto_class).map_err(CoordinatorError::from)?;
        let max = usize::try_from(req.max_motes).unwrap_or(usize::MAX);
        let (work, instance_id) = self.core.lease_work(worker, executor_class, max).await?;
        let items = work
            .into_iter()
            .map(|item| proto::WorkItem {
                mote: Some(item.mote.into()),
                warrant: Some(item.warrant.into()),
                // F-7 (assemble-into-serve): the leaf's resolved Data context, resolved
                // on the sole-writer thread (`lease_ready` → `resolve_parent_context`).
                // Empty for a Mote with no Data context ⇒ byte-identical to pre-F-7.
                parent_results: item
                    .parent_results
                    .into_iter()
                    .map(|(parent_mote_id, result_ref)| proto::ParentResult {
                        parent_mote_id: parent_mote_id.as_bytes().to_vec(),
                        result_ref: result_ref.as_bytes().to_vec(),
                    })
                    .collect(),
                // PR-2d-2 (react-tools-live): the coordinator-validated args +
                // the resolved tool's declared egress for a ReAct observation,
                // re-derived on the sole-writer thread at lease time
                // (`resolve_tool_args`). `None` for every other Mote — the
                // legacy WM/leaf wire payload is byte-identical.
                tool_args: item.tool_args.map(|(args_bytes, net_scope, fs_scope)| {
                    proto::ToolArgs {
                        args_bytes,
                        net_scope: Some(net_scope.into()),
                        // PR-6a/D155 (fs-list): the resolved tool's fs requirement
                        // (empty for echo ⇒ byte-identical to PR-2d-2).
                        fs_scope: Some(fs_scope.into()),
                    }
                }),
            })
            .collect();
        Ok(Response::new(proto::LeaseWorkResponse {
            items,
            // M1.2/D64: the run the leased work belongs to, so the worker derives
            // the run-scoped idempotency token. Empty for an unregistered run.
            instance_id: instance_id.map(|id| id.to_vec()).unwrap_or_default(),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn read_entries(
        &self,
        request: Request<proto::ReadEntriesRequest>,
    ) -> Result<Response<proto::ReadEntriesResponse>, Status> {
        let req = request.into_inner();
        // Cap the page so one poll cannot ask for an unbounded response; `0` means
        // "coordinator default". The journal-read itself is bounded by current_seq.
        let max = match usize::try_from(req.max).unwrap_or(READ_ENTRIES_MAX) {
            0 => READ_ENTRIES_DEFAULT,
            n => n.min(READ_ENTRIES_MAX),
        };
        let (entries, next_seq) = self.core.read_entries(req.since_seq, max).await?;
        let entries = entries.into_iter().filter_map(committed_to_proto).collect();
        Ok(Response::new(proto::ReadEntriesResponse {
            entries,
            next_seq,
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn register_run(
        &self,
        request: Request<proto::RegisterRunRequest>,
    ) -> Result<Response<proto::RegisterRunResponse>, Status> {
        let req = request.into_inner();
        // The recipe_fingerprint is a client-supplied 32-byte hash (discovery/dedup
        // only — never identity, so the client MAY compute it, unlike a MoteId).
        let recipe_fingerprint: [u8; 32] = req
            .recipe_fingerprint
            .as_slice()
            .try_into()
            .map_err(|_| Status::invalid_argument("recipe_fingerprint must be 32 bytes"))?;
        // The coordinator assigns the instance_id (D53: identity is server-side).
        let instance_id = self.core.register_run(recipe_fingerprint).await?;
        tracing::info!("run registered");
        Ok(Response::new(proto::RegisterRunResponse {
            instance_id: instance_id.to_vec(),
        }))
    }
}

/// Default page size when a client passes `max = 0`.
const READ_ENTRIES_DEFAULT: usize = 256;
/// Hard ceiling on a single `ReadEntries` page (bounds response size).
const READ_ENTRIES_MAX: usize = 4096;

/// Map a `Committed` journal entry to its wire form. Returns `None` for any other
/// entry kind (the core already filters to `Committed`, so this is just an
/// exhaustiveness guard) — the wire `JournalEntry.oneof` carries only `Committed`
/// in P2.4 (D55; other kinds are reserved for P3).
fn committed_to_proto(entry: JournalEntry) -> Option<proto::JournalEntry> {
    match entry {
        JournalEntry::Committed {
            mote_id,
            idempotency_key,
            seq,
            nondeterminism,
            result_ref,
            parents,
            warrant_ref,
            mote_def_hash,
        } => {
            let parents = parents
                .into_iter()
                .filter_map(|p| p.to_parent_ref().map(proto::ParentRef::from))
                .collect();
            Some(proto::JournalEntry {
                seq,
                kind: Some(proto::journal_entry::Kind::Committed(
                    proto::CommittedEntry {
                        mote_id: mote_id.as_bytes().to_vec(),
                        idempotency_key: idempotency_key.to_vec(),
                        seq,
                        nd_class: proto::NdClass::from(nondeterminism) as i32,
                        result_ref: result_ref.as_bytes().to_vec(),
                        parents,
                        warrant_ref: warrant_ref.as_bytes().to_vec(),
                        mote_def_hash: mote_def_hash.as_bytes().to_vec(),
                    },
                )),
            })
        }
        _ => None,
    }
}

/// Map a wire `FailureReason` to its domain value, fail-closed to the reasons a worker
/// may LEGITIMATELY self-report (F4 dead-letter): a TERMINAL-LOGIC verdict only. A
/// pre-commit-crash reason (`TimedOut`/`WorkerCrashed`) is coordinator-observed (the
/// death reaper), and accepting one from a worker would, under an `EffectStaged`, keep
/// the Mote re-dispatchable forever — the F4 hang. `UNSPECIFIED` (the proto3 zero
/// default) and every non-worker-reportable reason are rejected as `invalid_argument`.
/// (The proto<->domain map lives here, not in kx-proto, which by design does not depend
/// on kx-journal — a failure is assembled into a journal entry coordinator-side.)
// `tonic::Status` (the gRPC-idiomatic error) is large; the Ok arm is a 1-byte enum, so
// the ratio trips `result_large_err`. Boxing the error would just force every gRPC call
// site to unbox — Status is the right type at this boundary.
#[allow(clippy::result_large_err)]
fn worker_reportable_reason(wire: i32) -> Result<FailureReason, Status> {
    let proto_reason = proto::FailureReason::try_from(wire)
        .map_err(|_| Status::invalid_argument(format!("unknown failure reason {wire}")))?;
    match proto_reason {
        proto::FailureReason::DeadLettered => Ok(FailureReason::DeadLettered),
        proto::FailureReason::ExecutorRefused => Ok(FailureReason::ExecutorRefused),
        other => Err(Status::invalid_argument(format!(
            "failure reason {other:?} is not worker-self-reportable (terminal-logic only)"
        ))),
    }
}
