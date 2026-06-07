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
use std::fmt::Write as _;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use kx_content::ContentStore;
use kx_executor::{LocalResourceManager, StandardCommitProtocol};
use kx_inference::{inference_params_from_mote, InferenceBackend};
use kx_journal::{FailureReason, Journal, JournalEntry, SqliteJournal};
use kx_mote::{Mote, MoteDef, MoteId, RoleId, TopologyDecision};
use kx_planner::{
    decode_loop_proposal, decode_replan_proposal, lower_loop_to_topology_decision, max_plan_bytes,
    ReplanProposal, RoleRecipeResolver,
};
use kx_projection::{
    DefaultTopologyMaterializer, InMemoryMoteDefRegistry, InheritFromShaperResolver, MoteState,
    Projection, Snapshot, TopologyMaterializer,
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
    /// **PR-3 (AL2).** A SHARED, accumulating shaper-def registry. PR-2 has one
    /// shaper per run; a re-plan loop ([`run_replan_loop`]) chains MANY (one per
    /// round). Each round's [`Self::materializer`] registers its shaper def here
    /// (idempotent), so a SINGLE materializer folding the journal re-derives EVERY
    /// committed round's children (the def is looked up by `def_hash`). With one
    /// shaper (PR-2) the registry holds exactly that def — byte-identical behavior.
    def_registry: Arc<InMemoryMoteDefRegistry>,
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
            def_registry: Arc::new(InMemoryMoteDefRegistry::new()),
        }
    }

    /// **PR-3 (AL2).** Assemble this shaper's context (D78) and run the model ONCE,
    /// returning the raw completion bytes. Shared by [`TopologyProvider::decide`]
    /// (the initial round) and [`Self::replan`] (a re-plan round) so there is ONE
    /// model-runs-once boundary; the two differ only in how they DECODE the bytes.
    /// Enforces the cross-round budget first (fail-closed once exhausted).
    fn run_model_once(
        &self,
        shaper: &Mote,
        shaper_warrant: &WarrantSpec,
        snapshot: &Snapshot,
    ) -> Result<Vec<u8>, TopologyProviderError> {
        // (0) Round budget — fail-closed once exhausted. `fetch_add` returns the
        //     pre-increment count, so rounds `0..max_rounds` are honoured.
        let round = self.round.fetch_add(1, Ordering::SeqCst);
        if round >= self.budget.max_rounds {
            return Err(TopologyProviderError(format!(
                "loop budget exhausted: {} of max {} replan rounds used",
                round, self.budget.max_rounds
            )));
        }

        // (1) Assemble the shaper's (corrected) instruction + upstream context (D78),
        //     reusing the executor's exact wiring.
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
        Ok(out.bytes)
    }

    /// **PR-3 (AL2).** A model-driven RE-PLAN round: run the model once against the
    /// live snapshot (which now carries the prior round's failure — the shaper's
    /// instruction is the failure-corrected prompt the driver built), decode the
    /// 3-way `{"replan": …}` envelope FAIL-CLOSED ([`decode_replan_proposal`]), and
    /// either lower a corrective fan-out through vetted recipes
    /// ([`ReplanOutcome::Topology`]) or surface an escalation
    /// ([`ReplanOutcome::FlagHuman`]). SN-8: a permission-adapt proposes a role; the
    /// runtime narrows authority downstream (`intersect`), never widens.
    ///
    /// # Errors
    /// [`TopologyProviderError`] for an exhausted budget / malformed-or-over-budget
    /// proposal / unknown role — the caller dead-letters the shaper (PR-1 discipline).
    pub fn replan(
        &self,
        shaper: &Mote,
        shaper_warrant: &WarrantSpec,
        snapshot: &Snapshot,
    ) -> Result<ReplanOutcome, TopologyProviderError> {
        let bytes = self.run_model_once(shaper, shaper_warrant, snapshot)?;
        match decode_replan_proposal(&bytes, max_plan_bytes(shaper_warrant))
            .map_err(|e| TopologyProviderError(format!("decode replan proposal: {e}")))?
        {
            ReplanProposal::Topology(proposal) => {
                if proposal.next_steps.len() > self.budget.max_children {
                    return Err(TopologyProviderError(format!(
                        "replan proposes {} children, exceeding max_children {}",
                        proposal.next_steps.len(),
                        self.budget.max_children
                    )));
                }
                let td = lower_loop_to_topology_decision(&proposal, &*self.recipes)
                    .map_err(|e| TopologyProviderError(format!("lower: {e}")))?;
                Ok(ReplanOutcome::Topology(td))
            }
            ReplanProposal::FlagHuman(reason) => Ok(ReplanOutcome::FlagHuman(reason)),
        }
    }
}

/// **PR-3 (AL2).** The lowered form of a re-plan round's 3-way decision (the
/// runtime-side counterpart of [`kx_planner::ReplanProposal`]). `Topology` is a
/// vetted-recipe-lowered corrective fan-out (corrected-context / permission-adapt,
/// both narrowing-only); `FlagHuman` is a clean terminal escalation (the failed
/// step stays a durable dead-lettered fact; the active HITL handshake is a later PR).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplanOutcome {
    /// Spawn this corrective topology next (the re-plan continues).
    Topology(TopologyDecision),
    /// Escalate to a human — stop the loop, leave the failure dead-lettered.
    FlagHuman(String),
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
        // (0-2) Budget + assemble + run the model ONCE (shared with `replan`).
        let bytes = self.run_model_once(shaper, shaper_warrant, snapshot)?;

        // (3) Decode the proposal FAIL-CLOSED (the untrusted-bytes trust boundary:
        //     size-cap-before-parse, `deny_unknown_fields`, `<think>`-strip,
        //     versioned). The byte cap is the warrant's output ceiling.
        let proposal = decode_loop_proposal(&bytes, max_plan_bytes(shaper_warrant))
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
        // PR-3: register THIS round's shaper def into the SHARED accumulating
        // registry (idempotent), then build a materializer over it — so a single
        // fold of the journal re-derives EVERY committed round's children (lookup
        // by `def_hash`). With one shaper (PR-2) the registry holds exactly this
        // def — byte-identical to the prior fresh-registry behavior. Reuses the
        // SHARED store (the run's), so the committed decision + warrant bytes the
        // fold reads are the ones the run wrote. All rounds share the caller's
        // warrant, so a single `InheritShaperWarrantRoles` narrows every shaper's
        // children trivially (`intersect(shaper.warrant, role.spec=shaper.warrant)`).
        self.def_registry.register(shaper_def.clone());
        Box::new(DefaultTopologyMaterializer::new(
            self.store.clone(),
            self.def_registry.clone(),
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

// ===========================================================================
// PR-3 (AL2) — the bounded re-plan-on-failure loop.
// ===========================================================================

/// The outcome of a bounded re-plan loop ([`run_replan_loop`]).
#[derive(Debug, Clone)]
pub struct ReplanLoopOutcome {
    /// The final round's [`RunOutcome`]. Its `digest` is the WHOLE-journal replay
    /// surface (every committed Mote across every round), so a different process
    /// cold-folding the journal reproduces it (R49).
    pub run: RunOutcome,
    /// How many shaper rounds were driven. `1` ⇒ the initial plan needed no
    /// correction; `n` ⇒ `n-1` re-plan rounds followed the first failure.
    pub rounds_used: u32,
    /// `Some(reason)` iff the model chose flag-a-human: the loop stopped and the
    /// failed step remains a durable dead-lettered fact. The active operator
    /// handshake (resume/inject) is a later PR / the cloud tier; PR-3 only records.
    pub escalation: Option<String>,
}

/// **PR-3 (AL2) — run a bounded model-driven RE-PLAN-ON-FAILURE loop.**
///
/// Drives the initial plan ([`ModelTopologyProvider::decide`], round 0). If a
/// non-shaper step then dead-letters (a terminal `Failed` fact, via PR-1), the
/// driver folds the journal, reads WHY each step failed
/// ([`kx_projection::Snapshot::failure_reason_of`]), builds a failure-corrected
/// instruction, and asks the model for the next round
/// ([`ModelTopologyProvider::replan`]) — the 3-way router: a corrective
/// [`ReplanOutcome::Topology`] (corrected-context / permission-adapt, narrowing-only)
/// spawns the next round's steps; a [`ReplanOutcome::FlagHuman`] stops the loop and
/// records the escalation. The prior round's COMMITTED steps are never touched
/// (D76 append-only); each round is a NEW committed ROND shaper fact, so a crash
/// between rounds resumes by re-folding and a cold re-fold reproduces the exact
/// chain (R49). Bounded by [`LoopBudget`] (`max_rounds` × `max_children`); the
/// unbounded durable loop stays cloud (D126 cat-B).
///
/// `workflow` is the round-0 [`crate::workflows::loop_shaper`]. Composes with PR-1:
/// `max_attempts = 1` ⇒ a refused/failing step dead-letters at once, never a blind
/// re-run.
///
/// # Errors
/// Propagates store / journal / orchestrator failures. A refused proposal, a failing
/// step, budget exhaustion, and an escalation are NOT errors — they are reflected in
/// the returned [`ReplanLoopOutcome`].
// `Arc`-by-value params mirror `run_model_loop`'s public signature (the run owns
// them for its whole lifetime); `run_with_seams` allows the same lint.
#[allow(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    clippy::needless_pass_by_value
)]
pub fn run_replan_loop<B, S>(
    config: &RuntimeConfig,
    store: Arc<S>,
    journal: Arc<SqliteJournal>,
    backend: Arc<B>,
    registry: Arc<dyn ToolRegistry>,
    recipes: Arc<dyn RoleRecipeResolver>,
    workflow: &DemoWorkflow,
    budget: LoopBudget,
) -> Result<ReplanLoopOutcome, RuntimeError>
where
    B: InferenceBackend,
    S: ContentStore + Send + Sync + 'static,
{
    let rm = LocalResourceManager::dev_defaults();
    // PR-1: a refused proposal / failing step is a terminal `Failed` (no retry).
    let failure_policy = FailurePolicy::new(1, Duration::ZERO);
    let provider = ModelTopologyProvider::new(
        backend.clone(),
        store.clone(),
        registry.clone(),
        recipes,
        budget,
    );

    let shaper0 = workflow
        .motes
        .iter()
        .find(|w| w.mote.id == workflow.shaper_id)
        .cloned()
        .ok_or_else(|| {
            RuntimeError::Config("replan-loop workflow is missing its topology shaper".to_string())
        })?;
    let model_id = shaper0.mote.def.model_id.clone();
    let warrant = shaper0.warrant.clone();
    let base_prompt = prompt::raw_prompt(&shaper0.mote).ok_or_else(|| {
        RuntimeError::Config("replan-loop shaper carries no planning prompt".to_string())
    })?;

    let mut escalation: Option<String> = None;

    let mut round: u32 = 0;
    let mut current_wf: DemoWorkflow = workflow.clone();
    let mut current_shaper = shaper0;

    // The loop ALWAYS drives round 0, then re-plans while THIS round's own steps
    // keep failing and the budget allows — every exit `break`s with the round's
    // `RunOutcome` (whose `digest` is the whole-journal replay surface).
    let run: RunOutcome = loop {
        // Fold the journal (skip-committed guard + the snapshot for decide/replan).
        let proj = fold_for_inspection(&provider, &current_shaper, &warrant, &journal)?;
        let shaper_state = proj.state_of(&current_shaper.mote.id);

        // Resolve this round's staged decision. `None` ⇒ serve a committed fact
        // (R49 replay), or stop (a refused/escalated/already-dead-lettered shaper).
        let (staged, stop): (Option<TopologyDecision>, bool) = match shaper_state {
            // R49 replay: the decision is already a committed fact — serve it
            // (the materializer re-derives children), NEVER re-sample the model.
            MoteState::Committed => (None, false),
            // Already dead-lettered (a prior refused round) — drive to a clean stop.
            MoteState::Failed => (None, true),
            // Fresh round: round 0 plans (`decide`); a re-plan round corrects or
            // escalates (`replan`) against the live snapshot (its prompt carries the
            // failure). A refusal / escalation dead-letters the shaper and stops.
            _ => {
                let snapshot = proj.snapshot();
                if round == 0 {
                    match provider.decide(&current_shaper.mote, &warrant, &snapshot) {
                        Ok(td) => (Some(td), false),
                        Err(e) => {
                            tracing::warn!(error = %e, shaper = ?current_shaper.mote.id, "initial plan refused — dead-lettering the shaper");
                            dead_letter_shaper(&journal, current_shaper.mote.id)?;
                            (None, true)
                        }
                    }
                } else {
                    match provider.replan(&current_shaper.mote, &warrant, &snapshot) {
                        Ok(ReplanOutcome::Topology(td)) => (Some(td), false),
                        Ok(ReplanOutcome::FlagHuman(reason)) => {
                            tracing::warn!(reason = %reason, shaper = ?current_shaper.mote.id, "model escalated (flag-a-human) — stopping the loop");
                            dead_letter_shaper(&journal, current_shaper.mote.id)?;
                            escalation = Some(reason);
                            (None, true)
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, shaper = ?current_shaper.mote.id, "re-plan refused — dead-lettering the shaper");
                            dead_letter_shaper(&journal, current_shaper.mote.id)?;
                            (None, true)
                        }
                    }
                }
            }
        };

        let outcome = drive_one_round(
            config,
            &store,
            &journal,
            &backend,
            &registry,
            &rm,
            &failure_policy,
            &current_wf,
            &current_shaper,
            staged.as_ref(),
            &provider,
        )?;

        if stop {
            break outcome; // a refused / escalated / already-dead-lettered shaper stops the loop.
        }

        // Did any of THIS round's OWN steps (its shaper's children) dead-letter?
        // Scoping to this round's children — not all-time `Failed` motes — is what
        // makes the loop CONVERGE: a prior round's failed step stays a durable
        // `Failed` fact forever (D76), but it was already addressed by the round
        // that corrected it; only a fresh failure warrants another re-plan.
        let post = fold_for_inspection(&provider, &current_shaper, &warrant, &journal)?;
        let mut failures: Vec<(MoteId, Option<kx_journal::FailureReason>)> = post
            .children_of(&current_shaper.mote.id)
            .into_iter()
            .map(|(id, _)| id)
            .filter(|id| post.state_of(id) == MoteState::Failed)
            .map(|id| (id, post.failure_reason_of(&id)))
            .collect();
        // Deterministic order (replay-stable corrected prompt → replay-stable shaper).
        failures.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));

        if failures.is_empty() {
            break outcome; // success — the round's steps all committed.
        }
        if round + 1 >= budget.max_rounds {
            break outcome; // budget exhausted — leave the failure dead-lettered (bounded additive).
        }

        // Build the next re-plan round (a NEW, round-distinct shaper).
        round += 1;
        let corrected = corrected_prompt(&base_prompt, &failures);
        current_wf = crate::workflows::replan_shaper(&model_id, &warrant, &corrected, round);
        current_shaper = current_wf.motes[0].clone();
    };

    Ok(ReplanLoopOutcome {
        run,
        rounds_used: round + 1,
        escalation,
    })
}

/// Fold the journal into an inspection [`Projection`] through the provider's
/// (accumulating) materializer — so every committed round's children are
/// re-derived and the snapshot reflects the true cross-round state (incl. each
/// step's terminal `FailureReason`).
fn fold_for_inspection<B, S>(
    provider: &ModelTopologyProvider<B, S>,
    shaper_wm: &kx_runtime::workflow::WorkflowMote,
    warrant: &WarrantSpec,
    journal: &Arc<SqliteJournal>,
) -> Result<Projection, RuntimeError>
where
    B: InferenceBackend,
    S: ContentStore + Send + Sync + 'static,
{
    let materializer = provider.materializer(&shaper_wm.mote.def, warrant);
    Ok(Projection::from_journal_with_materializer(
        &**journal,
        materializer,
    )?)
}

/// Stage `staged` (a fresh round's decision) as the shaper's effect, then drive ONE
/// round through `run_with_seams`. `None` ⇒ the shaper is already committed (R49
/// replay) or dead-lettered ⇒ an empty broker (the materializer derives children
/// from the committed fact, or the terminal shaper derives none).
#[allow(clippy::too_many_arguments)]
fn drive_one_round<B, S>(
    config: &RuntimeConfig,
    store: &Arc<S>,
    journal: &Arc<SqliteJournal>,
    backend: &Arc<B>,
    registry: &Arc<dyn ToolRegistry>,
    rm: &LocalResourceManager,
    failure_policy: &FailurePolicy,
    round_wf: &DemoWorkflow,
    round_shaper: &kx_runtime::workflow::WorkflowMote,
    staged: Option<&TopologyDecision>,
    provider: &ModelTopologyProvider<B, S>,
) -> Result<RunOutcome, RuntimeError>
where
    B: InferenceBackend,
    S: ContentStore + Send + Sync + 'static,
{
    let mut responses: BTreeMap<MoteId, Vec<u8>> = BTreeMap::new();
    if let Some(td) = staged {
        responses.insert(round_shaper.mote.id, encode_topology_decision(td)?);
    }
    let sink = SnapshotSink::new();
    let executor = ModelExecutor::new(
        backend.clone(),
        store.clone(),
        sink.clone(),
        registry.clone(),
    );
    let broker = Arc::new(DemoBroker::new(
        store.clone(),
        responses,
        config.crash_at,
        Some(round_wf.stc_crash_target),
    ));
    let protocol = StandardCommitProtocol::new(store.clone(), journal.clone(), broker);
    let dummy = TopologyDecision {
        children: Vec::new(),
    };
    let decision_ref = staged.unwrap_or(&dummy);
    run_with_seams(
        config,
        round_wf,
        store.clone(),
        journal.clone(),
        rm,
        &executor,
        &protocol,
        Some((round_shaper, decision_ref)),
        Some(provider),
        Some(&sink),
        None,
        None,
        Some(failure_policy),
    )
}

/// Journal a terminal `Failed` for a shaper whose round was refused / escalated —
/// the same dead-letter fact PR-1 + [`run_model_loop`] write (a durable, auditable
/// record; the engine then skips the now-terminal shaper).
fn dead_letter_shaper(journal: &Arc<SqliteJournal>, shaper_id: MoteId) -> Result<(), RuntimeError> {
    journal.append(JournalEntry::Failed {
        mote_id: shaper_id,
        idempotency_key: *shaper_id.as_bytes(),
        seq: 0, // journal assigns
        reason_class: FailureReason::ExecutorRefused,
        reporter_id: TOPOLOGY_PROVIDER_REPORTER_ID,
    })?;
    Ok(())
}

/// A STABLE, rename-and-reorder-proof token for a [`kx_journal::FailureReason`].
///
/// **Load-bearing for R49.** This token is threaded into the re-plan shaper's
/// [`corrected_prompt`], which is identity-bearing (`config_subset` → `MoteDef::hash`
/// → the shaper's `MoteId`). Derived `Debug` is explicitly NOT a Rust stability
/// contract, so a future variant rename would silently shift the shaper's identity
/// and a cold re-fold of an OLD journal on the NEW binary would derive a DIFFERENT
/// chain (breaking replay). Keying off the canonical `as_u8()` (the `#[repr(u8)]`
/// discriminant — the journal's own on-disk reason encoding) makes the token
/// rename-proof; the `failure_reason_token_is_frozen` test pins the mapping so any
/// reorder/renumber that would shift identity bytes fails CI. `Debug` stays for
/// human-facing `tracing` ONLY — never for bytes that enter a `MoteId`.
fn failure_reason_token(reason: Option<kx_journal::FailureReason>) -> &'static str {
    match reason.map(kx_journal::FailureReason::as_u8) {
        Some(0) => "timed-out",
        Some(1) => "executor-refused",
        Some(2) => "validator-rejected",
        Some(3) => "worker-crashed",
        Some(4) => "upstream-repudiated",
        Some(5) => "unsafe-world-mutating-construction",
        Some(6) => "compensated-at-least-once",
        Some(7) => "quarantined-at-least-once",
        // F4: the engine dead-letter (a budget-exhausted transient or a terminal-logic
        // dispatch failure the loop gave up on). Distinct from `timed-out` (a liveness
        // pre-commit-crash) so the AL2 re-plan reads an accurate signal.
        Some(8) => "dead-lettered",
        // A None (transient/pre-commit-crash → no retained terminal reason) or any
        // future, not-yet-tokenized variant maps to a fixed sentinel — still stable.
        _ => "transient-or-unknown",
    }
}

/// Build the next round's failure-corrected planning instruction (deterministic —
/// `failures` is pre-sorted and each reason renders via the STABLE
/// [`failure_reason_token`], so a cold re-fold reconstructs the SAME prompt ⇒ the
/// SAME re-plan shaper identity, R49). The reasons are the low-entropy
/// [`kx_journal::FailureReason`] enum only (never result bytes / secrets — SN-8).
fn corrected_prompt(
    base: &str,
    failures: &[(MoteId, Option<kx_journal::FailureReason>)],
) -> String {
    let mut s = String::from(base);
    s.push_str(
        "\n\nThe previous attempt left failed step(s). Respond with a `replan` envelope: \
         either `next_steps` that retry or replace them (corrected context / a role whose \
         authority fits), or `flag_human` with a reason if you cannot fix it within your \
         authority. Failed step(s):",
    );
    for (id, reason) in failures {
        let label = failure_reason_token(*reason);
        let _ = write!(s, "\n- step {id} failed (reason: {label})");
    }
    s
}

#[cfg(test)]
mod replan_unit_tests {
    use super::*;
    use kx_journal::FailureReason;

    #[test]
    fn failure_reason_token_is_frozen() {
        // PINS the identity-bearing token for every FailureReason. Keyed off the
        // canonical `as_u8()` (the on-disk reason encoding), so a variant RENAME
        // can NEVER shift a re-plan shaper's MoteId (R49 forward-compat). Any
        // reorder/renumber that changes a token here is a deliberate, CI-caught
        // identity change.
        assert_eq!(failure_reason_token(None), "transient-or-unknown");
        assert_eq!(
            failure_reason_token(Some(FailureReason::TimedOut)),
            "timed-out"
        );
        assert_eq!(
            failure_reason_token(Some(FailureReason::ExecutorRefused)),
            "executor-refused"
        );
        assert_eq!(
            failure_reason_token(Some(FailureReason::ValidatorRejected)),
            "validator-rejected"
        );
        assert_eq!(
            failure_reason_token(Some(FailureReason::WorkerCrashed)),
            "worker-crashed"
        );
        assert_eq!(
            failure_reason_token(Some(FailureReason::UpstreamRepudiated)),
            "upstream-repudiated"
        );
        assert_eq!(
            failure_reason_token(Some(FailureReason::UnsafeWorldMutatingConstruction)),
            "unsafe-world-mutating-construction"
        );
        assert_eq!(
            failure_reason_token(Some(FailureReason::CompensatedAtLeastOnce)),
            "compensated-at-least-once"
        );
        assert_eq!(
            failure_reason_token(Some(FailureReason::QuarantinedAtLeastOnce)),
            "quarantined-at-least-once"
        );
        // F4: the engine dead-letter token (discriminant 8). Existing tokens 0..=7
        // stay byte-frozen above, so adding this cannot shift any committed identity.
        assert_eq!(
            failure_reason_token(Some(FailureReason::DeadLettered)),
            "dead-lettered"
        );
    }

    #[test]
    fn corrected_prompt_is_deterministic_and_reason_stable() {
        let base = "plan";
        let f = [(
            MoteId::from_bytes([3; 32]),
            Some(FailureReason::ExecutorRefused),
        )];
        let a = corrected_prompt(base, &f);
        let b = corrected_prompt(base, &f);
        assert_eq!(
            a, b,
            "same inputs ⇒ byte-identical prompt (identity-stable)"
        );
        assert!(a.contains("executor-refused"), "stable token, not Debug");
        assert!(
            !a.contains("ExecutorRefused"),
            "Debug spelling never enters identity"
        );
    }
}
