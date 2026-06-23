//! PR-4 (M5) — **the tool-call ReAct loop.** The model drives a bounded, durable,
//! crash-resumable Reason→Act→Observe→repeat loop: it proposes a tool, the runtime
//! ENFORCES + fires it (SN-8), the committed result becomes an OBSERVATION the next
//! turn reads back, until the model gives a final answer or a budget is hit.
//!
//! ## The two-fact split (the load-bearing shape)
//!
//! M5.2's fused single-call path commits the TOOL RESULT as a model Mote's sole
//! fact — so a refold cannot re-decode what the model proposed. The ReAct loop
//! instead writes TWO facts per acting turn:
//!
//! 1. a **turn Mote** ([`crate::workflows::react_turn`], ROND) whose committed
//!    `result_ref` is the model's RAW output (a `{"tool_call": …}` envelope or a
//!    final answer) — journal-durable, so a cold re-fold re-decodes the branch via
//!    [`crate::toolcall::parse_tool_call`] (R49), never re-sampling;
//! 2. an **observation Mote** ([`crate::workflows::react_tool_mote`], WM
//!    `StageThenCommit`) whose committed `result_ref` is the tool result.
//!
//! Each next turn declares Data edges to the FULL prior trajectory, so
//! [`kx_context_assembler::assemble`] (D78) reconstructs the whole transcript into
//! the model window (bounded by `window_bytes`, fail-closed on overflow).
//!
//! ## Where the loop lives
//!
//! In the harness DRIVER (like PR-3's [`crate::run_replan_loop`]); the engine +
//! frozen trio (`kx-executor`/`kx-scheduler`/`kx-inference`) are UNTOUCHED. The
//! model runs ONCE per fresh turn (`run_model_turn`); the orchestrator
//! ([`run_with_seams`]) only COMMITS the pre-computed turn fact + FIRES the decoded
//! tool through the SINGLE audited gate ([`crate::broker::dispatch_decoded_call`]).
//!
//! ## Bounded + fail-closed (composes with PR-1)
//!
//! Two independent hard caps ([`ReactBudget`]): `max_turns` (model turns) and
//! `max_tool_calls` (observations) — either exhausting stops the loop cleanly
//! ([`ReactStop::BudgetExhausted`]). PR-3 (A2) graceful recovery: a malformed /
//! ungranted / oversize proposal is NOT terminal — the rejected turn COMMITS (the
//! model sees its own bad proposal next turn), counts as a spent tool-call, and the
//! next turn is RE-PROMPTED with the durable reason so the model self-corrects
//! (mirrors the live serve coordinator). The loud [`ReactStop::DeadLettered`] fires
//! ONLY at budget exhaustion on a refused tail (never a panic, never a blind re-run,
//! never a fabricated answer). The unbounded durable loop stays cloud (D126 cat-B).
//!
//! ## Security (the prompt-injection surface)
//!
//! Tool results re-enter the prompt, so an observation is UNTRUSTED. It can NEVER
//! escalate: the warrant is FIXED for the whole run, and `parse_tool_call` enforces
//! the proposed tool ∈ `warrant.tool_grants` by exact crypto-equality (SN-8) — an
//! injected "call a tool you were not granted" decodes to `UngrantedTool` → the
//! turn dead-letters, no effect fires. The observation renders as `parent.<hex>`
//! content, never an authority turn.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use kx_capability::{BrokerError, BrokerHandle, CapabilityBroker, EffectRequest, INSTANCE_ID_LEN};
use kx_content::{ContentRef, ContentStore};
use kx_executor::{LocalResourceManager, StandardCommitProtocol};
use kx_inference::{inference_params_from_mote, InferenceBackend};
use kx_journal::SqliteJournal;
use kx_mote::{ModelId, Mote, MoteId, ToolName, ToolVersion};
use kx_projection::{MoteState, Projection, Snapshot};
use kx_runtime::workflow::WorkflowMote;
use kx_runtime::{
    run_with_seams, CrashPoint, DemoWorkflow, FailurePolicy, RunOutcome, RuntimeConfig,
    RuntimeError, SnapshotSink,
};
use kx_tool_registry::ToolRegistry;
use kx_warrant::WarrantSpec;

use crate::broker::dispatch_decoded_call;
use crate::toolcall::{self, ToolCall};
use crate::{context, prompt, react_reason, workflows, ModelExecutor};

/// Capability version reported on a ReAct turn's served fact (provenance only).
const REACT_CAPABILITY_VERSION: &str = "kx-react-0.1.0";

/// The per-run bound on a ReAct loop — the OSS "bounded additive" cap (the
/// unbounded durable loop stays cloud, D126 cat-B). Two INDEPENDENT bounds: a
/// turn that calls a tool consumes one `max_turns` AND one `max_tool_calls`; a
/// turn that answers consumes only one `max_turns`. A useful loop sets
/// `max_turns > max_tool_calls` (leaving a turn to read the last observation and
/// answer).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReactBudget {
    /// Max model turns the loop will drive.
    pub max_turns: u32,
    /// Max tool calls (observations) the loop will fire.
    pub max_tool_calls: u32,
}

impl Default for ReactBudget {
    fn default() -> Self {
        Self {
            max_turns: 8,
            max_tool_calls: 8,
        }
    }
}

/// Why a ReAct loop stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReactStop {
    /// The model produced a final answer (no tool call) — success.
    Answered,
    /// A budget (`max_turns` or `max_tool_calls`) was exhausted; the loop stopped
    /// cleanly with the work so far durably committed.
    BudgetExhausted,
    /// The loop exhausted its budget on REFUSED proposals (malformed / ungranted /
    /// oversize) without ever producing an answer — the loud terminal (PR-3/A2).
    /// Each refused turn COMMITS its output (the model self-corrects via the
    /// re-prompt); this fires only when the refused tail hits the budget. Also the
    /// terminal for a tool dispatch that fails fail-closed (the observation never
    /// commits) and for resuming a journal an OLD harness dead-lettered (`Failed`).
    DeadLettered,
}

/// The outcome of a bounded ReAct loop ([`run_react_loop`]).
#[derive(Debug, Clone)]
pub struct ReactLoopOutcome {
    /// The final round's [`RunOutcome`]. Its `digest` is the WHOLE-journal replay
    /// surface (every committed turn + observation), so a different process
    /// cold-folding the journal reproduces it (R49).
    pub run: RunOutcome,
    /// Model turns driven (1 ⇒ answered/refused on the first turn).
    pub turns_used: u32,
    /// Tool calls (observations) fired across the loop.
    pub tool_calls: u32,
    /// `Some(ref)` iff [`ReactStop::Answered`]: the committed final-answer fact.
    pub final_answer: Option<ContentRef>,
    /// Why the loop stopped.
    pub outcome: ReactStop,
}

/// **PR-4 — run a bounded model-driven ReAct (tool-call) loop.**
///
/// Drives turns from the seed `instruction` until the model answers, a budget is
/// hit, or a proposal is refused. Each acting turn writes two durable facts (the
/// model output + the observation); each next turn sees the full prior trajectory
/// via Data edges (D78). A crash between turns resumes by re-folding (committed
/// turns are served, never re-sampled; the tail re-fires exactly-once); a cold
/// re-fold reproduces the exact chain (R49). Composes with PR-1 (`max_attempts =
/// 1` ⇒ a refused/failing step dead-letters at once, never a blind re-run).
///
/// `registry` must resolve the run's MCP tool(s) (so `assemble` emits the menu and
/// the dispatch derives egress); `tool_broker` holds the concrete
/// `McpCapability` under each granted tool name; `instance_id` is the registered
/// run's id (D64) anchoring the run-scoped idempotency token. The `warrant` MUST
/// grant every tool the model may call.
///
/// # Errors
/// Propagates store / journal / orchestrator failures and a context-window overflow
/// (a typed assembly error, never a panic). A refused proposal, budget exhaustion,
/// and a final answer are NOT errors — they are reflected in [`ReactLoopOutcome`].
// Each argument is a distinct injected seam — mirrors `run_replan_loop`'s allow.
// `similar_names`: the loop is intrinsically about `turn_*` bindings.
#[allow(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    clippy::needless_pass_by_value,
    clippy::similar_names
)]
pub fn run_react_loop<B, S>(
    config: &RuntimeConfig,
    store: Arc<S>,
    journal: Arc<SqliteJournal>,
    backend: Arc<B>,
    registry: Arc<dyn ToolRegistry>,
    tool_broker: Arc<dyn CapabilityBroker>,
    instance_id: [u8; INSTANCE_ID_LEN],
    model_id: &ModelId,
    warrant: &WarrantSpec,
    instruction: &str,
    budget: ReactBudget,
) -> Result<ReactLoopOutcome, RuntimeError>
where
    B: InferenceBackend,
    S: ContentStore + Send + Sync + 'static,
{
    let rm = LocalResourceManager::dev_defaults();
    // PR-1: a refused/failing step is a terminal `Failed` (no retry, no blind re-run).
    let failure_policy = FailurePolicy::new(1, Duration::ZERO);

    let mut trajectory: Vec<MoteId> = Vec::new();
    let mut turns_used: u32 = 0;
    let mut tool_calls: u32 = 0;
    let mut final_answer: Option<ContentRef> = None;
    // PR-3 (A2): when the prior turn's proposal was refused, the next turn is
    // re-prompted with the durable reason so the model self-corrects (mirrors the
    // live serve coordinator's `advance_react_chain`). `None` ⇒ the base instruction.
    let mut pending_reprompt: Option<String> = None;

    // The loop ALWAYS drives at least turn 0; every exit `break`s with
    // `(RunOutcome, ReactStop)` (the run's `digest` is the whole-journal surface).
    let (run, outcome): (RunOutcome, ReactStop) = loop {
        let turn = turns_used;
        // PR-3 (A2): a refused prior turn re-prompts THIS turn (deterministic — a
        // pure function of the prior decode); the instruction rides the turn's
        // identity (PROMPT_KEY), so a re-prompted turn gets a distinct MoteId.
        let turn_instruction = match &pending_reprompt {
            Some(reason) => react_reason::render_reprompt(instruction, reason),
            None => instruction.to_string(),
        };
        let turn_wf =
            workflows::react_turn(model_id, warrant, &turn_instruction, turn, &trajectory);
        let turn_wm = turn_wf.motes[0].clone();
        let turn_id = turn_wm.mote.id;
        turns_used += 1;

        // Fold + skip-already-committed guard (R49 / crash-resume).
        let proj = Projection::from_journal(&*journal)?;
        let turn_state = proj.state_of(&turn_id);
        if turn_state == MoteState::Failed {
            // A prior run refused this turn — drive to a clean stop (serves the
            // journal, returns the whole-journal digest).
            let run = drive_react_round(
                config,
                &store,
                &journal,
                &backend,
                &registry,
                &tool_broker,
                instance_id,
                &rm,
                &failure_policy,
                &turn_wf,
                BTreeMap::new(),
                BTreeMap::new(),
            )?;
            break (run, ReactStop::DeadLettered);
        }

        // Obtain the model's raw output: served (committed → R49 replay) or fresh.
        let raw: Vec<u8> = if turn_state == MoteState::Committed {
            let r = proj.result_ref_of(&turn_id).ok_or_else(|| {
                RuntimeError::Config("react: committed turn lacks a result_ref".to_string())
            })?;
            store
                .get(&r)
                .map_err(|e| {
                    RuntimeError::Config(format!("react: committed turn output missing: {e}"))
                })?
                .to_vec()
        } else {
            run_model_turn(
                &backend,
                &store,
                &registry,
                &turn_wm.mote,
                warrant,
                &proj.snapshot(),
            )?
        };

        // Decode the proposal FAIL-CLOSED via the ONE authority gate. T-MULTI-ELEMENT-
        // TOOLCALLS: the plural gate decodes ALL proposed calls (N≥1) so a multi-element
        // body fires every call (mirrors the live coordinator); a single call is one.
        let branch = toolcall::parse_tool_calls(&raw, warrant, toolcall::max_args_bytes(warrant));

        // PR-3 (A2): a refused proposal is NOT terminal — mirror the live serve
        // coordinator. COMMIT the rejected turn (so the next turn sees the bad
        // proposal in its trajectory), count it as a spent tool-call, and re-prompt
        // with the durable reason. The loud `DeadLettered` fires ONLY at budget
        // exhaustion on a refused tail (BUG-27 preserved; never a fabricated answer,
        // GR15). A COMMITTED turn that re-decodes to `Err` is a previously-rejected
        // turn on REPLAY: the SAME path re-derives its in-memory state (trajectory,
        // count, re-prompt) from the served fact WITHOUT re-committing or
        // mis-classifying it as an answer (the deterministic re-fold law —
        // `parse_tool_call` is pure over the same bytes). A registry-miss /
        // schema-invalid resolves to `Ok(Some)` and fails closed later at dispatch
        // (out of A2's decode-refusal scope — its own fail-closed stop below).
        if let Err(error) = &branch {
            let reason = react_reason::bounded_reason(react_reason::decode_error_reason(error));
            tracing::warn!(turn, %reason, "react: proposal refused — re-prompting (A2)");
            // Commit the rejected turn alone (turn-only round; a committed turn is
            // served by the engine's P0.4 gate, its map entry then inert).
            let mut turn_responses: BTreeMap<MoteId, Vec<u8>> = BTreeMap::new();
            if turn_state != MoteState::Committed {
                turn_responses.insert(turn_id, raw.clone());
            }
            let reject_wf = DemoWorkflow {
                motes: vec![turn_wm.clone()],
                stc_crash_target: workflows::sentinel_shaper(),
                vtc_crash_target: workflows::sentinel_shaper(),
                shaper_id: workflows::sentinel_shaper(),
            };
            let run = drive_react_round(
                config,
                &store,
                &journal,
                &backend,
                &registry,
                &tool_broker,
                instance_id,
                &rm,
                &failure_policy,
                &reject_wf,
                turn_responses,
                BTreeMap::new(),
            )?;
            // The rejected turn MUST have committed (defensive: a store fault leaves
            // it non-committed ⇒ stop fail-closed rather than loop on a phantom turn).
            if Projection::from_journal(&*journal)?.state_of(&turn_id) != MoteState::Committed {
                break (run, ReactStop::DeadLettered);
            }
            trajectory.push(turn_id);
            tool_calls += 1; // a refused proposal is a spent tool-call attempt
                             // Budget gate (the harness mirror, line-for-line with the tool path): a
                             // refused TAIL at exhaustion is the LOUD terminal; otherwise re-prompt.
            if tool_calls >= budget.max_tool_calls || turns_used >= budget.max_turns {
                break (run, ReactStop::DeadLettered);
            }
            pending_reprompt = Some(reason);
            continue;
        }

        // A non-refused turn clears any prior re-prompt steer. A committed turn
        // re-decodes to the SAME `Ok` it committed on; an EMPTY list ⇒ final answer,
        // N≥1 calls ⇒ a (possibly multi-element) tool proposal. T-MULTI-ELEMENT-
        // TOOLCALLS: the harness mirrors the live coordinator's fire-ALL-N drain (the
        // R49 byte-twin) — N call-indexed observations fire in one turn.
        pending_reprompt = None;
        // An Err proposal already took the A2 re-prompt path above (the `continue`), so
        // here `branch` is always `Ok`; an empty list = a normal answer (the fail-closed
        // default), so a future-impossible `Err` degrades safely, never panics.
        let calls: Vec<ToolCall> = branch.unwrap_or_default();

        // Build this round's workflow: [turn] (+ one call-indexed [observation] per
        // proposed call). The fresh turn output is staged via the ReactBroker; a
        // committed turn is served by the engine's P0.4 gate (its map entry is inert).
        let tool_wms: Vec<WorkflowMote> = calls
            .iter()
            .enumerate()
            .map(|(i, c)| {
                workflows::react_tool_mote(
                    model_id,
                    warrant,
                    &c.name,
                    &c.version,
                    turn,
                    u32::try_from(i).unwrap_or(u32::MAX),
                    turn_id,
                )
            })
            .collect();
        let mut round_motes = vec![turn_wm.clone()];
        // PreCommitStc targets the FIRST observation (the single-call crash injection
        // point is unchanged; a multi-call batch crashes on call_index 0).
        let mut stc_crash_target = workflows::sentinel_shaper();
        if let Some(first) = tool_wms.first() {
            stc_crash_target = first.mote.id;
        }
        for tw in &tool_wms {
            round_motes.push(tw.clone());
        }
        let round_wf = DemoWorkflow {
            motes: round_motes,
            stc_crash_target,
            vtc_crash_target: workflows::sentinel_shaper(),
            shaper_id: workflows::sentinel_shaper(),
        };

        let mut turn_responses: BTreeMap<MoteId, Vec<u8>> = BTreeMap::new();
        if turn_state != MoteState::Committed {
            turn_responses.insert(turn_id, raw.clone());
        }
        let mut tool_call_map: BTreeMap<MoteId, ToolCall> = BTreeMap::new();
        for (tw, c) in tool_wms.iter().zip(calls.iter()) {
            tool_call_map.insert(tw.mote.id, c.clone());
        }

        let run = drive_react_round(
            config,
            &store,
            &journal,
            &backend,
            &registry,
            &tool_broker,
            instance_id,
            &rm,
            &failure_policy,
            &round_wf,
            turn_responses,
            tool_call_map,
        )?;

        if tool_wms.is_empty() {
            // Final answer (no tool call) — the loop is done; the committed turn
            // fact IS the answer.
            final_answer = Projection::from_journal(&*journal)?.result_ref_of(&turn_id);
            break (run, ReactStop::Answered);
        }
        // The tools fired — record the trajectory (turn output + EVERY observation in
        // call_index order), count each call against the budget, and bound the loop.
        // BACK-PRESSURE: every observation MUST have committed before the next turn (a
        // fail-closed tool dispatch leaves one non-committed (PR-1 `Failed`) ⇒ stop
        // cleanly rather than feed a non-existent observation forward).
        let post = Projection::from_journal(&*journal)?;
        if tool_wms
            .iter()
            .any(|tw| post.state_of(&tw.mote.id) != MoteState::Committed)
        {
            tracing::warn!(
                turn,
                "react: a tool dispatch did not commit — stopping the loop fail-closed"
            );
            break (run, ReactStop::DeadLettered);
        }
        trajectory.push(turn_id);
        for tw in &tool_wms {
            trajectory.push(tw.mote.id);
        }
        // Each fired call counts against max_tool_calls (a batch of N spends N).
        tool_calls += u32::try_from(tool_wms.len()).unwrap_or(u32::MAX);
        if tool_calls >= budget.max_tool_calls {
            break (run, ReactStop::BudgetExhausted);
        }
        if turns_used >= budget.max_turns {
            break (run, ReactStop::BudgetExhausted);
        }
    };

    Ok(ReactLoopOutcome {
        run,
        turns_used,
        tool_calls,
        final_answer,
        outcome,
    })
}

/// Run the model ONCE for a fresh ReAct turn: assemble the turn's full upstream
/// trajectory + tool menu (D78) and dispatch, returning the raw completion bytes.
/// Mirrors [`crate::topology_provider`]'s `run_model_once` (minus the cross-round
/// budget, which the ReAct loop bounds itself). A window overflow surfaces a typed
/// error (never a panic).
fn run_model_turn<B, S>(
    backend: &Arc<B>,
    store: &Arc<S>,
    registry: &Arc<dyn ToolRegistry>,
    turn_mote: &Mote,
    warrant: &WarrantSpec,
    snapshot: &Snapshot,
) -> Result<Vec<u8>, RuntimeError>
where
    B: InferenceBackend,
    S: ContentStore + Send + Sync + 'static,
{
    let instruction = prompt::raw_prompt(turn_mote)
        .ok_or_else(|| RuntimeError::Config("react turn carries no instruction".to_string()))?;
    let sink = SnapshotSink::new();
    sink.publish(snapshot.clone());
    let input = context::model_input(
        turn_mote,
        warrant,
        &instruction,
        &sink,
        &**store,
        &**registry,
    )
    .map_err(|e| RuntimeError::Config(format!("react context assembly: {e}")))?;
    let params = inference_params_from_mote(turn_mote, warrant)
        .map_err(|e| RuntimeError::Config(format!("react inference params: {e}")))?;
    let out = backend
        .dispatch(&turn_mote.def.model_id, &input, &params, warrant)
        .map_err(|e| RuntimeError::Config(format!("react model dispatch: {e}")))?;
    Ok(out.bytes)
}

/// Drive ONE ReAct round (a 1-or-2-Mote flat workflow) through the orchestrator
/// with a [`ReactBroker`] that serves the pre-computed turn output + fires the
/// decoded tool. Returns the round's [`RunOutcome`] (whole-journal digest, R49).
#[allow(clippy::too_many_arguments)]
fn drive_react_round<B, S>(
    config: &RuntimeConfig,
    store: &Arc<S>,
    journal: &Arc<SqliteJournal>,
    backend: &Arc<B>,
    registry: &Arc<dyn ToolRegistry>,
    tool_broker: &Arc<dyn CapabilityBroker>,
    instance_id: [u8; INSTANCE_ID_LEN],
    rm: &LocalResourceManager,
    failure_policy: &FailurePolicy,
    workflow: &DemoWorkflow,
    turn_responses: BTreeMap<MoteId, Vec<u8>>,
    tool_calls: BTreeMap<MoteId, ToolCall>,
) -> Result<RunOutcome, RuntimeError>
where
    B: InferenceBackend,
    S: ContentStore + Send + Sync + 'static,
{
    let sink = SnapshotSink::new();
    let executor = ModelExecutor::new(
        backend.clone(),
        store.clone(),
        sink.clone(),
        registry.clone(),
    );
    let broker = Arc::new(ReactBroker {
        store: store.clone(),
        turn_responses,
        tool_calls,
        tool_broker: tool_broker.clone(),
        registry: registry.clone(),
        instance_id,
        crash_at: config.crash_at,
        stc_crash_target: Some(workflow.stc_crash_target),
    });
    let protocol = StandardCommitProtocol::new(store.clone(), journal.clone(), broker);
    run_with_seams(
        config,
        workflow,
        store.clone(),
        journal.clone(),
        rm,
        &executor,
        &protocol,
        None, // shaper — a ReAct round is a flat 1-or-2-Mote DAG
        None, // topology_provider — no fan-out
        Some(&sink),
        None, // capture_sink — off for the loop foundation
        None, // audit_sink (R4) — off for the loop foundation
        Some(failure_policy),
    )
}

/// A [`CapabilityBroker`] for one ReAct round: it SERVES each turn's pre-computed
/// model output (content-addressed, like `DemoBroker`) and FIRES each observation's
/// decoded tool call through the SINGLE audited gate
/// ([`dispatch_decoded_call`]). A Mote in neither map is a bug (surfaced
/// fail-closed, never a silent default). Mirrors the `PreCommitStc` crash injection
/// of `DemoBroker` / `ModelBroker` so the observation's exactly-once-under-crash is
/// testable.
struct ReactBroker<S: ContentStore> {
    store: Arc<S>,
    /// Turn Mote id → the model's RAW output (staged as that turn's committed fact).
    turn_responses: BTreeMap<MoteId, Vec<u8>>,
    /// Observation Mote id → the decoded, warrant-checked tool call to fire.
    tool_calls: BTreeMap<MoteId, ToolCall>,
    tool_broker: Arc<dyn CapabilityBroker>,
    registry: Arc<dyn ToolRegistry>,
    instance_id: [u8; INSTANCE_ID_LEN],
    crash_at: Option<CrashPoint>,
    stc_crash_target: Option<MoteId>,
}

impl<S: ContentStore> ReactBroker<S> {
    fn maybe_crash_pre_commit_stc(&self, mote_id: MoteId) {
        if self.crash_at == Some(CrashPoint::PreCommitStc) && self.stc_crash_target == Some(mote_id)
        {
            CrashPoint::PreCommitStc.abort_now();
        }
    }
}

impl<S> CapabilityBroker for ReactBroker<S>
where
    S: ContentStore + Send + Sync,
{
    fn dispatch(
        &self,
        mote: &Mote,
        warrant: &WarrantSpec,
        capability: &ToolName,
        _request: EffectRequest,
    ) -> Result<BrokerHandle, BrokerError> {
        // (1) A turn Mote: stage the pre-computed model output verbatim. Content-
        //     addressing makes a recovery re-stage byte-identical → the same ref.
        if let Some(raw) = self.turn_responses.get(&mote.id) {
            let staged_ref = self
                .store
                .put(raw)
                .map_err(|e| BrokerError::StageWriteFailed {
                    capability: capability.clone(),
                    diagnostic: format!("{e}"),
                })?;
            self.maybe_crash_pre_commit_stc(mote.id);
            return Ok(BrokerHandle {
                staged_ref,
                capability: capability.clone(),
                capability_version: ToolVersion(REACT_CAPABILITY_VERSION.to_string()),
            });
        }
        // (2) An observation Mote: fire the decoded call through the audited gate
        //     (validate_args + net_scope ⊆ warrant + run-scoped idempotency).
        if let Some(call) = self.tool_calls.get(&mote.id) {
            // Keyed on the SAME `instance_id` every round, so a recovery re-dispatch
            // re-derives the SAME run-scoped idempotency token (remote exactly-once)
            // inside `dispatch_decoded_call`.
            let handle = dispatch_decoded_call(
                &*self.tool_broker,
                &*self.registry,
                mote,
                warrant,
                capability,
                call,
                &self.instance_id,
            )?;
            self.maybe_crash_pre_commit_stc(mote.id);
            return Ok(handle);
        }
        Err(BrokerError::StageWriteFailed {
            capability: capability.clone(),
            diagnostic: "react broker: dispatched a Mote that is neither a staged turn \
                         output nor a decoded tool call"
                .to_string(),
        })
    }

    fn probe_readback(
        &self,
        _mote: &Mote,
        _warrant: &WarrantSpec,
        _capability: &ToolName,
        _probe: EffectRequest,
    ) -> Result<Option<BrokerHandle>, BrokerError> {
        // No effect read-back: recovery relies on the deterministic idempotency-key
        // dedup at re-dispatch (same as DemoBroker / ModelBroker).
        Ok(None)
    }
}
