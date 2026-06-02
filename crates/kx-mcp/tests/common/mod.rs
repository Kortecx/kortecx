//! Shared fixtures for the kx-mcp integration tests: a Mote that declares the MCP
//! tool in its `tool_contract`, a warrant that grants it (net_scope = None, fs
//! empty — the stdio transport needs no egress), and an `EffectRequest` builder.

#![allow(
    dead_code,
    unreachable_pub,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::doc_markdown
)]

use std::collections::{BTreeMap, BTreeSet};

use kx_capability::EffectRequest;
use kx_mcp::{McpCapability, StdioTransport};
use kx_mote::{
    EffectPattern, GraphPosition, InferenceParams, InputDataId, LogicRef, ModelId, Mote, MoteDef,
    NdClass, PromptTemplateHash, ToolName, ToolVersion, MOTE_DEF_SCHEMA_VERSION,
};
use kx_tool_registry::McpEndpointId;
use kx_warrant::{
    ExecutorClass, FsScope, ModelRoute, MoteClass, NetScope, ResourceCeiling, ToolGrant,
    WarrantSpec,
};

/// An `McpCapability` over the bundled stdio echo server (default mode).
#[must_use]
pub fn echo_capability(name: ToolName, version: ToolVersion) -> McpCapability {
    McpCapability::new(
        name,
        version,
        McpEndpointId("stdio://mock".into()),
        "echo",
        Box::new(StdioTransport::new(MOCK_SERVER)),
    )
}

/// Absolute path to the bundled test stdio MCP server (set by Cargo for this crate's
/// integration tests because the server is one of this crate's `[[bin]]` targets).
pub const MOCK_SERVER: &str = env!("CARGO_BIN_EXE_kx-mcp-mock-stdio");

/// The MCP tool the fixtures use: `mcp-echo@1`.
#[must_use]
pub fn tool() -> (ToolName, ToolVersion) {
    (ToolName("mcp-echo".into()), ToolVersion("1".into()))
}

/// A WorldMutating `StageThenCommit` Mote that declares `(tool_name, tool_version)`
/// in its `tool_contract` (so `LocalCapabilityBroker::precheck` admits the call).
#[must_use]
pub fn sample_mote(tool_name: &ToolName, tool_version: &ToolVersion) -> Mote {
    let mut tool_contract = BTreeMap::new();
    tool_contract.insert(tool_name.clone(), tool_version.clone());
    let def = MoteDef {
        critic_check: None,
        logic_ref: LogicRef::from_bytes([7; 32]),
        model_id: ModelId("m".into()),
        prompt_template_hash: PromptTemplateHash::from_bytes([7; 32]),
        tool_contract,
        nd_class: NdClass::WorldMutating,
        config_subset: BTreeMap::new(),
        effect_pattern: EffectPattern::StageThenCommit,
        critic_for: None,
        is_topology_shaper: false,
        inference_params: InferenceParams::default(),
        schema_version: MOTE_DEF_SCHEMA_VERSION,
    };
    Mote::new(
        def,
        InputDataId::from_bytes([7; 32]),
        GraphPosition(vec![7]),
        smallvec::SmallVec::new(),
    )
}

/// A warrant granting exactly `(tool_name, tool_version)`, no egress, no fs.
#[must_use]
pub fn warrant_granting(tool_name: &ToolName, tool_version: &ToolVersion) -> WarrantSpec {
    let mut tool_grants = BTreeSet::new();
    tool_grants.insert(ToolGrant {
        tool_id: tool_name.clone(),
        tool_version: tool_version.clone(),
    });
    WarrantSpec {
        mote_class: MoteClass::WorldMutating,
        nd_class: MoteClass::WorldMutating,
        fs_scope: FsScope::empty(),
        net_scope: NetScope::None,
        syscall_profile_ref: kx_content::ContentRef::from_bytes([0; 32]),
        tool_grants,
        model_route: ModelRoute {
            model_id: ModelId("m".into()),
            max_input_tokens: 1024,
            max_output_tokens: 256,
            max_calls: 8,
        },
        resource_ceiling: ResourceCeiling {
            cpu_milli: 0,
            mem_bytes: 0,
            wall_clock_ms: 5_000,
            fd_count: 0,
            disk_bytes: 0,
        },
        environment_ref: None,
        executor_class: ExecutorClass::Bwrap,
    }
}

/// An `EffectRequest` carrying `args_json` (the tool arguments) under
/// `StageThenCommit` with no egress / fs.
#[must_use]
pub fn effect(args_json: &str) -> EffectRequest {
    EffectRequest {
        payload: args_json.as_bytes().to_vec(),
        pattern: EffectPattern::StageThenCommit,
        idempotency_key: None,
        net_scope: NetScope::None,
        fs_scope: FsScope::empty(),
    }
}

// ---------------------------------------------------------------------------
// M5.2b — HTTP transport test support: a hermetic in-process TcpListener mock
// (no `[[bin]]`, no live network, no new dep) + HTTP egress fixtures.
// ---------------------------------------------------------------------------

use std::io::{Read as _, Write as _};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use kx_mcp::HttpTransport;
use kx_warrant::Host;

/// What the mock HTTP server does with the one request it receives.
#[derive(Clone)]
pub enum HttpMode {
    /// 200 with `result.echoed = <request body's params.arguments>` (deterministic
    /// in the args — content-addressed dedup, like the stdio echo).
    Echo,
    /// 200 with a `result` string of `n` bytes (drives the IMP-16 oversize cap).
    Big(usize),
    /// 200 with a JSON-RPC `error` object (decoder ProtocolError path).
    Error,
    /// 200 with truncated JSON (decoder Malformed path).
    Malformed,
    /// Sleep `d`, then echo (drives the wall-clock watchdog).
    Slow(Duration),
    /// 302 with `Location: http://evil.example.com/` (drives redirect refusal).
    Redirect,
    /// 200 with `result.saw_auth = <bool>` reporting WHETHER an `Authorization`
    /// header was present — proves credential injection is in-play WITHOUT echoing
    /// the secret value (the secret-never-leak sweep then asserts absence).
    AuthProbe,
}

/// One captured inbound request (headers lowercased) for test assertions.
#[derive(Clone)]
pub struct Captured {
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Captured {
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        let name = name.to_ascii_lowercase();
        self.headers
            .iter()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| v.as_str())
    }
}

/// A hermetic in-process HTTP/1.1 mock on `127.0.0.1:0`. Each connection is handled
/// on its own detached thread so a `Slow` handler never blocks the accept loop (or
/// `Drop`). Stops cleanly on drop.
pub struct MockHttpServer {
    pub addr: SocketAddr,
    captured: Arc<Mutex<Vec<Captured>>>,
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl MockHttpServer {
    #[must_use]
    pub fn start(mode: HttpMode) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock http");
        let addr = listener.local_addr().expect("local_addr");
        let captured: Arc<Mutex<Vec<Captured>>> = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (cap_t, stop_t) = (captured.clone(), stop.clone());
        let handle = std::thread::spawn(move || {
            for stream in listener.incoming() {
                if stop_t.load(Ordering::SeqCst) {
                    break;
                }
                if let Ok(stream) = stream {
                    let (mode, cap) = (mode.clone(), cap_t.clone());
                    std::thread::spawn(move || handle_conn(stream, &mode, &cap));
                }
            }
        });
        Self {
            addr,
            captured,
            stop,
            handle: Some(handle),
        }
    }

    /// The endpoint URL a transport dials.
    #[must_use]
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/mcp", self.addr.port())
    }

    /// A `net_scope` granting egress to this server's bound (loopback) host literal.
    #[must_use]
    pub fn net_scope(&self) -> NetScope {
        NetScope::EgressAllowlist([Host(self.addr.ip().to_string())].into_iter().collect())
    }

    /// All requests captured so far (cloned).
    #[must_use]
    pub fn captured(&self) -> Vec<Captured> {
        self.captured.lock().unwrap().clone()
    }
}

impl Drop for MockHttpServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake the blocked `accept` by connecting once, then join the accept loop.
        let _ = TcpStream::connect(self.addr);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn handle_conn(mut stream: TcpStream, mode: &HttpMode, captured: &Arc<Mutex<Vec<Captured>>>) {
    let Some(req) = read_request(&mut stream) else {
        return;
    };
    let saw_auth = req.header("authorization").is_some();
    let args = extract_arguments(&req.body);
    captured.lock().unwrap().push(req);

    if let HttpMode::Slow(d) = mode {
        std::thread::sleep(*d);
    }

    let response: String = match mode {
        HttpMode::Big(n) => {
            let filler = "x".repeat(*n);
            jsonrpc_ok(&format!(r#"{{"blob":"{filler}"}}"#))
        }
        HttpMode::Error => {
            http_200(r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"mock error"}}"#)
        }
        HttpMode::Malformed => http_200(r#"{"jsonrpc":"2.0","id":1,"result":{"content":"#),
        HttpMode::Redirect => {
            "HTTP/1.1 302 Found\r\nLocation: http://evil.example.com/\r\nContent-Length: 0\r\n\r\n"
                .to_string()
        }
        HttpMode::AuthProbe => jsonrpc_ok(&format!(r#"{{"saw_auth":{saw_auth}}}"#)),
        HttpMode::Echo | HttpMode::Slow(_) => jsonrpc_ok(&format!(r#"{{"echoed":{args}}}"#)),
    };
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

/// Read an HTTP/1.1 request: headers up to `\r\n\r\n`, then `Content-Length` bytes.
fn read_request(stream: &mut TcpStream) -> Option<Captured> {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    // Read until the header terminator.
    while !buf.ends_with(b"\r\n\r\n") {
        match stream.read(&mut byte) {
            Ok(0) | Err(_) => return None,
            Ok(_) => buf.push(byte[0]),
        }
        if buf.len() > 64 * 1024 {
            return None;
        }
    }
    let head = String::from_utf8_lossy(&buf);
    let mut headers = Vec::new();
    let mut content_length = 0usize;
    for line in head.lines().skip(1) {
        if let Some((k, v)) = line.split_once(':') {
            let (k, v) = (k.trim().to_ascii_lowercase(), v.trim().to_string());
            if k == "content-length" {
                content_length = v.parse().unwrap_or(0);
            }
            headers.push((k, v));
        }
    }
    let mut body = vec![0u8; content_length];
    if content_length > 0 && stream.read_exact(&mut body).is_err() {
        return None;
    }
    Some(Captured { headers, body })
}

/// Pull `params.arguments` (verbatim JSON) out of a JSON-RPC request body; `{}` if
/// absent — mirrors the stdio echo server.
fn extract_arguments(body: &[u8]) -> String {
    #[derive(serde::Deserialize)]
    struct Req {
        params: Params,
    }
    #[derive(serde::Deserialize)]
    struct Params {
        #[serde(default)]
        arguments: Option<Box<serde_json::value::RawValue>>,
    }
    serde_json::from_slice::<Req>(body)
        .ok()
        .and_then(|r| r.params.arguments)
        .map_or_else(|| "{}".to_string(), |a| a.get().to_string())
}

fn jsonrpc_ok(result_obj: &str) -> String {
    http_200(&format!(
        r#"{{"jsonrpc":"2.0","id":1,"result":{result_obj}}}"#
    ))
}

fn http_200(body: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
        body.len(),
        body
    )
}

/// An `McpCapability` over a real `HttpTransport` pointed at `server` (echo etc.).
#[must_use]
pub fn http_capability(
    name: ToolName,
    version: ToolVersion,
    server: &MockHttpServer,
) -> McpCapability {
    let transport = HttpTransport::new(&server.url(), &server.net_scope())
        .expect("http transport builds for the loopback mock");
    McpCapability::new(
        name,
        version,
        McpEndpointId(server.url()),
        "echo",
        Box::new(transport),
    )
}

/// A warrant granting `(tool, version)` PLUS egress to the loopback mock host.
#[must_use]
pub fn warrant_granting_egress(tool_name: &ToolName, tool_version: &ToolVersion) -> WarrantSpec {
    let mut w = warrant_granting(tool_name, tool_version);
    w.net_scope = NetScope::EgressAllowlist([Host("127.0.0.1".to_string())].into_iter().collect());
    w
}

/// An `EffectRequest` carrying `args_json` under `StageThenCommit`, scoped to the
/// loopback egress host (so the broker `precheck` admits the HTTP dispatch).
#[must_use]
pub fn effect_egress(args_json: &str) -> EffectRequest {
    EffectRequest {
        payload: args_json.as_bytes().to_vec(),
        pattern: EffectPattern::StageThenCommit,
        idempotency_key: Some([0x11; 32]),
        net_scope: NetScope::EgressAllowlist([Host("127.0.0.1".to_string())].into_iter().collect()),
        fs_scope: FsScope::empty(),
    }
}
