//! M5.2 — the FIRST model-driven tool step, end-to-end through the real
//! `run_with_seams` orchestrator, deterministically (a stub backend stands in for
//! the GGUF). Proves:
//!
//! - **routing:** a model that emits a tool-call envelope → fail-closed decode →
//!   dispatch through the warrant/broker gate → `McpCapability` → committed result;
//! - **determinism/dedup:** two independent runs commit the byte-identical result
//!   ref (a recovery re-dispatch stages identical bytes ⇒ exactly-once rides the
//!   unchanged StageThenCommit machinery proven by row-G `kill_and_replay`);
//! - **fail-closed:** a malformed proposal never fires an effect (nothing commits).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::doc_markdown)]

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use kx_capability::{CapabilityBroker, LocalCapabilityBroker};
use kx_content::{ContentStore, LocalFsContentStore};
use kx_executor::{LocalResourceManager, StandardCommitProtocol};
use kx_inference::{InferenceBackend, InferenceError, InferenceInput, InferenceOutput};
use kx_journal::SqliteJournal;
use kx_mcp::{McpCapability, McpTransport, TransportError};
use kx_model_harness::{harness_warrant, workflows, BrokerObserver, ModelBroker, ModelExecutor};
use kx_mote::{ModelId, MoteId, ToolName, ToolVersion};
use kx_projection::{MoteState, Projection};
use kx_runtime::config::Mode;
use kx_runtime::{run_with_seams, RunOutcome, RuntimeConfig, RuntimeError, SnapshotSink};
use kx_tool_registry::{
    IdempotencyClass, InMemoryToolRegistry, McpEndpointId, ToolDef, ToolKind, ToolProvenance,
    ToolRegistry,
};
use kx_warrant::{FsScope, NetScope, ResourceCeiling, ToolGrant, ToolRequirement, WarrantSpec};

fn model_id() -> ModelId {
    ModelId("stub-model:test:0".to_string())
}

/// A registry carrying the builtins PLUS the MCP tool `(tool, version)` so the
/// M5.1 tool-menu assembler can resolve it (the model selects it from that menu).
fn registry_with_mcp(tool: &ToolName, version: &ToolVersion) -> InMemoryToolRegistry {
    let mut reg = InMemoryToolRegistry::with_builtins();
    let _ = reg.register(
        ToolDef {
            tool_id: tool.clone(),
            tool_version: version.clone(),
            kind: ToolKind::Mcp {
                endpoint: McpEndpointId("inproc://const".into()),
                remote_name: "echo".into(),
            },
            required_capability: ToolRequirement {
                net_scope_required: NetScope::None, // in-proc/stdio: no egress
                fs_scope_required: FsScope::empty(),
                syscall_profile_ref: kx_content::ContentRef::from_bytes([0; 32]),
                min_resource_ceiling: ResourceCeiling {
                    cpu_milli: 0,
                    mem_bytes: 0,
                    wall_clock_ms: 0,
                    fd_count: 0,
                    disk_bytes: 0,
                },
            },
            description: "MCP echo tool (M5.2 test).".into(),
            idempotency_class: IdempotencyClass::Staged,
            input_schema: None,
        },
        ToolProvenance::HumanAuthored {
            author: "test".into(),
        },
    );
    reg
}

/// A stub backend that returns a fixed completion regardless of input — standing in
/// for a model that decided to call a tool.
struct FixedBackend {
    completion: Vec<u8>,
}

impl InferenceBackend for FixedBackend {
    fn dispatch(
        &self,
        model_id: &ModelId,
        _input: &InferenceInput,
        _params: &kx_mote::InferenceParams,
        _warrant: &WarrantSpec,
    ) -> Result<InferenceOutput, InferenceError> {
        Ok(InferenceOutput {
            bytes: self.completion.clone(),
            output_tokens: 1,
            backend_name: "fixed-stub",
            model_id: model_id.clone(),
            elapsed: Duration::from_millis(0),
        })
    }
    fn supports(&self, _model_id: &ModelId) -> bool {
        true
    }
    fn name(&self) -> &'static str {
        "fixed-stub"
    }
}

/// An in-process MCP transport returning a constant `tools/call` result — no
/// subprocess (the real `StdioTransport` is exercised by the kx-mcp crate tests).
struct ConstResultTransport;

impl McpTransport for ConstResultTransport {
    fn round_trip(
        &self,
        _request: &[u8],
        _max: usize,
        _ms: u64,
        _idempotency_key: Option<&[u8; 32]>,
    ) -> Result<Vec<u8>, TransportError> {
        Ok(br#"{"jsonrpc":"2.0","id":1,"result":{"ok":true}}"#.to_vec())
    }
}

/// The decoded MCP result object the transport above yields (what gets committed).
const EXPECTED_RESULT: &[u8] = br#"{"ok":true}"#;

fn warrant_granting(tool: &ToolName, version: &ToolVersion) -> WarrantSpec {
    let mut w = harness_warrant(&model_id(), 64, 5_000);
    w.tool_grants.insert(ToolGrant {
        tool_id: tool.clone(),
        tool_version: version.clone(),
    });
    w
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

/// Drive a single-Mote `model_tool_call` workflow with a stub backend whose output
/// is `completion`, routing tool calls to an `McpCapability` over the const
/// transport. Returns the run result + the (store, journal) for inspection.
fn drive(
    dir: &Path,
    tool: &ToolName,
    version: &ToolVersion,
    completion: &[u8],
) -> (
    Result<RunOutcome, RuntimeError>,
    Arc<LocalFsContentStore>,
    Arc<SqliteJournal>,
    MoteId,
) {
    let config = config_for(dir);
    let store = Arc::new(LocalFsContentStore::open(&config.content_root).unwrap());
    let journal = Arc::new(SqliteJournal::open(&config.journal_path).unwrap());
    let backend = Arc::new(FixedBackend {
        completion: completion.to_vec(),
    });
    let sink = SnapshotSink::new();
    let registry: Arc<dyn ToolRegistry> = Arc::new(registry_with_mcp(tool, version));

    // The tool broker holds the concrete McpCapability under the granted name.
    let tool_broker_concrete = LocalCapabilityBroker::new(store.clone());
    tool_broker_concrete.register_capability(Box::new(McpCapability::new(
        tool.clone(),
        version.clone(),
        McpEndpointId("inproc://const".into()),
        "echo",
        Box::new(ConstResultTransport),
    )));
    let tool_broker: Arc<dyn CapabilityBroker> = Arc::new(tool_broker_concrete);

    let warrant = warrant_granting(tool, version);
    let workflow =
        workflows::model_tool_call(&model_id(), &warrant, "call the tool", tool, version);
    let m_id = workflow.stc_crash_target;

    let executor = ModelExecutor::new(
        backend.clone(),
        store.clone(),
        sink.clone(),
        registry.clone(),
    );
    let broker = Arc::new(ModelBroker::new(
        backend,
        store.clone(),
        None,
        Some(workflow.stc_crash_target),
        Arc::new(BrokerObserver::default()),
        sink.clone(),
        registry,
        tool_broker,
        // A fixed registered-run instance_id (D64): both runs in the determinism
        // test use the same id, so the run-scoped token (and thus the remote
        // Idempotency-Key) is stable — the staged ref is content-addressed anyway.
        [0x5au8; kx_capability::INSTANCE_ID_LEN],
    ));
    let protocol = StandardCommitProtocol::new(store.clone(), journal.clone(), broker);
    let rm = LocalResourceManager::dev_defaults();
    let result = run_with_seams(
        &config,
        &workflow,
        store.clone(),
        journal.clone(),
        &rm,
        &executor,
        &protocol,
        None,
        Some(&sink),
        None, // capture_sink (D67) — off for this MCP model-driven test
        None, // audit_sink (R4) — off for this MCP model-driven test
        None, // failure_policy (PR-1) — legacy abort-on-failure
    );
    (result, store, journal, m_id)
}

/// A well-formed tool-call envelope naming the granted tool.
fn envelope(tool: &str, version: &str) -> Vec<u8> {
    format!(r#"{{"tool_call":{{"name":"{tool}","version":"{version}","args":{{"q":"x"}}}}}}"#)
        .into_bytes()
}

#[test]
fn model_selects_tool_and_runtime_dispatches_through_mcp() {
    let tool = ToolName("mcp-echo".into());
    let version = ToolVersion("1".into());
    let dir = tempfile::tempdir().unwrap();

    let (result, store, journal, m_id) =
        drive(dir.path(), &tool, &version, &envelope("mcp-echo", "1"));

    let outcome = result.expect("run completes");
    assert!(
        outcome.is_complete(),
        "the single model-tool Mote committed"
    );

    // The committed result is the MCP capability's output (routed through the gate).
    let projection = Projection::from_journal(&*journal).unwrap();
    let result_ref = projection.result_ref_of(&m_id).expect("Mote committed");
    let bytes = store.get(&result_ref).unwrap();
    assert_eq!(
        &*bytes, EXPECTED_RESULT,
        "committed bytes are the MCP result"
    );
}

#[test]
fn model_driven_dispatch_is_deterministic_across_runs() {
    let tool = ToolName("mcp-echo".into());
    let version = ToolVersion("1".into());

    let dir_a = tempfile::tempdir().unwrap();
    let dir_b = tempfile::tempdir().unwrap();
    let (ra, sa, ja, ma) = drive(dir_a.path(), &tool, &version, &envelope("mcp-echo", "1"));
    let (rb, sb, jb, mb) = drive(dir_b.path(), &tool, &version, &envelope("mcp-echo", "1"));
    assert!(ra.unwrap().is_complete() && rb.unwrap().is_complete());

    // Same args ⇒ byte-identical staged bytes ⇒ identical content ref. This is the
    // property that makes a recovery re-dispatch exactly-once (content-addressed
    // dedup), riding the unchanged StageThenCommit machinery.
    let ref_a = Projection::from_journal(&*ja)
        .unwrap()
        .result_ref_of(&ma)
        .unwrap();
    let ref_b = Projection::from_journal(&*jb)
        .unwrap()
        .result_ref_of(&mb)
        .unwrap();
    assert_eq!(
        ref_a, ref_b,
        "the MCP dispatch is deterministic across runs"
    );
    assert_eq!(&*sa.get(&ref_a).unwrap(), EXPECTED_RESULT);
    assert_eq!(&*sb.get(&ref_b).unwrap(), EXPECTED_RESULT);
}

#[test]
fn malformed_proposal_fires_no_effect() {
    let tool = ToolName("mcp-echo".into());
    let version = ToolVersion("1".into());
    let dir = tempfile::tempdir().unwrap();

    // A truncated envelope: the model "committed" to a call but garbled it. The
    // decode is fail-closed → the effect NEVER fires → the Mote does not commit.
    let garbled = br#"{"tool_call":{"name":"mcp-echo","version":"#.to_vec();
    let (result, _store, journal, m_id) = drive(dir.path(), &tool, &version, &garbled);

    // The run does not complete the Mote (it errored or stalled fail-closed).
    if let Ok(outcome) = &result {
        assert!(
            !outcome.is_complete(),
            "a malformed proposal must not complete the run"
        );
    }
    let state = Projection::from_journal(&*journal).unwrap().state_of(&m_id);
    assert_ne!(
        state,
        MoteState::Committed,
        "no effect committed on a malformed proposal"
    );
}

#[test]
fn ungranted_proposal_fires_no_effect() {
    let granted = ToolName("mcp-echo".into());
    let version = ToolVersion("1".into());
    let dir = tempfile::tempdir().unwrap();

    // The model names a DIFFERENT tool than the one granted/declared → ungranted →
    // refused fail-closed; no effect commits. (The Mote's contract/grant is mcp-echo.)
    let (result, _store, journal, m_id) =
        drive(dir.path(), &granted, &version, &envelope("mcp-danger", "1"));

    if let Ok(outcome) = &result {
        assert!(
            !outcome.is_complete(),
            "an ungranted proposal must not complete the run"
        );
    }
    let state = Projection::from_journal(&*journal).unwrap().state_of(&m_id);
    assert_ne!(
        state,
        MoteState::Committed,
        "no effect committed on an ungranted proposal"
    );
}

#[test]
fn oversize_proposal_fires_no_effect() {
    let tool = ToolName("mcp-echo".into());
    let version = ToolVersion("1".into());
    let dir = tempfile::tempdir().unwrap();

    // The warrant grants 64 max_output_tokens ⇒ max_args_bytes = 256. Propose args
    // well beyond that — the IMP-16 cap refuses fail-closed; no effect commits.
    let big = "x".repeat(400);
    let env =
        format!(r#"{{"tool_call":{{"name":"mcp-echo","version":"1","args":{{"q":"{big}"}}}}}}"#)
            .into_bytes();
    let (result, _store, journal, m_id) = drive(dir.path(), &tool, &version, &env);

    if let Ok(outcome) = &result {
        assert!(
            !outcome.is_complete(),
            "an oversize proposal must not complete the run"
        );
    }
    let state = Projection::from_journal(&*journal).unwrap().state_of(&m_id);
    assert_ne!(
        state,
        MoteState::Committed,
        "no effect committed on an oversize proposal"
    );
}
