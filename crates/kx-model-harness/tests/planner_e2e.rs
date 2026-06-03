//! M6 — the agentic loop end-to-end: prompt → committed plan → decode → lower →
//! compile → registered Mote DAG → run → result.
//!
//! Runs WITHOUT the GGUF model (a stub `InferenceBackend` returns a fixed plan
//! envelope for the planner step and a fixed completion for the worker steps), so
//! it gates in plain `cargo test`. Proves the M6 DoD directly:
//!
//! - **plan-committed-as-a-fact (D74):** the planner Mote's committed `result_ref`
//!   IS the plan bytes; the runtime reads them back and compiles them.
//! - **replays, never re-runs (D74):** re-driving the same journal serves the
//!   committed plan and dispatches the model ZERO times.
//! - **re-run resamples (D74):** two fresh runs with different model output commit
//!   different plans (the planner is ROND), each compiling deterministically.
//! - **role-centric authority (D75):** every produced Mote's warrant is
//!   `intersect(parent, role)` — no widening.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::doc_markdown)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use kx_capability::{CapabilityBroker, LocalCapabilityBroker};
use kx_content::{ContentStore, LocalFsContentStore};
use kx_executor::{LocalResourceManager, StandardCommitProtocol};
use kx_inference::{InferenceBackend, InferenceError, InferenceInput, InferenceOutput};
use kx_journal::SqliteJournal;
use kx_model_harness::{workflows, BrokerObserver, ModelBroker, ModelExecutor};
use kx_mote::{
    EffectPattern, InferenceParams, LogicRef, ModelId, NdClass, PromptTemplateHash, RoleId,
    ToolName,
};
use kx_planner::{
    compile_plan, decode_plan, max_plan_bytes, seed_from_plan_bytes, InMemoryRoleRecipes,
    RoleRecipe, PLAN_PROMPT_KEY,
};
use kx_projection::{fold_run_metadata, Projection};
use kx_runtime::config::Mode;
use kx_runtime::{
    run_with_seams, DemoWorkflow, RunOutcome, RuntimeConfig, RuntimeError, SnapshotSink,
};
use kx_tool_registry::{InMemoryToolRegistry, ToolRegistry};
use kx_warrant::{
    ExecutorClass, FsScope, InMemoryRoleRegistry, ModelRoute, MoteClass, NetScope, ResourceCeiling,
    Role, WarrantSpec,
};

/// The planning instruction the planner Mote carries.
const PLANNING_PROMPT: &str = "Plan: read the input, then summarize it.";

/// A fixed, well-formed plan the stub "model" emits: reader → summarizer (Data edge).
const PLAN_JSON: &str = r#"{"plan":{"version":1,"steps":[{"role":"reader","intent":"read the input"},{"role":"summarizer","intent":"summarize it"}],"edges":[{"parent":0,"child":1}]}}"#;

/// A *different* valid plan (a single reader step) — used to prove ROND resampling.
const PLAN_JSON_ALT: &str =
    r#"{"plan":{"version":1,"steps":[{"role":"reader","intent":"just read"}]}}"#;

fn model_id() -> ModelId {
    ModelId("stub-model:test:0".to_string())
}

/// A stub backend that returns a fixed `reply` for every dispatch and counts how
/// many times it was called (so a test can prove "served, never re-run").
struct StubBackend {
    reply: Vec<u8>,
    calls: Arc<AtomicUsize>,
    inputs: Arc<Mutex<Vec<String>>>,
}

impl InferenceBackend for StubBackend {
    fn dispatch(
        &self,
        model_id: &ModelId,
        input: &InferenceInput,
        _params: &InferenceParams,
        _warrant: &WarrantSpec,
    ) -> Result<InferenceOutput, InferenceError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let text = match input {
            InferenceInput::Text(s) => s.clone(),
            InferenceInput::Multimodal { text, .. } => text.clone(),
        };
        self.inputs.lock().unwrap().push(text);
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

/// A permissive parent warrant granting no tools (the planner's output is the
/// plan, never a tool call), routed to `model_id`.
fn parent_warrant(model_id: &ModelId) -> WarrantSpec {
    WarrantSpec {
        mote_class: MoteClass::WorldMutating,
        nd_class: MoteClass::WorldMutating,
        fs_scope: FsScope::empty(),
        net_scope: NetScope::None,
        syscall_profile_ref: kx_content::ContentRef::from_bytes([0; 32]),
        tool_grants: BTreeSet::new(),
        model_route: ModelRoute {
            model_id: model_id.clone(),
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

/// Register the `reader` + `summarizer` roles (warrant templates within the
/// parent) and their vetted recipes (PURE model steps, no tools).
fn registries(
    model_id: &ModelId,
    parent: &WarrantSpec,
) -> (InMemoryRoleRegistry, InMemoryRoleRecipes) {
    let roles = InMemoryRoleRegistry::new();
    let recipes = InMemoryRoleRecipes::new();
    for (i, name) in ["reader", "summarizer"].iter().enumerate() {
        let tag = u8::try_from(i).unwrap();
        // Role warrant = a within-parent clone (no tools, same syscall profile).
        let mut spec = parent.clone();
        spec.mote_class = MoteClass::Pure;
        spec.nd_class = MoteClass::Pure;
        roles.register(
            RoleId((*name).into()),
            Role {
                name: (*name).into(),
                version: 1,
                spec,
                description: String::new(),
            },
        );
        recipes.register(
            RoleId((*name).into()),
            RoleRecipe {
                logic_ref: LogicRef::from_bytes([0x70 + tag; 32]),
                model_id: model_id.clone(),
                prompt_template_hash: PromptTemplateHash::from_bytes([0x80 + tag; 32]),
                tool_contract: BTreeMap::new(),
                capability: ToolName("kx-model".into()),
                nd_class: NdClass::Pure,
                effect_pattern: EffectPattern::IdempotentByConstruction,
                inference_params: InferenceParams::default(),
                deterministic_check: None,
            },
        );
    }
    (roles, recipes)
}

fn config_for(dir: &Path) -> RuntimeConfig {
    RuntimeConfig {
        journal_path: dir.join("j.sqlite"),
        content_root: dir.join("c"),
        mode: Mode::Run,
        crash_at: None,
        checkpoint_every: None,
    }
}

/// Drive `workflow` through the real orchestrator with a stub backend returning
/// `reply`. Returns the run result + the dispatch count.
fn drive(
    workflow: &DemoWorkflow,
    dir: &Path,
    reply: &[u8],
) -> (Result<RunOutcome, RuntimeError>, usize) {
    let config = config_for(dir);
    let store = Arc::new(LocalFsContentStore::open(&config.content_root).unwrap());
    let journal = Arc::new(SqliteJournal::open(&config.journal_path).unwrap());
    let calls = Arc::new(AtomicUsize::new(0));
    let backend = Arc::new(StubBackend {
        reply: reply.to_vec(),
        calls: calls.clone(),
        inputs: Arc::new(Mutex::new(Vec::new())),
    });
    let sink = SnapshotSink::new();
    let registry: Arc<dyn ToolRegistry> = Arc::new(InMemoryToolRegistry::with_builtins());
    let executor = ModelExecutor::new(
        backend.clone(),
        store.clone(),
        sink.clone(),
        registry.clone(),
    );
    let observer = Arc::new(BrokerObserver::default());
    let tool_broker: Arc<dyn CapabilityBroker> =
        Arc::new(LocalCapabilityBroker::new(store.clone()));
    let broker = Arc::new(ModelBroker::new(
        backend,
        store.clone(),
        None,
        Some(workflow.stc_crash_target),
        observer,
        sink.clone(),
        registry,
        tool_broker,
        [0u8; kx_capability::INSTANCE_ID_LEN],
    ));
    let protocol = StandardCommitProtocol::new(store.clone(), journal.clone(), broker);
    let rm = LocalResourceManager::dev_defaults();
    let result = run_with_seams(
        &config,
        workflow,
        store,
        journal,
        &rm,
        &executor,
        &protocol,
        None,
        Some(&sink),
        None,
    );
    (result, calls.load(Ordering::SeqCst))
}

/// Read the planner Mote's committed plan bytes back from the journal/store.
fn read_committed_plan(dir: &Path, planner_id: kx_mote::MoteId) -> Vec<u8> {
    let config = config_for(dir);
    let store = LocalFsContentStore::open(&config.content_root).unwrap();
    let journal = SqliteJournal::open(&config.journal_path).unwrap();
    let projection = Projection::from_journal(&journal).unwrap();
    let plan_ref = projection
        .result_ref_of(&planner_id)
        .expect("planner committed its plan");
    store.get(&plan_ref).expect("plan bytes in store").to_vec()
}

#[test]
fn plan_commits_as_a_fact_compiles_and_runs() {
    let id = model_id();
    let warrant = parent_warrant(&id);
    let (roles, recipes) = registries(&id, &warrant);

    // (1) Run the planner Mote — its output (the plan) commits as its result_ref.
    let planner_wf = workflows::planner_mote(&id, &warrant, PLANNING_PROMPT);
    let planner_id = planner_wf.motes[0].mote.id;
    let plan_dir = tempfile::tempdir().unwrap();
    let (r1, calls1) = drive(&planner_wf, plan_dir.path(), PLAN_JSON.as_bytes());
    assert!(r1.unwrap().is_complete(), "planner committed");
    assert_eq!(calls1, 1, "the planner dispatched the model exactly once");

    // (2) Read the plan back as a FACT and decode + compile it.
    let plan_bytes = read_committed_plan(plan_dir.path(), planner_id);
    assert_eq!(
        plan_bytes.as_slice(),
        PLAN_JSON.as_bytes(),
        "committed plan == model output"
    );
    let plan = decode_plan(&plan_bytes, max_plan_bytes(&warrant)).expect("decodes");
    let seed = seed_from_plan_bytes(&plan_bytes);
    let compiled = compile_plan(&plan, seed, &warrant, &roles, &recipes).expect("compiles");
    assert_eq!(compiled.motes.len(), 2, "reader + summarizer");
    // D75: every produced warrant ⊆ parent (no widening).
    for m in &compiled.motes {
        assert!(m.warrant.tool_grants.is_subset(&warrant.tool_grants));
    }
    // The intent reached identity-bearing config under the prompt key.
    let reader_cfg = &compiled.motes[0].mote.def.config_subset;
    assert!(
        reader_cfg.contains_key(&kx_mote::ConfigKey(PLAN_PROMPT_KEY.to_string())),
        "the intent is carried under the prompt key"
    );

    // (3) Run the planner-produced DAG — no new execution mechanism.
    let dag = workflows::from_compiled(&compiled);
    let run_dir = tempfile::tempdir().unwrap();
    let (r2, calls2) = drive(&dag, run_dir.path(), b"ok");
    assert!(
        r2.unwrap().is_complete(),
        "the compiled DAG ran to completion"
    );
    assert_eq!(calls2, 2, "each model worker dispatched once");

    // (4) M6.2 — fold the run's journal into planner-ready metadata (D78). The
    // observability the planner would propose its NEXT round over: 2 committed
    // workers + their distinct recipe fingerprints.
    let journal = SqliteJournal::open(&config_for(run_dir.path()).journal_path).unwrap();
    let md = fold_run_metadata(&journal).unwrap();
    assert_eq!(md.committed, 2, "metadata fold sees both committed workers");
    assert_eq!(
        md.recipe_fingerprints.len(),
        2,
        "two distinct worker recipes"
    );
    let summary = String::from_utf8(md.summary_bytes()).unwrap();
    assert!(
        summary.contains("committed=2"),
        "summary is planner-readable: {summary}"
    );
}

#[test]
fn replay_serves_the_committed_plan_never_re_runs_the_model() {
    let id = model_id();
    let warrant = parent_warrant(&id);
    let planner_wf = workflows::planner_mote(&id, &warrant, PLANNING_PROMPT);
    let planner_id = planner_wf.motes[0].mote.id;
    let dir = tempfile::tempdir().unwrap();

    // First run commits the plan.
    let (r1, calls1) = drive(&planner_wf, dir.path(), PLAN_JSON.as_bytes());
    assert!(r1.unwrap().is_complete());
    assert_eq!(calls1, 1);
    let first = read_committed_plan(dir.path(), planner_id);

    // Re-drive over the SAME journal with a tripwire backend that would emit
    // DIFFERENT bytes if called. The committed plan is served, the model is NOT
    // re-run — so the tripwire fires zero times and the plan is byte-identical.
    let (r2, calls2) = drive(&planner_wf, dir.path(), b"DIFFERENT-SHOULD-NOT-COMMIT");
    assert!(r2.unwrap().is_complete());
    assert_eq!(
        calls2, 0,
        "replay served the committed plan; the model never re-ran"
    );
    let second = read_committed_plan(dir.path(), planner_id);
    assert_eq!(
        first, second,
        "the committed plan FACT is unchanged on replay"
    );
    assert_eq!(first.as_slice(), PLAN_JSON.as_bytes());
}

#[test]
fn re_run_resamples_distinct_plans_each_compiling_deterministically() {
    let id = model_id();
    let warrant = parent_warrant(&id);
    let (roles, recipes) = registries(&id, &warrant);
    let planner_wf = workflows::planner_mote(&id, &warrant, PLANNING_PROMPT);
    let planner_id = planner_wf.motes[0].mote.id;

    // Two fresh runs whose ROND planner "resamples" different plans.
    let dir_a = tempfile::tempdir().unwrap();
    let dir_b = tempfile::tempdir().unwrap();
    drive(&planner_wf, dir_a.path(), PLAN_JSON.as_bytes())
        .0
        .unwrap();
    drive(&planner_wf, dir_b.path(), PLAN_JSON_ALT.as_bytes())
        .0
        .unwrap();
    let a = read_committed_plan(dir_a.path(), planner_id);
    let b = read_committed_plan(dir_b.path(), planner_id);
    assert_ne!(a, b, "ROND: distinct samples commit distinct plans (D74)");

    // Each committed plan re-compiles deterministically (pure lowering).
    for bytes in [&a, &b] {
        let plan = decode_plan(bytes, max_plan_bytes(&warrant)).unwrap();
        let seed = seed_from_plan_bytes(bytes);
        let c1 = compile_plan(&plan, seed, &warrant, &roles, &recipes).unwrap();
        let c2 = compile_plan(&plan, seed, &warrant, &roles, &recipes).unwrap();
        let ids1: Vec<_> = c1.motes.iter().map(|m| m.mote.id).collect();
        let ids2: Vec<_> = c2.motes.iter().map(|m| m.mote.id).collect();
        assert_eq!(
            ids1, ids2,
            "re-compile of a committed plan is deterministic"
        );
    }
}
