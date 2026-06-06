//! PR-2 (F-4) — **the model drives the loop.** A real model computes a topology
//! shaper's [`kx_mote::TopologyDecision`] (the next-step fan-out), replacing the
//! hardcoded `demo_topology_decision()`. This is the D111 home of the
//! "model-runs-once → decode → lower → read-the-plan-back" wiring: `kx-planner`
//! stays a pure lowering library; the runtime stays free of any model type; the
//! orchestration seam ([`kx_runtime::TopologyProvider`]) is implemented HERE.
//!
//! ## Flow ([`run_model_loop`])
//!
//! 1. [`ModelTopologyProvider::decide`] assembles the shaper's context (D78 —
//!    reusing the crate's `context::model_input` / `assemble` wiring), runs the model ONCE,
//!    decodes the completion **fail-closed** via [`kx_planner::decode_loop_proposal`],
//!    enforces the per-decision child budget, and lowers it through *vetted
//!    recipes* ([`kx_planner::lower_loop_to_topology_decision`]) into a
//!    `TopologyDecision` (SN-8: the model proposes role *names*; identity axes +
//!    warrant narrowing are the runtime's).
//! 2. The decision's canonical bytes are staged as the shaper's effect (a
//!    [`kx_runtime::broker::DemoBroker`] response), so the shaper's committed
//!    `result_ref` IS the decision hash — a captured fact the
//!    `DefaultTopologyMaterializer` + the engine both decode (one source of truth).
//! 3. `run_with_seams` drives the run: the shaper commits the decision, its
//!    children materialize + execute, and a cold re-fold re-derives byte-identical
//!    children (R49 — the model's choice is replayed, never re-sampled).
//!
//! ## Fail-closed (composes with PR-1)
//!
//! A malformed / oversized / un-grantable / over-budget proposal makes `decide`
//! return [`TopologyProviderError`]; [`run_model_loop`] then **dead-letters** the
//! shaper with a terminal `Failed` fact (never a panic) and the run completes
//! PAST it with no children — exactly the PR-1 "a failing Mote is recorded, never
//! blindly re-run" discipline.
//!
//! ## Bounded (the OSS "bounded additive" guarantee)
//!
//! Three layered bounds: the byte cap ([`kx_planner::max_plan_bytes`], enforced
//! before parse), the structural per-round cap (`MAX_LOOP_STEPS` in decode), and
//! the run-policy [`LoopBudget`] (`max_rounds` across rounds + `max_children` per
//! decision). The unbounded durable-loop topology stays cloud (D126 cat-B).

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use kx_content::ContentStore;
use kx_executor::{LocalResourceManager, StandardCommitProtocol};
use kx_inference::{inference_params_from_mote, InferenceBackend};
use kx_journal::{FailureReason, Journal, JournalEntry, SqliteJournal};
use kx_mote::{Mote, MoteDef, MoteId, RoleId, TopologyDecision};
use kx_planner::{
    decode_loop_proposal, lower_loop_to_topology_decision, max_plan_bytes, RoleRecipeResolver,
};
use kx_projection::{
    DefaultTopologyMaterializer, InMemoryMoteDefRegistry, InheritFromShaperResolver, Projection,
    Snapshot, TopologyMaterializer,
};
use kx_runtime::broker::DemoBroker;
use kx_runtime::topology::encode_topology_decision;
use kx_runtime::{
    run_with_seams, DemoWorkflow, FailurePolicy, RunOutcome, RuntimeConfig, RuntimeError,
    SnapshotSink, TopologyProvider, TopologyProviderError,
};
use kx_tool_registry::ToolRegistry;
use kx_warrant::{Role, RoleRegistry, WarrantSpec};

use crate::{context, prompt, ModelExecutor};

/// A [`RoleRegistry`] that grants EVERY proposed role the shaper's own warrant, so
/// the materializer's `intersect(shaper.warrant, role.spec)` narrowing is a no-op:
/// children inherit the shaper's authority — never wider (SN-8 narrowing-only).
/// The role ALLOWLIST is the [`RoleRecipeResolver`] (an unregistered role fails
/// closed at `lower_loop_to_topology_decision` before the materializer is ever
/// consulted); per-role *restriction* (tighter child warrants) arrives with the
/// PR-4 tool-call warrants. This keeps PR-2's foundation correct + bounded without
/// a workflow author pre-registering a parallel role catalog.
struct InheritShaperWarrantRoles {
    shaper_warrant: WarrantSpec,
}

impl RoleRegistry for InheritShaperWarrantRoles {
    fn resolve(&self, role_id: &RoleId) -> Option<Role> {
        Some(Role {
            name: role_id.0.clone(),
            version: 1,
            spec: self.shaper_warrant.clone(),
            description: String::new(),
        })
    }
}

/// Reporter id stamped on a dead-lettered shaper's terminal `Failed` entry — a
/// fixed, UUID-shaped provenance marker (never identity or dedup input, D19),
/// distinct from the engine's `RUNTIME_REPORTER_ID` and the coordinator's `0`.
const TOPOLOGY_PROVIDER_REPORTER_ID: u128 = 0x6b78_5f74_6f70_6f6c_6f67_7900_0000_0001;

/// The per-run bound on a model-driven loop — the OSS "bounded additive" cap (the
/// unbounded durable-loop / recursion topology stays cloud, D126 cat-B).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoopBudget {
    /// Max replan rounds per run the provider will honour (shaper invocations).
    /// The `(max_rounds + 1)`-th [`ModelTopologyProvider::decide`] returns
    /// [`TopologyProviderError`] ⇒ the shaper dead-letters ⇒ the run completes
    /// bounded. (One round in PR-2's single-shaper foundation; exercised across
    /// rounds by PR-3 re-plan.)
    pub max_rounds: u32,
    /// Max children a single decision may spawn — the run-policy fan-out cap,
    /// enforced AFTER decode's structural `MAX_LOOP_STEPS` cap (typically tighter).
    pub max_children: usize,
}

impl Default for LoopBudget {
    fn default() -> Self {
        Self {
            max_rounds: 4,
            max_children: 8,
        }
    }
}

/// A [`TopologyProvider`] that computes a shaper's decision from a real model.
///
/// Generic over the backend + store so the live campaign uses
/// [`crate::MeteredBackend`]`<`[`kx_inference::LlamaInferenceBackend`]`>` while a
/// deterministic test uses a stub backend (no GGUF needed). Shares the backend
/// `Arc` with the run's [`ModelExecutor`] so the dispatch count aggregates.
pub struct ModelTopologyProvider<B: InferenceBackend, S: ContentStore> {
    backend: Arc<B>,
    store: Arc<S>,
    registry: Arc<dyn ToolRegistry>,
    recipes: Arc<dyn RoleRecipeResolver>,
    budget: LoopBudget,
    /// Rounds consumed so far (off the truth path — a crash re-folds the committed
    /// prefix and the counter restarts harmlessly; it only ever *limits* fresh
    /// rounds, never affects a committed shaper's already-materialized children).
    round: AtomicU32,
}

impl<B: InferenceBackend, S: ContentStore> std::fmt::Debug for ModelTopologyProvider<B, S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModelTopologyProvider")
            .field("budget", &self.budget)
            .field("round", &self.round.load(Ordering::SeqCst))
            .finish_non_exhaustive()
    }
}

impl<B: InferenceBackend, S: ContentStore> ModelTopologyProvider<B, S> {
    /// Build a provider over a shared backend + content store + tool registry +
    /// vetted role-recipe resolver, bounded by `budget`.
    #[must_use]
    pub fn new(
        backend: Arc<B>,
        store: Arc<S>,
        registry: Arc<dyn ToolRegistry>,
        recipes: Arc<dyn RoleRecipeResolver>,
        budget: LoopBudget,
    ) -> Self {
        Self {
            backend,
            store,
            registry,
            recipes,
            budget,
            round: AtomicU32::new(0),
        }
    }
}

impl<B, S> TopologyProvider for ModelTopologyProvider<B, S>
where
    B: InferenceBackend,
    S: ContentStore + Send + Sync + 'static,
{
    fn decide(
        &self,
        shaper: &Mote,
        shaper_warrant: &WarrantSpec,
        snapshot: &Snapshot,
    ) -> Result<TopologyDecision, TopologyProviderError> {
        // (0) Round budget — fail-closed once exhausted (the run dead-letters the
        //     shaper; bounded additive). `fetch_add` returns the pre-increment
        //     count, so `max_rounds` rounds (0..max_rounds) are honoured.
        let round = self.round.fetch_add(1, Ordering::SeqCst);
        if round >= self.budget.max_rounds {
            return Err(TopologyProviderError(format!(
                "loop budget exhausted: {} of max {} replan rounds used",
                round, self.budget.max_rounds
            )));
        }

        // (1) Assemble the shaper's context (D78) + ChatML-wrap the planning
        //     instruction. Reuses the executor's exact wiring (assemble against
        //     the snapshot; a parentless shaper yields an empty context, so the
        //     input is `chatml(instruction)` — byte-identical to the leaf path).
        let instruction = prompt::raw_prompt(shaper).ok_or_else(|| {
            TopologyProviderError("topology shaper carries no planning prompt".to_string())
        })?;
        let sink = SnapshotSink::new();
        sink.publish(snapshot.clone());
        let input = context::model_input(
            shaper,
            shaper_warrant,
            &instruction,
            &sink,
            &*self.store,
            &*self.registry,
        )
        .map_err(|e| TopologyProviderError(format!("context assembly: {e}")))?;

        // (2) Run the model ONCE (params verbatim from the identity-bearing
        //     `mote.def.inference_params`, the sole permitted constructor — D50).
        let params = inference_params_from_mote(shaper, shaper_warrant)
            .map_err(|e| TopologyProviderError(format!("inference params: {e}")))?;
        let out = self
            .backend
            .dispatch(&shaper.def.model_id, &input, &params, shaper_warrant)
            .map_err(|e| TopologyProviderError(format!("model dispatch: {e}")))?;

        // (3) Decode the proposal FAIL-CLOSED (the untrusted-bytes trust boundary:
        //     size-cap-before-parse, `deny_unknown_fields`, `<think>`-strip,
        //     versioned). The byte cap is the warrant's output ceiling.
        let proposal = decode_loop_proposal(&out.bytes, max_plan_bytes(shaper_warrant))
            .map_err(|e| TopologyProviderError(format!("decode loop proposal: {e}")))?;

        // (4) Run-policy fan-out cap (after decode's structural cap).
        if proposal.next_steps.len() > self.budget.max_children {
            return Err(TopologyProviderError(format!(
                "decision proposes {} children, exceeding max_children {}",
                proposal.next_steps.len(),
                self.budget.max_children
            )));
        }

        // (5) Lower through VETTED recipes — role identity axes (logic_ref /
        //     nd_class / effect_pattern) come from the recipe, never model output
        //     (SN-8 / IMP-5 / D70). An unregistered role fails closed.
        lower_loop_to_topology_decision(&proposal, &*self.recipes)
            .map_err(|e| TopologyProviderError(format!("lower: {e}")))
    }

    fn materializer(
        &self,
        shaper_def: &MoteDef,
        shaper_warrant: &WarrantSpec,
    ) -> Box<dyn TopologyMaterializer> {
        // Resolve the model's proposed roles (the recipe allowlist already gated
        // them at `lower`) and narrow each child trivially against the shaper's
        // own warrant. Reuses the SHARED store (the run's), so the committed
        // decision + warrant bytes the fold reads are the ones the run wrote.
        let def_registry = InMemoryMoteDefRegistry::new();
        def_registry.register(shaper_def.clone());
        Box::new(DefaultTopologyMaterializer::new(
            self.store.clone(),
            Arc::new(def_registry),
            Arc::new(InheritShaperWarrantRoles {
                shaper_warrant: shaper_warrant.clone(),
            }),
            InheritFromShaperResolver,
        ))
    }
}

/// Drive a model-driven topology loop end-to-end through the real orchestrator.
///
/// Shared by the live campaign ([`crate::Harness::drive_model_loop`]) and the
/// deterministic CI test (a stub backend), so there is ONE eager-decide → stage →
/// drive path. `workflow` must be a single topology-shaper [`DemoWorkflow`] (see
/// [`crate::workflows::loop_shaper`]); `shaper_id` selects the shaper.
///
/// PR-2 computes the (parentless) shaper's decision **eagerly** — once, before the
/// run, against an empty initial snapshot — then stages it as the shaper's effect.
/// PR-3 re-plan will compute lazily per round through the same provider seam.
///
/// # Errors
/// Propagates store / journal / orchestrator failures. A *refused model proposal*
/// is NOT an error: the shaper is dead-lettered (terminal `Failed`) and the run
/// completes with no children — the returned [`RunOutcome`] reflects that.
// Each argument is a distinct injected seam (config / store / journal / backend /
// registry / recipes / workflow / budget) — mirrors `run_with_seams`'s own allow.
#[allow(clippy::too_many_arguments)]
pub fn run_model_loop<B, S>(
    config: &RuntimeConfig,
    store: Arc<S>,
    journal: Arc<SqliteJournal>,
    backend: Arc<B>,
    registry: Arc<dyn ToolRegistry>,
    recipes: Arc<dyn RoleRecipeResolver>,
    workflow: &DemoWorkflow,
    budget: LoopBudget,
) -> Result<RunOutcome, RuntimeError>
where
    B: InferenceBackend,
    S: ContentStore + Send + Sync + 'static,
{
    let rm = LocalResourceManager::dev_defaults();
    // Bounded additive (PR-1): a refused proposal / failing child is recorded as a
    // terminal `Failed` and the run continues past it — never an abort, never a
    // blind re-run. `max_attempts = 1` ⇒ a (deterministic) refusal dead-letters at
    // once rather than pointlessly re-running the model.
    let failure_policy = FailurePolicy::new(1, Duration::ZERO);

    let shaper_wm = workflow
        .motes
        .iter()
        .find(|w| w.mote.id == workflow.shaper_id)
        .cloned()
        .ok_or_else(|| {
            RuntimeError::Config("model-loop workflow is missing its topology shaper".to_string())
        })?;

    let provider = ModelTopologyProvider::new(
        backend.clone(),
        store.clone(),
        registry.clone(),
        recipes,
        budget,
    );

    // EAGER decide: the parentless shaper's decision is computed once, before the
    // run, against an empty initial snapshot (no parents committed yet ⇒ empty
    // assembled context). PR-3 re-plan computes lazily per round.
    let initial_snapshot = Projection::new().snapshot();
    let decision = match provider.decide(&shaper_wm.mote, &shaper_wm.warrant, &initial_snapshot) {
        Ok(decision) => decision,
        Err(e) => {
            // FAIL-CLOSED: record a terminal `Failed` for the shaper (dead-letter)
            // and run — the engine skips the now-terminal shaper, no children
            // materialize, and the run completes (PR-1 discipline). `register_mote`
            // only sets declared info, so this folded `Failed` state survives the
            // scheduler's submit (verified) and `pick_next` skips it.
            tracing::warn!(
                error = %e,
                shaper = ?workflow.shaper_id,
                "topology provider refused the model proposal — dead-lettering the shaper"
            );
            journal.append(JournalEntry::Failed {
                mote_id: workflow.shaper_id,
                idempotency_key: *workflow.shaper_id.as_bytes(),
                seq: 0, // journal assigns
                reason_class: FailureReason::ExecutorRefused,
                reporter_id: TOPOLOGY_PROVIDER_REPORTER_ID,
            })?;
            return drive_dead_lettered(
                config,
                &store,
                journal,
                backend,
                registry,
                workflow,
                &rm,
                &failure_policy,
                &shaper_wm,
                &provider,
            );
        }
    };

    // The model's lowered decision becomes the shaper's committed effect: stage its
    // canonical bytes so the committed `result_ref` is the decision hash — the
    // exact bytes the materializer + the engine's fact-driven derivation decode.
    let mut responses: BTreeMap<MoteId, Vec<u8>> = BTreeMap::new();
    responses.insert(workflow.shaper_id, encode_topology_decision(&decision)?);

    let sink = SnapshotSink::new();
    let executor = ModelExecutor::new(backend, store.clone(), sink.clone(), registry);
    let broker = Arc::new(DemoBroker::new(
        store.clone(),
        responses,
        config.crash_at,
        Some(workflow.stc_crash_target),
    ));
    let protocol = StandardCommitProtocol::new(store.clone(), journal.clone(), broker);

    run_with_seams(
        config,
        workflow,
        store,
        journal,
        &rm,
        &executor,
        &protocol,
        // The shaper arg supplies the workflow Mote + the (decided) topology; with
        // `topology_provider = Some` below, the engine derives children from the
        // COMMITTED fact, so this decision is the staged-and-committed one anyway.
        Some((&shaper_wm, &decision)),
        Some(&provider),
        Some(&sink),
        None, // capture_sink — off for the loop foundation
        None, // audit_sink (R4) — off for the loop foundation
        Some(&failure_policy),
    )
}

/// Drive a run whose shaper has already been dead-lettered (a refused proposal):
/// no decision is staged, the engine skips the terminal shaper, and the run
/// completes with no children. Split out so the borrow of the eager `provider`
/// outlives `run_with_seams` (the `Some(&provider)` arg).
#[allow(clippy::too_many_arguments)]
fn drive_dead_lettered<B, S>(
    config: &RuntimeConfig,
    store: &Arc<S>,
    journal: Arc<SqliteJournal>,
    backend: Arc<B>,
    registry: Arc<dyn ToolRegistry>,
    workflow: &DemoWorkflow,
    rm: &LocalResourceManager,
    failure_policy: &FailurePolicy,
    shaper_wm: &kx_runtime::workflow::WorkflowMote,
    provider: &ModelTopologyProvider<B, S>,
) -> Result<RunOutcome, RuntimeError>
where
    B: InferenceBackend,
    S: ContentStore + Send + Sync + 'static,
{
    let sink = SnapshotSink::new();
    let executor = ModelExecutor::new(backend, store.clone(), sink.clone(), registry);
    let broker = Arc::new(DemoBroker::new(
        store.clone(),
        BTreeMap::new(),
        config.crash_at,
        Some(workflow.stc_crash_target),
    ));
    let protocol = StandardCommitProtocol::new(store.clone(), journal.clone(), broker);
    // A throwaway decision satisfies the `shaper` arg's type; it is never used —
    // `topology_provider = Some` selects fact-driven derivation, and the shaper is
    // already terminal so no children derive.
    let unused = kx_mote::TopologyDecision {
        children: Vec::new(),
    };
    run_with_seams(
        config,
        workflow,
        store.clone(),
        journal,
        rm,
        &executor,
        &protocol,
        Some((shaper_wm, &unused)),
        Some(provider),
        Some(&sink),
        None,
        None,
        Some(failure_policy),
    )
}
