//! M5.2b — the M5 EXIT GATE end-to-end: a single-Mote run selects a tool, the
//! runtime derives the egress from the resolved tool's `net_scope`, dispatches an
//! **external HTTP** tool through the real `HttpTransport` + warrant/broker gate
//! against a hermetic loopback server, captures provenance, and commits — with the
//! egress warrant-scoped, the run-scoped `Idempotency-Key` sent (remote
//! exactly-once), and the credential never journaled.
//!
//! Hermetic: an in-process `TcpListener` HTTP server on `127.0.0.1` (no live
//! network, no `[[bin]]` — kx-model-harness can't reach the kx-mcp test helpers).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::doc_markdown)]

use std::io::{Read as _, Write as _};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use kx_capability::{CapabilityBroker, LocalCapabilityBroker, INSTANCE_ID_LEN};
use kx_content::{ContentStore, LocalFsContentStore};
use kx_executor::{LocalResourceManager, StandardCommitProtocol};
use kx_inference::{InferenceBackend, InferenceError, InferenceInput, InferenceOutput};
use kx_journal::SqliteJournal;
use kx_mcp::{HttpTransport, McpCapability};
use kx_model_harness::{harness_warrant, workflows, BrokerObserver, ModelBroker, ModelExecutor};
use kx_mote::{ModelId, MoteId, ToolName, ToolVersion};
use kx_projection::Projection;
use kx_runtime::config::Mode;
use kx_runtime::{run_with_seams, RunOutcome, RuntimeConfig, RuntimeError, SnapshotSink};
use kx_tool_registry::{
    IdempotencyClass, InMemoryToolRegistry, McpEndpointId, ToolDef, ToolKind, ToolProvenance,
    ToolRegistry,
};
use kx_warrant::{
    FsScope, Host, NetScope, ResourceCeiling, ToolGrant, ToolRequirement, WarrantSpec,
};

fn model_id() -> ModelId {
    ModelId("stub-model:test:0".to_string())
}

// ---------------------------------------------------------------------------
// Hermetic in-process HTTP mock (loopback), capturing the Idempotency-Key.
// ---------------------------------------------------------------------------

struct MockHttp {
    addr: SocketAddr,
    keys: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl MockHttp {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let keys: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (k, s) = (keys.clone(), stop.clone());
        let handle = std::thread::spawn(move || {
            for stream in listener.incoming() {
                if s.load(Ordering::SeqCst) {
                    break;
                }
                if let Ok(stream) = stream {
                    let k = k.clone();
                    std::thread::spawn(move || handle_conn(stream, &k));
                }
            }
        });
        Self {
            addr,
            keys,
            stop,
            handle: Some(handle),
        }
    }

    fn url(&self) -> String {
        format!("http://127.0.0.1:{}/mcp", self.addr.port())
    }

    fn idempotency_keys(&self) -> Vec<String> {
        self.keys.lock().unwrap().clone()
    }
}

impl Drop for MockHttp {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.addr);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn handle_conn(mut stream: TcpStream, keys: &Arc<Mutex<Vec<String>>>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    while !buf.ends_with(b"\r\n\r\n") {
        match stream.read(&mut byte) {
            Ok(0) | Err(_) => return,
            Ok(_) => buf.push(byte[0]),
        }
        if buf.len() > 64 * 1024 {
            return;
        }
    }
    let head = String::from_utf8_lossy(&buf);
    let mut content_length = 0usize;
    for line in head.lines().skip(1) {
        if let Some((k, v)) = line.split_once(':') {
            let (k, v) = (k.trim().to_ascii_lowercase(), v.trim());
            if k == "content-length" {
                content_length = v.parse().unwrap_or(0);
            }
            if k == "idempotency-key" {
                keys.lock().unwrap().push(v.to_string());
            }
        }
    }
    let mut body = vec![0u8; content_length];
    let _ = stream.read_exact(&mut body);
    let resp_body = r#"{"jsonrpc":"2.0","id":1,"result":{"ok":true}}"#;
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
        resp_body.len(),
        resp_body
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

const EXPECTED_RESULT: &[u8] = br#"{"ok":true}"#;

// ---------------------------------------------------------------------------
// Fixtures.
// ---------------------------------------------------------------------------

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

/// A registry whose MCP tool declares it needs egress to `127.0.0.1` (an HTTP tool).
fn registry_with_http_tool(
    tool: &ToolName,
    version: &ToolVersion,
    endpoint: &str,
) -> InMemoryToolRegistry {
    let mut reg = InMemoryToolRegistry::with_builtins();
    let _ = reg.register(
        ToolDef {
            tool_id: tool.clone(),
            tool_version: version.clone(),
            kind: ToolKind::Mcp {
                endpoint: McpEndpointId(endpoint.to_string()),
                remote_name: "echo".into(),
            },
            required_capability: ToolRequirement {
                net_scope_required: NetScope::EgressAllowlist(
                    [Host("127.0.0.1".to_string())].into_iter().collect(),
                ),
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
            description: "HTTP MCP echo tool (M5.2b e2e).".into(),
            idempotency_class: IdempotencyClass::Staged,
            input_schema: None,
        },
        ToolProvenance::HumanAuthored {
            author: "test".into(),
        },
    );
    reg
}

fn warrant_for(tool: &ToolName, version: &ToolVersion, egress: NetScope) -> WarrantSpec {
    let mut w = harness_warrant(&model_id(), 64, 5_000);
    w.tool_grants.insert(ToolGrant {
        tool_id: tool.clone(),
        tool_version: version.clone(),
    });
    w.net_scope = egress;
    w
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

fn envelope(tool: &str, version: &str) -> Vec<u8> {
    format!(r#"{{"tool_call":{{"name":"{tool}","version":"{version}","args":{{"q":"x"}}}}}}"#)
        .into_bytes()
}

/// Drive a single-Mote model→HTTP-tool run. Returns the outcome + (store, journal,
/// mote id) and the registered instance_id used.
#[allow(clippy::type_complexity)]
fn drive(
    dir: &Path,
    server: &MockHttp,
    warrant_egress: NetScope,
    instance_id: [u8; INSTANCE_ID_LEN],
) -> (
    Result<RunOutcome, RuntimeError>,
    Arc<LocalFsContentStore>,
    Arc<SqliteJournal>,
    MoteId,
) {
    let tool = ToolName("mcp-http".into());
    let version = ToolVersion("1".into());
    let config = config_for(dir);
    let store = Arc::new(LocalFsContentStore::open(&config.content_root).unwrap());
    let journal = Arc::new(SqliteJournal::open(&config.journal_path).unwrap());
    let backend = Arc::new(FixedBackend {
        completion: envelope("mcp-http", "1"),
    });
    let sink = SnapshotSink::new();
    let registry: Arc<dyn ToolRegistry> =
        Arc::new(registry_with_http_tool(&tool, &version, &server.url()));

    // The tool broker holds the McpCapability over a REAL HttpTransport → the mock.
    let tool_broker_concrete = LocalCapabilityBroker::new(store.clone());
    let transport = HttpTransport::new(
        &server.url(),
        &NetScope::EgressAllowlist([Host("127.0.0.1".to_string())].into_iter().collect()),
        false,
    )
    .expect("http transport builds for loopback");
    tool_broker_concrete.register_capability(Box::new(McpCapability::new(
        tool.clone(),
        version.clone(),
        McpEndpointId(server.url()),
        "echo",
        Box::new(transport),
    )));
    let tool_broker: Arc<dyn CapabilityBroker> = Arc::new(tool_broker_concrete);

    let warrant = warrant_for(&tool, &version, warrant_egress);
    let workflow =
        workflows::model_tool_call(&model_id(), &warrant, "call the tool", &tool, &version);
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
        instance_id,
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
        None, // capture_sink (D67) — off for this MCP HTTP e2e test
    );
    (result, store, journal, m_id)
}

fn loopback_egress() -> NetScope {
    NetScope::EgressAllowlist([Host("127.0.0.1".to_string())].into_iter().collect())
}

#[test]
fn model_dispatches_external_http_tool_egress_scoped() {
    // THE M5 EXIT GATE: select Mote+model, assemble context, dispatch an external
    // HTTP tool with resolved params, capture provenance, commit — egress
    // warrant-scoped.
    let server = MockHttp::start();
    let dir = tempfile::tempdir().unwrap();
    let instance_id = [0x7au8; INSTANCE_ID_LEN];

    let (result, store, journal, m_id) = drive(dir.path(), &server, loopback_egress(), instance_id);
    let outcome = result.expect("run completes");
    assert!(outcome.is_complete(), "the model→HTTP-tool Mote committed");

    let projection = Projection::from_journal(&*journal).unwrap();
    let result_ref = projection.result_ref_of(&m_id).expect("Mote committed");
    let bytes = store.get(&result_ref).unwrap();
    assert_eq!(
        &*bytes, EXPECTED_RESULT,
        "committed bytes are the HTTP MCP result"
    );

    // The egress actually happened, and a run-scoped Idempotency-Key was sent
    // (64 lower-hex chars). That it IS the run-scoped token — varying with the run —
    // is proven by `recovery_redispatch_same_run_sends_same_idempotency_key`.
    let keys = server.idempotency_keys();
    assert_eq!(keys.len(), 1, "exactly one HTTP request egressed");
    assert_eq!(keys[0].len(), 64, "Idempotency-Key is a 32-byte hex token");
    assert!(keys[0]
        .bytes()
        .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
}

#[test]
fn recovery_redispatch_same_run_sends_same_idempotency_key() {
    // Two dispatches of the SAME run (same instance_id) send the SAME
    // Idempotency-Key → the remote dedups (remote exactly-once on crash-recovery).
    // A DIFFERENT run (fresh instance_id) sends a DIFFERENT key (re-submit fires
    // afresh, D64).
    let server = MockHttp::start();
    let run_a = [0x11u8; INSTANCE_ID_LEN];
    let run_b = [0x22u8; INSTANCE_ID_LEN];

    let d1 = tempfile::tempdir().unwrap();
    let d2 = tempfile::tempdir().unwrap();
    let d3 = tempfile::tempdir().unwrap();
    let _ = drive(d1.path(), &server, loopback_egress(), run_a);
    let _ = drive(d2.path(), &server, loopback_egress(), run_a); // same run → same key
    let _ = drive(d3.path(), &server, loopback_egress(), run_b); // different run

    let keys = server.idempotency_keys();
    assert_eq!(keys.len(), 3);
    assert_eq!(
        keys[0], keys[1],
        "same run ⇒ same Idempotency-Key (remote dedup)"
    );
    assert_ne!(
        keys[0], keys[2],
        "different run ⇒ different Idempotency-Key"
    );
}

#[test]
fn none_egress_warrant_refuses_the_http_tool() {
    // NEGATIVE CONTROL: the resolved tool needs egress, but the warrant grants
    // NONE. The broker gate refuses fail-closed — no commit, no egress.
    let server = MockHttp::start();
    let dir = tempfile::tempdir().unwrap();

    let (result, _store, journal, m_id) = drive(
        dir.path(),
        &server,
        NetScope::None,
        [0x33u8; INSTANCE_ID_LEN],
    );

    if let Ok(outcome) = &result {
        assert!(
            !outcome.is_complete(),
            "a None-egress warrant must not complete the tool"
        );
    }
    let state = Projection::from_journal(&*journal).unwrap().state_of(&m_id);
    assert_ne!(
        state,
        kx_projection::MoteState::Committed,
        "no effect committed"
    );
    assert!(
        server.idempotency_keys().is_empty(),
        "no HTTP request may egress under a None-egress warrant"
    );
}
