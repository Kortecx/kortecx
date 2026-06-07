//! PR-3 (AL2) — "the model re-plans on failure", deterministically (NO GGUF).
//!
//! A [`ScriptedBackend`] returns a fixed, *call-indexed* sequence of completions
//! (and injected failures) so a re-plan loop is fully reproducible without a model.
//! It drives the REAL orchestrator + the shipped materializer through
//! [`kx_model_harness::run_replan_loop`] to prove, without a model:
//!
//! - **corrected-context re-plan** — round 0 spawns a step that dead-letters; the
//!   driver reads WHY (`failure_reason_of`), the model proposes a corrected round,
//!   and the prior round's committed steps are NEVER touched (D76);
//! - **durability / R49** — re-running the driver over the SAME committed journal
//!   reconstructs the SAME chain, serving every committed round's decision from the
//!   fact (the model is NOT re-sampled — the skip-already-committed-round guard);
//! - **bounded** — a step that keeps failing exhausts `max_rounds` and the loop
//!   stops with the failure dead-lettered (never an infinite loop);
//! - **flag-a-human** — an escalation stops the loop and records the reason;
//! - **fail-closed** — a malformed re-plan round dead-letters that round's shaper.
//!
//! The call sequence per round is deterministic: the driver calls the model ONCE
//! for the round's plan (`decide`/`replan`), the shaper's effect is the staged
//! decision (broker-served, NO model call), then each PURE child dispatches through
//! the model once. So for a 1-child round: `[plan, child]`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use kx_content::LocalFsContentStore;
use kx_inference::{
    InferenceBackend, InferenceError, InferenceInput, InferenceOutput, InferenceParams,
};
use kx_journal::SqliteJournal;
use kx_model_harness::{run_replan_loop, workflows, LoopBudget, ReplanLoopOutcome};
use kx_mote::{EffectPattern, LogicRef, ModelId, NdClass, PromptTemplateHash, RoleId, ToolName};
use kx_planner::{InMemoryRoleRecipes, RoleRecipe, RoleRecipeResolver};
use kx_projection::{MoteState, Projection};
use kx_runtime::{DemoWorkflow, Mode, RuntimeConfig};
use kx_tool_registry::{InMemoryToolRegistry, ToolRegistry};
use kx_warrant::{
    ExecutorClass, FsScope, ModelRoute, MoteClass, NetScope, ResourceCeiling, WarrantSpec,
};

const MODEL: &str = "stub-replan-model:test:0";
const PLAN_PROMPT: &str = "Propose the next steps as a loop_proposal envelope.";
const SHAPER_SEED: u8 = 0x71;

fn model_id() -> ModelId {
    ModelId(MODEL.into())
}

/// One scripted backend response, keyed by global dispatch index.
#[derive(Clone)]
enum Reply {
    /// Return these completion bytes.
    Ok(Vec<u8>),
    /// Return a backend failure (→ the dispatching Mote dead-letters terminally
    /// with `FailureReason::ExecutorRefused`).
    Fail,
}

/// A backend that replays a fixed `Reply` sequence by call index (trailing calls
/// get a benign `b"done"`). Records every call so a test can assert "served from
/// the committed fact, never re-sampled".
struct ScriptedBackend {
    replies: Mutex<std::vec::IntoIter<Reply>>,
    calls: Arc<AtomicUsize>,
}

impl ScriptedBackend {
    fn new(replies: Vec<Reply>, calls: Arc<AtomicUsize>) -> Self {
        Self {
            replies: Mutex::new(replies.into_iter()),
            calls,
        }
    }
}

impl InferenceBackend for ScriptedBackend {
    fn dispatch(
        &self,
        model_id: &ModelId,
        _input: &InferenceInput,
        _params: &InferenceParams,
        _warrant: &WarrantSpec,
    ) -> Result<InferenceOutput, InferenceError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let bytes = match self.replies.lock().unwrap().next() {
            Some(Reply::Ok(b)) => b,
            None => b"done".to_vec(), // trailing calls succeed benignly
            Some(Reply::Fail) => {
                return Err(InferenceError::ModelNotFound {
                    model_id: model_id.0.clone(),
                })
            }
        };
        Ok(InferenceOutput {
            bytes,
            output_tokens: 1,
            backend_name: "scripted",
            model_id: model_id.clone(),
            elapsed: Duration::from_millis(0),
        })
    }
    fn supports(&self, _model_id: &ModelId) -> bool {
        true
    }
    fn name(&self) -> &'static str {
        "scripted"
    }
}

/// A permissive WORLD-MUTATING warrant routed to [`model_id`].
fn warrant() -> WarrantSpec {
    WarrantSpec {
        mote_class: MoteClass::WorldMutating,
        nd_class: MoteClass::WorldMutating,
        fs_scope: FsScope::empty(),
        net_scope: NetScope::None,
        syscall_profile_ref: kx_content::ContentRef::from_bytes([0; 32]),
        tool_grants: BTreeSet::new(),
        model_route: ModelRoute {
            model_id: model_id(),
            max_input_tokens: 8192,
            max_output_tokens: 1024,
            max_calls: 64,
        },
        resource_ceiling: ResourceCeiling {
            cpu_milli: 0,
            mem_bytes: 0,
            wall_clock_ms: 1000,
            fd_count: 0,
            disk_bytes: 0,
        },
        environment_ref: None,
        executor_class: ExecutorClass::Bwrap,
        ..Default::default()
    }
}

/// The vetted role→recipe allowlist (PURE, no tools) every round lowers through.
fn recipes() -> Arc<dyn RoleRecipeResolver> {
    let r = InMemoryRoleRecipes::new();
    for (i, name) in ["reader", "summarizer"].iter().enumerate() {
        let tag = u8::try_from(i).unwrap();
        r.register(
            RoleId((*name).into()),
            RoleRecipe {
                logic_ref: LogicRef::from_bytes([0x90 + tag; 32]),
                model_id: model_id(),
                prompt_template_hash: PromptTemplateHash::from_bytes([0xA0 + tag; 32]),
                tool_contract: BTreeMap::new(),
                capability: ToolName("kx-model".into()),
                nd_class: NdClass::Pure,
                effect_pattern: EffectPattern::IdempotentByConstruction,
                inference_params: InferenceParams::default(),
                deterministic_check: None,
            },
        );
    }
    Arc::new(r)
}

fn config_for(dir: &Path) -> RuntimeConfig {
    RuntimeConfig {
        journal_path: dir.join("j.sqlite"),
        content_root: dir.join("c"),
        mode: Mode::Run,
        crash_at: None,
        checkpoint_every: None,
        audit_log: None,
    }
}

fn loop_wf() -> DemoWorkflow {
    workflows::loop_shaper(&model_id(), &warrant(), PLAN_PROMPT, SHAPER_SEED)
}

fn one_child_loop_proposal(role: &str) -> Vec<u8> {
    format!(
        r#"{{"loop_proposal":{{"version":1,"next_steps":[{{"role":"{role}","intent":"go"}}]}}}}"#
    )
    .into_bytes()
}

fn one_child_replan(role: &str) -> Vec<u8> {
    format!(r#"{{"replan":{{"version":1,"next_steps":[{{"role":"{role}","intent":"retry"}}]}}}}"#)
        .into_bytes()
}

fn flag_human_replan(reason: &str) -> Vec<u8> {
    format!(r#"{{"replan":{{"version":1,"flag_human":"{reason}"}}}}"#).into_bytes()
}

struct Driven {
    result: ReplanLoopOutcome,
    journal: Arc<SqliteJournal>,
}

/// Drive [`run_replan_loop`] over a fresh store+journal in `dir` with a scripted
/// backend. `dir` persists, so [`redrive`] can simulate a process restart.
fn drive(dir: &Path, replies: Vec<Reply>, budget: LoopBudget) -> Driven {
    let config = config_for(dir);
    let store = Arc::new(LocalFsContentStore::open(&config.content_root).unwrap());
    let journal = Arc::new(SqliteJournal::open(&config.journal_path).unwrap());
    let calls = Arc::new(AtomicUsize::new(0));
    let backend = Arc::new(ScriptedBackend::new(replies, calls));
    let registry: Arc<dyn ToolRegistry> = Arc::new(InMemoryToolRegistry::with_builtins());
    let result = run_replan_loop(
        &config,
        store,
        journal.clone(),
        backend,
        registry,
        recipes(),
        &loop_wf(),
        budget,
    )
    .expect("the re-plan loop completes (a failing step is not a run error)");
    Driven { result, journal }
}

/// Re-drive over the SAME persisted `dir` (a fresh process / fresh provider, with a
/// fresh `replies` script) — a process-restart simulation. A committed round serves
/// its decision from the fact (NO model call); only an UNCOMMITTED tail consumes
/// `replies`. Returns the outcome + the number of model calls the replay made
/// (`0` ⇒ a complete journal fully served from facts; `>0` ⇒ a mid-chain resume
/// only re-sampled the incomplete tail, never a committed round).
fn redrive(dir: &Path, replies: Vec<Reply>, budget: LoopBudget) -> (ReplanLoopOutcome, usize) {
    let config = config_for(dir);
    let store = Arc::new(LocalFsContentStore::open(&config.content_root).unwrap());
    let journal = Arc::new(SqliteJournal::open(&config.journal_path).unwrap());
    let calls = Arc::new(AtomicUsize::new(0));
    let backend = Arc::new(ScriptedBackend::new(replies, calls.clone()));
    let registry: Arc<dyn ToolRegistry> = Arc::new(InMemoryToolRegistry::with_builtins());
    let out = run_replan_loop(
        &config,
        store,
        journal,
        backend,
        registry,
        recipes(),
        &loop_wf(),
        budget,
    )
    .expect("replay completes");
    (out, calls.load(Ordering::SeqCst))
}

/// Plain fold of `dir`'s journal — for asserting the final committed/failed set
/// after a [`redrive`] (a fresh process re-opens the persisted journal).
fn fold_dir(dir: &Path) -> Projection {
    Projection::from_journal(&SqliteJournal::open(&config_for(dir).journal_path).unwrap()).unwrap()
}

/// Plain fold (no materializer) — each Mote's state from its own journal entries.
fn fold(d: &Driven) -> Projection {
    Projection::from_journal(&*d.journal).unwrap()
}

// ---------------------------------------------------------------------------

#[test]
fn two_round_replan_corrected_context_appends_and_cold_refolds() {
    // Round 0: plan one "reader" child → it FAILS at dispatch (Reply::Fail).
    // Round 1: re-plan one "reader" child → it succeeds.
    // Calls: [plan0, child0=FAIL, replan1, child1=ok].
    let replies = vec![
        Reply::Ok(one_child_loop_proposal("reader")),
        Reply::Fail,
        Reply::Ok(one_child_replan("reader")),
        Reply::Ok(b"done".to_vec()),
    ];
    let dir = tempfile::tempdir().unwrap();
    let d = drive(dir.path(), replies, LoopBudget::default());

    assert_eq!(
        d.result.rounds_used, 2,
        "one initial round + one re-plan round"
    );
    assert!(d.result.escalation.is_none(), "no escalation");

    let p = fold(&d);
    // 2 shapers + the round-1 child commit; the round-0 child stays dead-lettered.
    assert_eq!(p.committed_count(), 3, "2 shapers + the corrected child");
    assert_eq!(
        p.failed_count(),
        1,
        "the round-0 step is a durable Failed fact"
    );
    // The round-0 shaper (a fact) and its committed-prefix are intact (D76).
    assert_eq!(
        p.state_of(&loop_wf().shaper_id),
        MoteState::Committed,
        "round-0 shaper committed; its decision is never re-touched"
    );

    // R49: re-running over the SAME (complete) journal serves every committed round
    // from the fact through the SHIPPED recovery fold (the driver's
    // `from_journal_with_materializer`), so the model is NEVER re-sampled.
    let (replay, replay_calls) = redrive(dir.path(), vec![], LoopBudget::default());
    assert_eq!(
        replay_calls, 0,
        "no model call on replay — the whole multi-round chain served from facts"
    );
    assert_eq!(replay.rounds_used, 2, "the chain reconstructs identically");
    let rp = fold_dir(dir.path());
    assert_eq!(
        rp.committed_count(),
        3,
        "replay reproduces the committed set (R49)"
    );
    assert_eq!(rp.failed_count(), 1);
}

#[test]
fn mid_chain_crash_resumes_and_corrects_without_resampling_round_0() {
    // The NON-NEGOTIABLE durability gate: a crash BETWEEN rounds resumes by
    // re-folding. Simulate it by capping round 0 (`max_rounds = 1`) so the run
    // stops with round-0's step dead-lettered and NO round 1 — a journal frozen
    // mid-chain. Then "restart" (a fresh process via `redrive`) with more budget +
    // the round-1 correction: the driver must SERVE round 0 from the fact (NOT
    // re-sample it) and resume at round 1.
    let dir = tempfile::tempdir().unwrap();
    let _ = drive(
        dir.path(),
        vec![Reply::Ok(one_child_loop_proposal("reader")), Reply::Fail],
        LoopBudget {
            max_rounds: 1,
            max_children: 8,
        },
    );
    // After the cap: round-0 shaper committed, round-0 child dead-lettered, no round 1.
    let mid = fold_dir(dir.path());
    assert_eq!(
        mid.committed_count(),
        1,
        "only the round-0 shaper committed"
    );
    assert_eq!(
        mid.failed_count(),
        1,
        "round-0 step is a durable Failed fact"
    );

    // Restart with budget for the correction. The redrive script supplies ONLY the
    // round-1 plan + child — round 0 is served from the journal, never re-sampled.
    let (resumed, calls) = redrive(
        dir.path(),
        vec![
            Reply::Ok(one_child_replan("reader")),
            Reply::Ok(b"done".to_vec()),
        ],
        LoopBudget::default(),
    );
    assert_eq!(
        resumed.rounds_used, 2,
        "resume reconstructs round 0 (served) then drives the round-1 correction"
    );
    // Exactly the round-1 plan + child were sampled — round 0's plan was NOT
    // re-run (else `calls` would be 3+ and round 0 would be re-sampled).
    assert_eq!(
        calls, 2,
        "only the uncommitted tail (round-1 plan + child) called the model"
    );
    let after = fold_dir(dir.path());
    assert_eq!(
        after.committed_count(),
        3,
        "2 shapers + the corrected child after the resume"
    );
    assert_eq!(
        after.failed_count(),
        1,
        "the original round-0 failure persists (D76)"
    );
    assert_eq!(
        after.state_of(&loop_wf().shaper_id),
        MoteState::Committed,
        "the round-0 decision survived the crash, served not re-sampled (R49)"
    );
}

#[test]
fn round_budget_exhaustion_fails_closed() {
    // Every round's child FAILS; max_rounds = 2 → the loop drives exactly 2 rounds
    // then stops with the last failure dead-lettered (bounded additive, no infinite loop).
    // Calls: [plan0, child0=FAIL, replan1, child1=FAIL].
    let replies = vec![
        Reply::Ok(one_child_loop_proposal("reader")),
        Reply::Fail,
        Reply::Ok(one_child_replan("reader")),
        Reply::Fail,
    ];
    let dir = tempfile::tempdir().unwrap();
    let budget = LoopBudget {
        max_rounds: 2,
        max_children: 8,
    };
    let d = drive(dir.path(), replies, budget);

    assert_eq!(d.result.rounds_used, 2, "stops after exactly max_rounds");
    assert!(d.result.escalation.is_none());
    let p = fold(&d);
    assert_eq!(p.committed_count(), 2, "both shapers committed");
    assert_eq!(p.failed_count(), 2, "both rounds' steps stay dead-lettered");
}

#[test]
fn flag_human_stops_the_loop_and_records_a_durable_reason() {
    // Round 0: a child fails. Round 1: the model escalates (flag_human).
    // Calls: [plan0, child0=FAIL, replan1=flag_human].
    let replies = vec![
        Reply::Ok(one_child_loop_proposal("reader")),
        Reply::Fail,
        Reply::Ok(flag_human_replan("needs a credential I cannot grant")),
    ];
    let dir = tempfile::tempdir().unwrap();
    let d = drive(dir.path(), replies, LoopBudget::default());

    assert_eq!(
        d.result.escalation.as_deref(),
        Some("needs a credential I cannot grant"),
        "the escalation reason is surfaced"
    );
    let p = fold(&d);
    // round-0 shaper committed; round-0 child Failed; round-1 shaper dead-lettered.
    assert_eq!(
        p.state_of(&loop_wf().shaper_id),
        MoteState::Committed,
        "round-0 plan stands"
    );
    assert!(
        p.failed_count() >= 1,
        "the failed step remains a durable record"
    );
    assert_eq!(
        p.committed_count(),
        1,
        "only the round-0 shaper; nothing forced through"
    );
}

#[test]
fn no_failure_means_no_replan() {
    // The initial plan's child succeeds → the loop ends after round 0 (PR-2 parity).
    // Calls: [plan0, child0=ok].
    let replies = vec![
        Reply::Ok(one_child_loop_proposal("reader")),
        Reply::Ok(b"done".to_vec()),
    ];
    let dir = tempfile::tempdir().unwrap();
    let d = drive(dir.path(), replies, LoopBudget::default());
    assert_eq!(d.result.rounds_used, 1, "no correction needed");
    assert!(d.result.escalation.is_none());
    let p = fold(&d);
    assert_eq!(p.committed_count(), 2, "shaper + child");
    assert_eq!(p.failed_count(), 0);
}

#[test]
fn two_failures_in_one_round_are_replanned_once() {
    // Round 0 spawns TWO children, both FAIL → ONE re-plan round addresses them.
    // Calls: [plan0, child0a=FAIL, child0b=FAIL, replan1, child1=ok].
    let two = br#"{"loop_proposal":{"version":1,"next_steps":[{"role":"reader","intent":"a"},{"role":"summarizer","intent":"b"}]}}"#.to_vec();
    let replies = vec![
        Reply::Ok(two),
        Reply::Fail,
        Reply::Fail,
        Reply::Ok(one_child_replan("reader")),
        Reply::Ok(b"done".to_vec()),
    ];
    let dir = tempfile::tempdir().unwrap();
    let d = drive(dir.path(), replies, LoopBudget::default());
    assert_eq!(
        d.result.rounds_used, 2,
        "one re-plan addresses BOTH failures"
    );
    let p = fold(&d);
    assert_eq!(p.failed_count(), 2, "both round-0 steps stay dead-lettered");
    assert_eq!(
        p.committed_count(),
        3,
        "2 shapers + the one corrected child"
    );
}

#[test]
fn malformed_replan_round_dead_letters_that_shaper() {
    // Round 0: a child fails. Round 1: the model returns garbage → the round-1
    // shaper dead-letters fail-closed; the run completes; prior steps untouched.
    // Calls: [plan0, child0=FAIL, replan1=garbage].
    let replies = vec![
        Reply::Ok(one_child_loop_proposal("reader")),
        Reply::Fail,
        Reply::Ok(b"this is not a replan envelope".to_vec()),
    ];
    let dir = tempfile::tempdir().unwrap();
    let d = drive(dir.path(), replies, LoopBudget::default());
    assert!(d.result.escalation.is_none());
    let p = fold(&d);
    assert_eq!(
        p.state_of(&loop_wf().shaper_id),
        MoteState::Committed,
        "round-0 plan stands"
    );
    // round-0 child Failed + round-1 shaper dead-lettered = 2 Failed; nothing forced.
    assert!(
        p.failed_count() >= 2,
        "the refused round-1 shaper is a Failed fact too"
    );
    assert_eq!(p.committed_count(), 1, "only the round-0 shaper committed");
}

#[test]
fn replan_proposing_an_unknown_role_dead_letters_that_round() {
    // Round 0: a child fails. Round 1: the model proposes a role NOT in the recipe
    // allowlist → `lower` fails closed (UnknownRecipe) inside `replan` → the round-1
    // shaper dead-letters (SN-8: an unvetted role never materializes). The run
    // completes; the round-0 committed prefix is untouched.
    // Calls: [plan0, child0=FAIL, replan1=ghost-role].
    let replies = vec![
        Reply::Ok(one_child_loop_proposal("reader")),
        Reply::Fail,
        Reply::Ok(one_child_replan("ghost")), // "ghost" ∉ recipes()
    ];
    let dir = tempfile::tempdir().unwrap();
    let d = drive(dir.path(), replies, LoopBudget::default());
    assert!(d.result.escalation.is_none());
    let p = fold(&d);
    assert_eq!(
        p.state_of(&loop_wf().shaper_id),
        MoteState::Committed,
        "round-0 plan stands"
    );
    assert!(
        p.failed_count() >= 2,
        "round-0 child + the unvetted round-1 shaper both dead-letter"
    );
    assert_eq!(
        p.committed_count(),
        1,
        "no unvetted child ever materialized (SN-8)"
    );
}

#[test]
fn replan_over_max_children_dead_letters_that_round() {
    // Round 0: a child fails. Round 1: the model proposes MORE children than the
    // per-decision fan-out cap → `replan` fails closed → the round-1 shaper
    // dead-letters (bounded additive). max_children = 1, round-1 proposes 2.
    let two_replan = br#"{"replan":{"version":1,"next_steps":[{"role":"reader","intent":"a"},{"role":"summarizer","intent":"b"}]}}"#.to_vec();
    let replies = vec![
        Reply::Ok(one_child_loop_proposal("reader")),
        Reply::Fail,
        Reply::Ok(two_replan),
    ];
    let dir = tempfile::tempdir().unwrap();
    let d = drive(
        dir.path(),
        replies,
        LoopBudget {
            max_rounds: 4,
            max_children: 1,
        },
    );
    let p = fold(&d);
    assert!(
        p.failed_count() >= 2,
        "the over-cap round-1 fan-out is refused fail-closed"
    );
    assert_eq!(p.committed_count(), 1, "only the round-0 shaper committed");
}
