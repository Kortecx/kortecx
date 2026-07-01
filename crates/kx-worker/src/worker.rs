//! [`Worker`] — registers with the coordinator, then leases / runs / proposes.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use kx_capability::{CapabilityBroker, INSTANCE_ID_LEN};
use kx_content::{ContentRef, ContentStore, LocalFsContentStore};
use kx_executor::{LocalResourceManager, MoteExecutor};
use kx_mote::{ConfigKey, Mote, MoteId, NdClass, REACT_TURN_KEY, RERANK_TURN_KEY};
use kx_proto::proto;
use kx_warrant::{ExecutorClass, WarrantSpec};
use tokio::task::JoinHandle;

use crate::client::WorkerClient;
use crate::context_sink::ContextSink;
use crate::error::{classify_worker_failure, FailureClass, WorkerError};
use crate::read_model::ReadModel;
use crate::{commit_builder, run, run_wm};

/// `ReadEntries` page size when folding the local read model.
const READ_PAGE: u32 = 256;

/// How many times the worker retries a Mote whose execution fails *transiently*
/// before giving up and dead-lettering it (F4). A *terminal* failure (a deterministic
/// logic error — e.g. the shaper executor's fail-closed verdict on a malformed model
/// proposal, or a body that always non-zero-exits) is dead-lettered on the FIRST
/// observation, never retried. This bound is what turns the old "re-lease a failing
/// Mote forever" spin into a clean, bounded terminal `Failed`.
const WORKER_MAX_ATTEMPTS: u32 = 3;

/// Default cadence for the background liveness heartbeat ([`Worker::spawn_heartbeat`]).
///
/// Kept well under the coordinator's liveness timeout
/// (`kx_coordinator::DEFAULT_LIVENESS_TIMEOUT` = 6 s): the **invariant** is
/// `coordinator_timeout >= 3 * cadence`, so two dropped/late heartbeats do not trip
/// a false worker-death (the P0.9 stuck-vs-dead policy — never declare a
/// slow-but-alive worker dead).
pub const DEFAULT_HEARTBEAT_CADENCE: Duration = Duration::from_secs(2);

/// A registered worker bound to one coordinator. Holds the hosted executor + a
/// resource manager (the verbatim P1 execution stack), the shared content store it
/// reads peer results from (data plane), and a local read model of committed results
/// folded incrementally from the coordinator's log (so reads stay off the hot path).
pub struct Worker {
    client: WorkerClient,
    id: u64,
    executor_class: ExecutorClass,
    executor: Arc<dyn MoteExecutor>,
    resource_manager: LocalResourceManager,
    store: Arc<LocalFsContentStore>,
    /// Fires WORLD-MUTATING / READ-ONLY-NONDET effects (P3.6b, D58): staging the
    /// response bytes into the shared `store` (data plane). PURE Motes never touch it.
    /// One worker can run both classes, so the field is always present even on a
    /// PURE-only worker (it simply has no capabilities registered).
    broker: Arc<dyn CapabilityBroker>,
    read_model: ReadModel,
    max_lease: u32,
    /// The worker's live in-flight Mote count — the single source of truth for the
    /// load it reports. [`run_once`](Self::run_once) updates it around execution; the
    /// background heartbeat ([`spawn_heartbeat`](Self::spawn_heartbeat)) reads it, so
    /// liveness *and* load (D56 placement) stay accurate even while idle.
    in_flight: Arc<AtomicU32>,
    /// Per-Mote *transient* execution-failure counter (F4). In-memory, off the truth
    /// path: it bounds how many times a Mote whose execution fails transiently is
    /// retried before the worker dead-letters it. A success or a dead-letter clears the
    /// entry. A worker restart resets it harmlessly — the bound is a sanity ceiling, not
    /// durable state (the coordinator's terminal `Failed` is the durable truth).
    attempts: std::collections::HashMap<MoteId, u32>,
    /// F-7 (assemble-into-serve): the optional side-channel that hands a leased Mote's
    /// resolved Data context (`WorkItem.parent_results`) to the executor *before* each
    /// dispatch (the frozen `MoteExecutor::run` carries no snapshot). `None` for a worker
    /// whose executor does not assemble — its dispatch is byte-identical to pre-F-7.
    context_sink: Option<Arc<dyn ContextSink>>,
}

impl Worker {
    /// Register `client` with the coordinator as a worker for `executor_class`,
    /// reachable at `endpoint`, and return a ready worker. `executor` +
    /// `resource_manager` host the P1 execution stack verbatim; `store` is the shared
    /// content-addressed store (the worker's executor publishes results to it and the
    /// worker reads peer results from it); `broker` fires WORLD-MUTATING effects,
    /// staging their bytes into that same `store` (P3.6b, D58); `max_lease` bounds how
    /// many Motes a single [`run_once`](Self::run_once) pulls.
    // A wide constructor that injects the worker's full dependency set in one place
    // (transport, backend, resources, data plane, effect surface). Bundling these into a
    // config struct would be churn for no clarity gain (Rule 1) — the args are distinct,
    // named, and set exactly once at registration.
    #[allow(clippy::too_many_arguments)]
    pub async fn register(
        mut client: WorkerClient,
        executor_class: ExecutorClass,
        endpoint: impl Into<String>,
        executor: Arc<dyn MoteExecutor>,
        resource_manager: LocalResourceManager,
        store: Arc<LocalFsContentStore>,
        broker: Arc<dyn CapabilityBroker>,
        max_lease: u32,
    ) -> Result<Self, WorkerError> {
        let id = client.register_worker(executor_class, endpoint).await?;
        Ok(Self {
            client,
            id,
            executor_class,
            executor,
            resource_manager,
            store,
            broker,
            read_model: ReadModel::new(),
            max_lease,
            in_flight: Arc::new(AtomicU32::new(0)),
            attempts: std::collections::HashMap::new(),
            context_sink: None,
        })
    }

    /// Attach an F-7 [`ContextSink`] (assemble-into-serve): before each dispatch the
    /// worker hands the leased Mote's resolved Data context to `sink`, which the model
    /// executor consumes inside `run`. The gateway clones ONE `Arc` into both the
    /// `MoteExecutor` and the `ContextSink` role, so the slot is shared. Additive —
    /// a worker without a sink behaves byte-identically to pre-F-7.
    #[must_use]
    pub fn with_context_sink(mut self, sink: Arc<dyn ContextSink>) -> Self {
        self.context_sink = Some(sink);
        self
    }

    /// The coordinator-assigned worker id.
    #[must_use]
    pub fn worker_id(&self) -> u64 {
        self.id
    }

    /// Read a peer's committed result: fold the coordinator's committed-entry log
    /// into the local read model until `mote_id`'s commit is seen, resolve its
    /// `result_ref`, and fetch the bytes from the shared content store. This is the
    /// distributed-read path (D55) — the journal stays single-writer, the content
    /// store is the shared data plane.
    pub async fn peer_read(&mut self, mote_id: MoteId) -> Result<Vec<u8>, WorkerError> {
        loop {
            if let Some(result_ref) = self.read_model.result_ref_of(&mote_id) {
                let bytes = self
                    .store
                    .get(&result_ref)
                    .map_err(|_| WorkerError::ContentMissing(result_ref))?;
                return Ok(bytes.to_vec());
            }
            let cursor = self.read_model.cursor();
            let (entries, next_seq) = self.client.read_entries(cursor, READ_PAGE).await?;
            self.read_model.fold(entries, next_seq);
            if next_seq == cursor {
                // Caught up to current_seq without finding the commit.
                return Err(WorkerError::NotCommitted(mote_id));
            }
        }
    }

    /// Lease one batch of ready Motes, dispatch each (PURE recomputes through the
    /// hosted executor; non-PURE stages-then-fires via the broker, P3.6b/D58), and
    /// propose its commit. Returns the number of commits the coordinator accepted
    /// this round (0 when no ready work matches).
    pub async fn run_once(&mut self) -> Result<usize, WorkerError> {
        let (items, instance_id_bytes) = self
            .client
            .lease_work(self.id, self.executor_class, self.max_lease)
            .await?;
        // M1.2/D64: the registered run this batch belongs to. A 16-byte id ⇒ the
        // worker derives a run-scoped cross-boundary token; empty (unregistered
        // run) ⇒ fall back to the MoteId-only token.
        let instance_id: Option<[u8; INSTANCE_ID_LEN]> = instance_id_bytes.try_into().ok();

        // Report load so the coordinator's placement (D56) can balance across workers:
        // `in_flight` = the batch we're about to run, reset to 0 when it drains.
        // Publish to the shared counter (the background heartbeat reads it too), then
        // send an immediate heartbeat so placement sees the load without waiting for
        // the next tick. Best-effort — a heartbeat hiccup must never abort execution.
        let in_flight = u32::try_from(items.len()).unwrap_or(u32::MAX);
        self.in_flight.store(in_flight, Ordering::Relaxed);
        if in_flight > 0 {
            let _ = self.client.heartbeat(self.id, now_ms(), in_flight).await;
        }

        let mut committed = 0usize;
        for item in items {
            let mote: Mote = item
                .mote
                .ok_or(WorkerError::MissingField("mote"))?
                .try_into()?;
            let warrant: WarrantSpec = item
                .warrant
                .ok_or(WorkerError::MissingField("warrant"))?
                .try_into()?;

            // F-7 (assemble-into-serve): hand the executor this Mote's out-of-band
            // context BEFORE dispatch (the frozen `MoteExecutor::run` carries no snapshot).
            self.deliver_executor_context(
                mote.id,
                &item.parent_results,
                &item.context_items,
                &item.image_ref,
            );

            // PURE recomputes locally through the hosted executor (verbatim, D40 — a
            // throwaway journal). Non-PURE (WORLD-MUTATING / READ-ONLY-NONDET) drives
            // stage→fire→commit via RPCs + the broker (P3.6b, D58 §4): the worker is not
            // the journal writer, so it cannot run `run_wm_mote`. Either path yields the
            // `result_ref` (= the broker's `staged_ref` for non-PURE) the worker PROPOSES.
            //
            // F4: a per-Mote execution failure must NOT `?`-abort the whole batch — that
            // discards the rest of the lease AND re-leases the failing Mote forever (the
            // spin PR-9b's startup probe only narrowly patched). Instead classify it:
            // a terminal failure dead-letters now; a transient one retries within
            // `WORKER_MAX_ATTEMPTS`, then dead-letters. Either way we `continue` to the
            // next item. Transport/RPC errors on the lease/commit calls stay batch-level.
            let exec = if mote.nd_class() == NdClass::Pure {
                run::run_pure(&mote, &warrant, &*self.executor, &self.resource_manager)
            } else if dispatches_through_executor(&mote) {
                // PR-2d-2: a coordinator-materialized ReAct TURN (the identity-
                // bearing marker, NO tool_contract) is a prompt-carrying ROND
                // model Mote — it dispatches through the hosted EXECUTOR (whose
                // react arm decodes + fences pre-commit), never the capability
                // broker (it proposes; the observation Mote fires). Direct
                // dispatch matches the IdempotentByConstruction pattern.
                //
                // T-AGENT2: the opt-in LLM-JUDGE critic is the SAME shape — a ROND
                // model Mote the executor's `run_judge` arm grades + commits as a
                // verdict (no broker effect). A *native* critic is `Pure` (caught by
                // the first arm ⇒ `run_pure`), so `critic_check.is_some()` HERE
                // uniquely identifies the ReadOnlyNondet judge; the executor routes
                // both react turns and judges through this direct dispatch.
                run::run_react_turn(&mote, &warrant, &*self.executor)
            } else {
                // PR-2d-2: the coordinator-validated args + egress for a ReAct
                // observation (`WorkItem.tool_args`) — decoded here, consumed by
                // `run_wm` into the `EffectRequest`. A malformed wire NetScope
                // decodes to `None` args, which `run_wm` then REFUSES for a
                // granted-tool Mote (fail-closed — never fire empty/garbled).
                let tool_args: Option<(Vec<u8>, kx_warrant::NetScope, kx_warrant::FsScope)> =
                    item.tool_args.and_then(|ta| {
                        let net_scope = ta.net_scope?.try_into().ok()?;
                        // PR-6a/D155 (fs-list): an ABSENT fs_scope decodes to empty
                        // (an old coordinator / a non-fs tool ⇒ byte-identical).
                        let fs_scope = ta
                            .fs_scope
                            .map(TryInto::try_into)
                            .transpose()
                            .ok()
                            .flatten()
                            .unwrap_or_default();
                        Some((ta.args_bytes, net_scope, fs_scope))
                    });
                run_wm::run_wm(
                    &mut self.client,
                    &*self.broker,
                    &mote,
                    &warrant,
                    self.id,
                    instance_id,
                    tool_args,
                )
                .await
            };
            let result_ref = match exec {
                Ok(result_ref) => {
                    self.attempts.remove(&mote.id); // a clean run clears the retry counter
                    result_ref
                }
                Err(error) => {
                    self.handle_execution_failure(mote.id, &error).await;
                    continue;
                }
            };
            let request =
                commit_builder::report_commit_request(&mote, &warrant, result_ref, self.id);
            let response = self.client.report_commit(request).await?;

            match proto::CommitOutcome::try_from(response.outcome) {
                Ok(proto::CommitOutcome::Committed | proto::CommitOutcome::AlreadyCommitted) => {
                    tracing::info!(
                        worker_id = self.id,
                        seq = response.committed_seq,
                        mote = ?mote.id,
                        "commit proposal accepted"
                    );
                    self.attempts.remove(&mote.id);
                    committed += 1;
                }
                _ => return Err(WorkerError::CommitRejected(response.detail)),
            }
        }
        self.in_flight.store(0, Ordering::Relaxed);
        if in_flight > 0 {
            let _ = self.client.heartbeat(self.id, now_ms(), 0).await;
        }
        Ok(committed)
    }

    /// Hand a leased Mote's out-of-band executor context to the [`ContextSink`] BEFORE
    /// dispatch (the frozen `MoteExecutor::run` carries no snapshot): its F-7 Data
    /// trajectory (`parent_results`), the PR-9d grounding-context ref, and the
    /// AGENTIC-VISION grounding-image ref. ALWAYS set — including empties — so a prior
    /// Mote's context can never leak into this one; malformed/empty refs decode to `None`
    /// (byte-identical to the pre-feature path). A no-op when the executor holds no sink.
    fn deliver_executor_context(
        &self,
        mote_id: MoteId,
        parent_results: &[proto::ParentResult],
        context_items: &[u8],
        image_ref: &[u8],
    ) {
        let Some(sink) = &self.context_sink else {
            return;
        };
        let parents: Vec<(MoteId, ContentRef)> = parent_results
            .iter()
            .filter_map(|pr| {
                let id: [u8; 32] = pr.parent_mote_id.as_slice().try_into().ok()?;
                let r: [u8; 32] = pr.result_ref.as_slice().try_into().ok()?;
                Some((MoteId::from_bytes(id), ContentRef::from_bytes(r)))
            })
            .collect();
        sink.set_parent_results(mote_id, parents);
        // PR-9d: the run's grounding-context ref for a SUCCESSOR ReAct turn (the executor
        // prepends it ahead of the F-7 trajectory on the next `run`).
        let context_items_ref = <[u8; 32]>::try_from(context_items)
            .ok()
            .map(ContentRef::from_bytes);
        sink.set_context_items(mote_id, context_items_ref);
        // AGENTIC-VISION: the run's grounding-image ref for a SUCCESSOR ReAct turn (the
        // executor feeds it into the per-turn multimodal call on the next `run`).
        let image = <[u8; 32]>::try_from(image_ref)
            .ok()
            .map(ContentRef::from_bytes);
        sink.set_image_ref(mote_id, image);
    }

    /// Handle a per-Mote execution failure (F4): classify it, and either dead-letter the
    /// Mote now (a terminal-logic failure, or a transient one that has exhausted
    /// [`WORKER_MAX_ATTEMPTS`]) or leave it `Pending` for a bounded retry on the next
    /// lease. Never propagates — the caller `continue`s to the next leased item, so one
    /// Mote's failure can neither abort the batch nor spin the worker.
    async fn handle_execution_failure(&mut self, mote_id: MoteId, error: &WorkerError) {
        match classify_worker_failure(error) {
            FailureClass::TerminalLogic => {
                tracing::warn!(
                    worker_id = self.id,
                    mote = ?mote_id,
                    %error,
                    "terminal execution failure; dead-lettering"
                );
                self.dead_letter(mote_id).await;
            }
            FailureClass::TransientInfra => {
                let attempts = self.attempts.entry(mote_id).or_insert(0);
                *attempts += 1;
                if *attempts >= WORKER_MAX_ATTEMPTS {
                    tracing::warn!(
                        worker_id = self.id,
                        mote = ?mote_id,
                        attempts = *attempts,
                        %error,
                        "transient failure budget exhausted; dead-lettering"
                    );
                    self.dead_letter(mote_id).await;
                } else {
                    tracing::warn!(
                        worker_id = self.id,
                        mote = ?mote_id,
                        attempts = *attempts,
                        %error,
                        "transient execution failure; will retry on re-lease"
                    );
                }
            }
        }
    }

    /// Dead-letter `mote_id`: report a terminal failure to the coordinator (F4) and drop
    /// the local retry counter. **Best-effort**: a failed report is logged, not
    /// propagated — the Mote simply stays leasable and a later report (or the
    /// coordinator's death reaper) resolves it; the coordinator's `ReportFailure` is
    /// idempotent, so a duplicate after a retry is a no-op.
    async fn dead_letter(&mut self, mote_id: MoteId) {
        self.attempts.remove(&mote_id);
        if let Err(error) = self
            .client
            .report_failure(*mote_id.as_bytes(), self.id)
            .await
        {
            tracing::warn!(
                worker_id = self.id,
                mote = ?mote_id,
                %error,
                "report_failure failed; mote will be retried or reaped"
            );
        }
    }

    /// Send a liveness heartbeat with the current wall-clock and `in_flight` count.
    /// Returns the coordinator's ack.
    pub async fn heartbeat(&mut self, in_flight: u32) -> Result<bool, WorkerError> {
        self.client.heartbeat(self.id, now_ms(), in_flight).await
    }

    /// Spawn a background task that heartbeats every `cadence`, keeping this worker
    /// **live** in the coordinator's registry even while idle — an idle worker leases
    /// no work, so without this it would send nothing and be falsely declared dead
    /// (worker-death detection, P3.1). The reported `in_flight` tracks the worker's
    /// live batch via the shared counter [`run_once`](Self::run_once) maintains, so
    /// load-aware placement (D56) stays accurate.
    ///
    /// Returns the task [`JoinHandle`]; the caller owns its lifetime (drop/abort to
    /// stop, e.g. on shutdown). Best-effort: a failed heartbeat is logged and retried
    /// on the next tick — a transient hiccup never tears the worker down. Missed ticks
    /// are delayed (not bursted) so a stall cannot produce a thundering catch-up.
    ///
    /// INVARIANT: `cadence` must be well under the coordinator's liveness timeout —
    /// `timeout >= 3 * cadence` (see [`DEFAULT_HEARTBEAT_CADENCE`]) — so a couple of
    /// dropped heartbeats do not trip a false death (P0.9: never declare a
    /// slow-but-alive worker dead).
    #[must_use]
    pub fn spawn_heartbeat(&self, cadence: Duration) -> JoinHandle<()> {
        let mut client = self.client.clone();
        let id = self.id;
        let in_flight = Arc::clone(&self.in_flight);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(cadence);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                let load = in_flight.load(Ordering::Relaxed);
                if let Err(error) = client.heartbeat(id, now_ms(), load).await {
                    tracing::debug!(
                        worker_id = id,
                        %error,
                        "background heartbeat failed; retrying next tick"
                    );
                }
            }
        })
    }
}

/// PR-2d-2: `true` iff `mote` is a coordinator-materialized ReAct TURN — the
/// identity-bearing [`REACT_TURN_KEY`] marker (D53: in `config_subset`, so it
/// folds into the `MoteId` and can never be dropped in transit) with NO
/// `tool_contract` (a turn PROPOSES; the marker-less observation Mote, which
/// DOES declare a contract, fires through the broker arm as usual). A
/// client-crafted marker reaches a strictly STRICTER path: the executor's react
/// arm decodes + fences the output pre-commit and fires nothing.
fn is_react_turn(mote: &Mote) -> bool {
    mote.def.tool_contract.is_empty()
        && mote
            .def
            .config_subset
            .contains_key(&ConfigKey(REACT_TURN_KEY.to_string()))
}

/// RC4c-2b: `true` iff `mote` is a coordinator-materialized live LLM RERANK TURN — the
/// identity-bearing [`RERANK_TURN_KEY`] marker with NO `tool_contract`. Like a ReAct
/// turn it is a ROND model Mote that dispatches DIRECTLY through the hosted executor
/// (whose `run_rerank_turn` arm renders the rerank prompt + commits the raw
/// permutation), never the capability-broker WM path (a rerank PROPOSES an order; it
/// fires no effect). Without this the worker would route it to `run_wm` and dead-letter
/// it (no tool_contract to resolve).
fn is_rerank_turn(mote: &Mote) -> bool {
    mote.def.tool_contract.is_empty()
        && mote
            .def
            .config_subset
            .contains_key(&ConfigKey(RERANK_TURN_KEY.to_string()))
}

/// `true` iff `mote` dispatches DIRECTLY through the hosted executor rather than the
/// capability-broker WM path. Three coordinator-materialized ROND "model" Mote classes
/// route here — a ReAct TURN, a live LLM RERANK TURN, and the opt-in LLM-JUDGE critic —
/// each PROPOSES output the executor's `run()` commits, firing NO effect (so `run_wm`'s
/// tool-contract resolution would dead-letter them).
///
/// **Named + unit-tested deliberately (RC4c-2c).** A new such class silently missing from
/// this predicate is the `T-RERANK-WORKER-ROUTE` class: RC4c-2b shipped with the rerank
/// turn routed to `run_wm` → dead-lettered UNRUN → the rerank silently fell back to base
/// order, and it passed every unit test + 22 CI jobs precisely because it fails closed. The
/// `dispatches_through_executor_covers_rerank_and_react` test bites if the rerank (or
/// react) term is ever dropped again.
fn dispatches_through_executor(mote: &Mote) -> bool {
    is_react_turn(mote) || is_rerank_turn(mote) || mote.def.critic_check.is_some()
}

/// Wall-clock milliseconds since the Unix epoch (liveness only; never hashed).
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| u64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kx_mote::{
        ConfigVal, EffectPattern, GraphPosition, InferenceParams, InputDataId, LogicRef, ModelId,
        MoteDef, PromptTemplateHash, ToolName, ToolVersion, MOTE_DEF_SCHEMA_VERSION,
    };
    use smallvec::SmallVec;
    use std::collections::BTreeMap;

    /// Build a minimal Mote for the routing predicate: an optional `config_subset` marker
    /// (react/rerank), and whether it declares a `tool_contract` (the WM/broker shape).
    fn routing_mote(marker: Option<&str>, wm: bool) -> Mote {
        let mut config_subset = BTreeMap::new();
        if let Some(key) = marker {
            config_subset.insert(ConfigKey(key.to_string()), ConfigVal(vec![1]));
        }
        let mut tool_contract = BTreeMap::new();
        if wm {
            tool_contract.insert(ToolName("kx-test/effect".into()), ToolVersion("1".into()));
        }
        let def = MoteDef {
            critic_check: None,
            logic_ref: LogicRef::from_bytes([7u8; 32]),
            model_id: ModelId("m".into()),
            prompt_template_hash: PromptTemplateHash::from_bytes([9u8; 32]),
            tool_contract,
            nd_class: if wm {
                NdClass::WorldMutating
            } else {
                NdClass::ReadOnlyNondet
            },
            config_subset,
            effect_pattern: EffectPattern::IdempotentByConstruction,
            critic_for: None,
            is_topology_shaper: false,
            inference_params: InferenceParams::default(),
            schema_version: MOTE_DEF_SCHEMA_VERSION,
        };
        Mote::new(
            def,
            InputDataId::from_bytes([1u8; 32]),
            GraphPosition(vec![1]),
            SmallVec::new(),
        )
    }

    /// RC4c-2c regression guard for the `T-RERANK-WORKER-ROUTE` class: a coordinator-
    /// materialized live LLM RERANK TURN (identity-bearing marker, NO `tool_contract`) MUST
    /// route to the direct-executor arm, NOT the broker WM path (which would dead-letter it
    /// unrun → the rerank silently falls back to base order — a fail-closed miss that no
    /// unit test + 22 CI jobs caught). A ReAct TURN routes the same way; a plain WM tool
    /// Mote does NOT.
    #[test]
    fn dispatches_through_executor_covers_rerank_and_react() {
        assert!(
            dispatches_through_executor(&routing_mote(Some(RERANK_TURN_KEY), false)),
            "a rerank turn MUST dispatch through the executor (T-RERANK-WORKER-ROUTE)"
        );
        assert!(
            dispatches_through_executor(&routing_mote(Some(REACT_TURN_KEY), false)),
            "a react turn dispatches through the executor"
        );
        assert!(
            !dispatches_through_executor(&routing_mote(None, true)),
            "a plain WM tool Mote routes to the broker, not the executor"
        );
        // A marker with a NON-empty tool_contract is NOT a rerank/react turn (the markers
        // require an empty contract) — it stays on the WM path.
        assert!(
            !dispatches_through_executor(&routing_mote(Some(RERANK_TURN_KEY), true)),
            "a marker + tool_contract is not a promptless turn"
        );
    }
}
