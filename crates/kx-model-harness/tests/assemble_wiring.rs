//! D78 — `assemble()` is wired into the model dispatch path.
//!
//! These run WITHOUT the GGUF model (a recording stub `InferenceBackend` captures
//! the exact `InferenceInput` the model would see), so they gate in plain
//! `cargo test`. They prove the M5.1 DoD directly:
//!
//! - **(a)** the dispatch path calls `assemble`, and the resolved tool-menu +
//!   Data-edge parent bytes reach the model input;
//! - **(b)** the wired path is deterministic (assemble is pure);
//! - **(c)** no `source_ref` hash reaches the model window (only `bytes`);
//! - **(d)** a window overflow surfaces a typed error (a shaper-decision seam),
//!   never a panic.
//!
//! The leaf path (no Data parents, no tool grants ⇒ empty assembled context) is
//! covered by the existing A–J rows + `a0_guard` staying byte-identical.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::doc_markdown)]

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use kx_capability::{CapabilityBroker, LocalCapabilityBroker};
use kx_content::LocalFsContentStore;
use kx_executor::{LocalResourceManager, StandardCommitProtocol};
use kx_inference::{InferenceBackend, InferenceError, InferenceInput, InferenceOutput};
use kx_journal::SqliteJournal;
use kx_model_harness::{
    harness_warrant, prompt, workflows, BrokerObserver, ModelBroker, ModelExecutor,
};
use kx_mote::{
    EdgeMeta, EffectPattern, GraphPosition, InferenceParams, InputDataId, LogicRef, ModelId, Mote,
    MoteDef, MoteId, NdClass, ParentRef, PromptTemplateHash, ToolName, ToolVersion,
    MOTE_DEF_SCHEMA_VERSION,
};
use kx_projection::Projection;
use kx_runtime::config::Mode;
use kx_runtime::workflow::WorkflowMote;
use kx_runtime::{
    run_with_seams, DemoWorkflow, RunOutcome, RuntimeConfig, RuntimeError, SnapshotSink,
};
use kx_tool_registry::{InMemoryToolRegistry, ToolRegistry};
use kx_warrant::{ToolGrant, WarrantSpec};
use smallvec::SmallVec;

/// The instruction carried by the model Mote (identity-bearing, `config_subset`).
const INSTRUCTION: &str = "Summarize the input.";
/// The fs-read builtin's exact description (asserted to reach the window).
const FS_READ_DESC: &str =
    "Read bytes from a path declared in the warrant's fs_scope. Read-only; naturally idempotent.";

/// A stub `InferenceBackend` that records every prompt it is handed (so a test
/// can inspect exactly what the model would see) and returns a fixed completion.
struct RecordingBackend {
    inputs: Arc<Mutex<Vec<String>>>,
}

impl InferenceBackend for RecordingBackend {
    fn dispatch(
        &self,
        model_id: &ModelId,
        input: &InferenceInput,
        _params: &kx_mote::InferenceParams,
        _warrant: &WarrantSpec,
    ) -> Result<InferenceOutput, InferenceError> {
        let text = match input {
            InferenceInput::Text(s) => s.clone(),
            InferenceInput::Multimodal { text, .. } => text.clone(),
        };
        self.inputs.lock().unwrap().push(text);
        Ok(InferenceOutput {
            bytes: b"RECORDED_OUTPUT".to_vec(),
            output_tokens: 1,
            backend_name: "recording-stub",
            model_id: model_id.clone(),
            elapsed: Duration::from_millis(0),
        })
    }

    fn supports(&self, _model_id: &ModelId) -> bool {
        true
    }

    fn name(&self) -> &'static str {
        "recording-stub"
    }
}

fn model_id() -> ModelId {
    ModelId("stub-model:test:0".to_string())
}

#[allow(clippy::too_many_arguments)]
fn build_mote(
    seed: u8,
    model_id: &ModelId,
    prompt_text: Option<&str>,
    params: InferenceParams,
    parents: &[ParentRef],
) -> Mote {
    let mut config_subset = BTreeMap::new();
    if let Some(p) = prompt_text {
        prompt::put_prompt(&mut config_subset, p);
    }
    let def = MoteDef {
        critic_check: None,
        logic_ref: LogicRef::from_bytes([seed; 32]),
        model_id: model_id.clone(),
        prompt_template_hash: PromptTemplateHash::from_bytes([seed; 32]),
        tool_contract: BTreeMap::new(),
        nd_class: NdClass::Pure,
        config_subset,
        effect_pattern: EffectPattern::IdempotentByConstruction,
        critic_for: None,
        is_topology_shaper: false,
        inference_params: params,
        schema_version: MOTE_DEF_SCHEMA_VERSION,
    };
    Mote::new(
        def,
        InputDataId::from_bytes([seed; 32]),
        GraphPosition(vec![seed]),
        parents
            .iter()
            .copied()
            .collect::<SmallVec<[ParentRef; 4]>>(),
    )
}

/// A warrant that grants the `fs-read` builtin (so assemble emits a tool-menu
/// item) with `max_input_tokens` controlling the assemble byte budget.
fn warrant_granting_fs_read(model_id: &ModelId, max_input_tokens: u32) -> WarrantSpec {
    let mut w = harness_warrant(model_id, 48, 1000);
    w.model_route.max_input_tokens = max_input_tokens;
    w.tool_grants.insert(ToolGrant {
        tool_id: ToolName("fs-read".into()),
        tool_version: ToolVersion("1".into()),
    });
    w
}

/// A flat workflow: a non-model PURE parent `P` (commits deterministic bytes) →
/// a model Mote `M` (prompt + Data edge to `P` + a warrant granting fs-read).
/// `M`'s assembled context is therefore non-empty (parent bytes + tool menu).
fn workflow_with(m_warrant: &WarrantSpec, model_id: &ModelId) -> (DemoWorkflow, MoteId, MoteId) {
    let parent = build_mote(0x01, model_id, None, InferenceParams::default(), &[]);
    let child = build_mote(
        0x02,
        model_id,
        Some(INSTRUCTION),
        InferenceParams {
            max_output_tokens: 32,
            ..InferenceParams::default()
        },
        &[ParentRef {
            parent_id: parent.id,
            edge: EdgeMeta::data(),
        }],
    );
    let p_id = parent.id;
    let m_id = child.id;
    let cap = ToolName("kx-model".into());
    let sentinel = workflows::sentinel_shaper();
    let motes = vec![
        WorkflowMote {
            mote: parent,
            warrant: harness_warrant(model_id, 48, 1000),
            capability: cap.clone(),
        },
        WorkflowMote {
            mote: child,
            warrant: m_warrant.clone(),
            capability: cap,
        },
    ];
    (
        DemoWorkflow {
            motes,
            stc_crash_target: sentinel,
            vtc_crash_target: sentinel,
            shaper_id: sentinel,
        },
        p_id,
        m_id,
    )
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

/// Drive `workflow` through the real orchestrator with a recording stub backend.
/// Returns the run result + every prompt the backend was handed (in order).
fn drive(workflow: &DemoWorkflow, dir: &Path) -> (Result<RunOutcome, RuntimeError>, Vec<String>) {
    let config = config_for(dir);
    let store = Arc::new(LocalFsContentStore::open(&config.content_root).unwrap());
    let journal = Arc::new(SqliteJournal::open(&config.journal_path).unwrap());
    let inputs = Arc::new(Mutex::new(Vec::new()));
    let backend = Arc::new(RecordingBackend {
        inputs: inputs.clone(),
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
    // M5.2: an empty tool broker (these wiring tests assert the menu/parent bytes
    // reach the model — not tool selection), so the routing is inert here.
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
        // These wiring tests grant no tools (the tool arm is never entered), so the
        // instance_id is inert — an all-zero sentinel suffices.
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
        None, // capture_sink (D67) — off for this assemble-wiring test
    );
    let captured = inputs.lock().unwrap().clone();
    (result, captured)
}

#[test]
fn tool_menu_and_parent_bytes_reach_the_model_input() {
    let id = model_id();
    let m_warrant = warrant_granting_fs_read(&id, 8192);
    let (workflow, p_id, _m_id) = workflow_with(&m_warrant, &id);
    let dir = tempfile::tempdir().unwrap();
    let (result, inputs) = drive(&workflow, dir.path());

    assert!(result.unwrap().is_complete(), "both Motes committed");
    // Only the model Mote `M` hits the backend; the non-model parent does not.
    assert_eq!(inputs.len(), 1, "exactly one model dispatch");
    let input = &inputs[0];

    // (a) The instruction is present (ChatML user turn).
    assert!(input.contains(INSTRUCTION), "instruction reached the model");
    // (a) The Data-edge parent's committed bytes reached the window. The parent
    // is a non-model PURE Mote, whose deterministic body bytes carry this prefix.
    assert!(
        input.contains("kx-model-harness-pure:"),
        "parent result bytes reached the model input"
    );
    // (a) The resolved tool-menu reached the window: the granted fs-read tool's
    // label AND its description bytes.
    assert!(
        input.contains("tool.fs-read@1"),
        "tool-menu label reached the model input"
    );
    assert!(
        input.contains(FS_READ_DESC),
        "tool description bytes reached the model input"
    );

    // (c) No hash reaches the window: the parent item's `source_ref` (its
    // committed content ref) must NOT appear — only its bytes do.
    let config = config_for(dir.path());
    let journal = SqliteJournal::open(&config.journal_path).unwrap();
    let projection = Projection::from_journal(&journal).unwrap();
    let parent_ref = projection.result_ref_of(&p_id).expect("parent committed");
    assert!(
        !input.contains(&parent_ref.to_hex()),
        "no source_ref hash reaches the model window (D78 invariant)"
    );
}

#[test]
fn wired_path_is_deterministic() {
    // (b) assemble is pure: two independent drives of the same workflow hand the
    // model a byte-identical input.
    let id = model_id();
    let m_warrant = warrant_granting_fs_read(&id, 8192);
    let (workflow, _p, _m) = workflow_with(&m_warrant, &id);

    let dir_a = tempfile::tempdir().unwrap();
    let dir_b = tempfile::tempdir().unwrap();
    let (ra, inputs_a) = drive(&workflow, dir_a.path());
    let (rb, inputs_b) = drive(&workflow, dir_b.path());

    assert!(ra.unwrap().is_complete() && rb.unwrap().is_complete());
    assert_eq!(inputs_a.len(), 1);
    assert_eq!(inputs_b.len(), 1);
    assert_eq!(
        inputs_a[0], inputs_b[0],
        "the assembled model input is deterministic across runs"
    );
}

#[test]
fn window_overflow_is_a_typed_decision_not_a_panic() {
    // (d) A tiny budget (max_input_tokens=1 ⇒ 4-byte window) cannot hold the
    // parent bytes + tool description. assemble fails closed with
    // OverflowDecisionRequired ⇒ the executor returns a typed error ⇒ the run
    // returns Err. The process survives (this test returning proves no panic).
    let id = model_id();
    let m_warrant = warrant_granting_fs_read(&id, 1);
    let (workflow, _p, _m) = workflow_with(&m_warrant, &id);
    let dir = tempfile::tempdir().unwrap();
    let (result, _inputs) = drive(&workflow, dir.path());

    let err = result.expect_err("overflow must surface as a typed error, not a panic");
    let msg = err.to_string();
    assert!(
        msg.contains("exceeds window"),
        "the typed overflow decision is surfaced (got: {msg})"
    );
}
