//! PR-2 (F-4) — "the model drives the loop", deterministically (NO GGUF).
//!
//! A stub backend stands in for the model, returning a fixed completion for every
//! dispatch. Drives the REAL orchestrator + the shipped materializer through
//! [`kx_model_harness::run_model_loop`] to prove, without a model:
//!
//! - **the model computes the topology** — a stub `{"loop_proposal": …}` is decoded
//!   (`decode_loop_proposal`), lowered through vetted recipes, committed as the
//!   shaper's `result_ref` fact, materialized into children that execute;
//! - **determinism / R49** — a cold re-fold over the committed journal re-derives
//!   byte-identical child `MoteId`s (the model's choice is replayed, never
//!   re-sampled — served from the fact, the materializer never calls the backend);
//! - **fail-closed (PR-1)** — a malformed / over-budget / unknown-role proposal
//!   dead-letters the shaper (terminal `Failed`) and the run completes with no
//!   children, never a panic;
//! - **`<think>`-stripping** — a Qwen3 reasoning preamble is stripped before decode.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use kx_content::LocalFsContentStore;
use kx_inference::{
    InferenceBackend, InferenceError, InferenceInput, InferenceOutput, InferenceParams,
};
use kx_journal::SqliteJournal;
use kx_model_harness::workflows;
use kx_model_harness::{run_model_loop, LoopBudget};
use kx_mote::{
    EffectPattern, LogicRef, ModelId, MoteDef, NdClass, PromptTemplateHash, RoleId, ToolName,
};
use kx_planner::{InMemoryRoleRecipes, RoleRecipe, RoleRecipeResolver};
use kx_projection::{
    DefaultTopologyMaterializer, InMemoryMoteDefRegistry, InheritFromShaperResolver, MoteState,
    Projection,
};
use kx_runtime::{DemoWorkflow, Mode, RunOutcome, RuntimeConfig, RuntimeError};
use kx_tool_registry::{InMemoryToolRegistry, ToolRegistry};
use kx_warrant::{
    ExecutorClass, FsScope, InMemoryRoleRegistry, ModelRoute, MoteClass, NetScope, ResourceCeiling,
    Role, WarrantSpec,
};

const MODEL: &str = "stub-loop-model:test:0";
const PLAN_PROMPT: &str = "Propose the next steps as a loop_proposal envelope.";
const SHAPER_SEED: u8 = 0x71;

fn model_id() -> ModelId {
    ModelId(MODEL.into())
}

/// A stub backend returning a fixed `reply` for every dispatch, counting calls so
/// a test can assert "served from the committed fact, never re-sampled".
struct StubBackend {
    reply: Vec<u8>,
    calls: Arc<AtomicUsize>,
}

impl InferenceBackend for StubBackend {
    fn dispatch(
        &self,
        model_id: &ModelId,
        _input: &InferenceInput,
        _params: &InferenceParams,
        _warrant: &WarrantSpec,
    ) -> Result<InferenceOutput, InferenceError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(InferenceOutput {
            bytes: self.reply.clone(),
            output_tokens: 1,
            backend_name: "stub",
            model_id: model_id.clone(),
            elapsed: Duration::from_millis(0),
        })
    }
    fn supports(&self, _model_id: &ModelId) -> bool {
        true
    }
    fn name(&self) -> &'static str {
        "stub"
    }
}

/// A permissive WORLD-MUTATING warrant routed to [`model_id`] (so a ROND shaper +
/// its PURE children are admissible; the model route authorises dispatch, D35).
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

/// The vetted role→recipe allowlist (PURE, no tools) the proposal lowers through.
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

/// A well-formed two-child proposal naming the allowlisted roles.
fn two_child_proposal() -> Vec<u8> {
    br#"{"loop_proposal":{"version":1,"next_steps":[{"role":"reader","intent":"read"},{"role":"summarizer","intent":"sum"}]}}"#.to_vec()
}

struct Driven {
    result: Result<RunOutcome, RuntimeError>,
    calls: usize,
    store: Arc<LocalFsContentStore>,
    journal: Arc<SqliteJournal>,
    workflow: DemoWorkflow,
}

fn drive(dir: &Path, reply: &[u8], budget: LoopBudget) -> Driven {
    let config = config_for(dir);
    let store = Arc::new(LocalFsContentStore::open(&config.content_root).unwrap());
    let journal = Arc::new(SqliteJournal::open(&config.journal_path).unwrap());
    let calls = Arc::new(AtomicUsize::new(0));
    let backend = Arc::new(StubBackend {
        reply: reply.to_vec(),
        calls: calls.clone(),
    });
    let registry: Arc<dyn ToolRegistry> = Arc::new(InMemoryToolRegistry::with_builtins());
    let workflow = loop_wf();
    let result = run_model_loop(
        &config,
        store.clone(),
        journal.clone(),
        backend,
        registry,
        recipes(),
        &workflow,
        budget,
    );
    Driven {
        result,
        calls: calls.load(Ordering::SeqCst),
        store,
        journal,
        workflow,
    }
}

/// Cold re-fold the committed journal through a fresh materializer (resolving the
/// allowlisted roles) — the SHIPPED recovery path, which decodes the committed
/// decision fact and NEVER calls a model.
fn cold_fold(d: &Driven) -> Projection {
    let shaper_def: MoteDef = d.workflow.motes[0].mote.def.clone();
    let def_registry = InMemoryMoteDefRegistry::new();
    def_registry.register(shaper_def);
    let role_registry = InMemoryRoleRegistry::new();
    for r in ["reader", "summarizer"] {
        role_registry.register(
            RoleId(r.into()),
            Role {
                name: r.into(),
                version: 1,
                spec: warrant(),
                description: String::new(),
            },
        );
    }
    let materializer = Box::new(DefaultTopologyMaterializer::new(
        d.store.clone(),
        Arc::new(def_registry),
        Arc::new(role_registry),
        InheritFromShaperResolver,
    ));
    Projection::from_journal_with_materializer(&*d.journal, materializer).unwrap()
}

// ---------------------------------------------------------------------------

#[test]
fn model_drives_topology_children_materialize_and_run() {
    let dir = tempfile::tempdir().unwrap();
    let d = drive(dir.path(), &two_child_proposal(), LoopBudget::default());
    let outcome = d.result.as_ref().expect("the run completes");

    // shaper + 2 children, all committed.
    assert_eq!(outcome.total, 3, "shaper + 2 materialized children");
    assert_eq!(outcome.committed, 3, "all committed");

    // R49: a cold re-fold reproduces the same committed set + the children's
    // parent is the shaper — with NO backend (served from the committed fact).
    let cold = cold_fold(&d);
    assert_eq!(cold.committed_count(), 3);
    let shaper_id = d.workflow.shaper_id;
    assert_eq!(cold.state_of(&shaper_id), MoteState::Committed);
    let children: Vec<_> = cold
        .iter_motes()
        .map(|(id, _)| id)
        .filter(|id| *id != shaper_id)
        .collect();
    assert_eq!(children.len(), 2, "two children re-fold");
    for c in &children {
        assert_eq!(
            cold.parents_of(c)[0].0,
            shaper_id,
            "each child's parent is the shaper"
        );
    }
}

#[test]
fn cold_refold_is_identical_and_serves_the_committed_decision() {
    let dir = tempfile::tempdir().unwrap();
    let d = drive(dir.path(), &two_child_proposal(), LoopBudget::default());
    d.result.as_ref().expect("the run completes");

    // The live run called the backend (the eager plan + the children's dispatch).
    assert!(d.calls >= 1, "the model was consulted during the live run");

    // Two cold re-folds reproduce byte-identical identities (R49) — the
    // materializer decodes the committed decision fact, never re-running a model.
    let a: Vec<_> = cold_fold(&d).iter_motes().map(|(id, _)| id).collect();
    let b: Vec<_> = cold_fold(&d).iter_motes().map(|(id, _)| id).collect();
    assert_eq!(
        a, b,
        "cold re-folds are identical (R49, served-not-resampled)"
    );
    assert_eq!(a.len(), 3);
}

#[test]
fn think_preamble_is_stripped_before_decode() {
    let dir = tempfile::tempdir().unwrap();
    let mut reply = b"<think>I will spawn a reader and a summarizer.</think>\n".to_vec();
    reply.extend_from_slice(&two_child_proposal());
    let d = drive(dir.path(), &reply, LoopBudget::default());
    let outcome = d
        .result
        .as_ref()
        .expect("the run completes after the think strip");
    assert_eq!(
        outcome.committed, 3,
        "shaper + 2 children despite the preamble"
    );
}

#[test]
fn malformed_proposal_dead_letters_the_shaper() {
    let dir = tempfile::tempdir().unwrap();
    let d = drive(
        dir.path(),
        b"this is not a loop proposal",
        LoopBudget::default(),
    );
    let outcome = d
        .result
        .as_ref()
        .expect("a refused proposal is NOT a run error (PR-1)");

    // No children materialize; the shaper is a terminal Failed fact; run completes.
    assert_eq!(outcome.committed, 0, "nothing committed");
    assert_eq!(outcome.total, 1, "only the (dead-lettered) shaper");
    let cold = cold_fold(&d);
    assert_eq!(
        cold.state_of(&d.workflow.shaper_id),
        MoteState::Failed,
        "the shaper is dead-lettered, never blindly re-run"
    );
    assert_eq!(cold.committed_count(), 0);
}

#[test]
fn unknown_role_dead_letters_the_shaper() {
    let dir = tempfile::tempdir().unwrap();
    // "ghost" is not in the recipe allowlist → lower fails closed (UnknownRecipe).
    let reply =
        br#"{"loop_proposal":{"version":1,"next_steps":[{"role":"ghost","intent":"haunt"}]}}"#;
    let d = drive(dir.path(), reply, LoopBudget::default());
    let outcome = d
        .result
        .as_ref()
        .expect("a refused proposal completes the run");
    assert_eq!(outcome.committed, 0);
    assert_eq!(
        cold_fold(&d).state_of(&d.workflow.shaper_id),
        MoteState::Failed
    );
}

#[test]
fn fan_out_budget_dead_letters_an_over_cap_decision() {
    let dir = tempfile::tempdir().unwrap();
    // A 2-child proposal under a max_children = 1 budget → fail-closed dead-letter.
    let budget = LoopBudget {
        max_rounds: 4,
        max_children: 1,
    };
    let d = drive(dir.path(), &two_child_proposal(), budget);
    let outcome = d
        .result
        .as_ref()
        .expect("an over-budget proposal completes the run");
    assert_eq!(outcome.committed, 0, "the over-cap fan-out is refused");
    assert_eq!(
        cold_fold(&d).state_of(&d.workflow.shaper_id),
        MoteState::Failed
    );
}

#[test]
fn round_budget_is_enforced_at_the_provider() {
    // The cross-round budget is a provider concern (exercised across rounds by PR-3
    // re-plan). Drive it directly: a `max_rounds = 2` provider honours 2 `decide`s
    // then fails closed on the 3rd.
    use kx_model_harness::ModelTopologyProvider;
    use kx_projection::Projection as ColdProjection;
    use kx_runtime::TopologyProvider;

    let store = Arc::new(kx_content::InMemoryContentStore::new());
    let backend = Arc::new(StubBackend {
        reply: two_child_proposal(),
        calls: Arc::new(AtomicUsize::new(0)),
    });
    let registry: Arc<dyn ToolRegistry> = Arc::new(InMemoryToolRegistry::with_builtins());
    let provider = ModelTopologyProvider::new(
        backend,
        store,
        registry,
        recipes(),
        LoopBudget {
            max_rounds: 2,
            max_children: 8,
        },
    );
    let wf = loop_wf();
    let shaper = &wf.motes[0].mote;
    let snapshot = ColdProjection::new().snapshot();

    assert!(
        provider.decide(shaper, &warrant(), &snapshot).is_ok(),
        "round 1"
    );
    assert!(
        provider.decide(shaper, &warrant(), &snapshot).is_ok(),
        "round 2"
    );
    assert!(
        provider.decide(shaper, &warrant(), &snapshot).is_err(),
        "round 3 exceeds max_rounds → fail-closed"
    );
}
