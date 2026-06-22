//! End-to-end: dial a STATEFUL stdio MCP server (the `kx-mcp-test-stdio` support
//! bin) over the real `kx-mcp` session seam, discover its tools, register them
//! into the durable registry + the broker sink, and fire one tool through the
//! registered [`McpSessionCapability`]. Exercises the full PR-6b-1 dial path.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex};

use kx_capability::{Capability, EffectRequest};
use kx_mcp_gateway::{
    CapabilitySink, ConnectionHealth, McpGateway, SessionMode, SqliteConnectionStore, TransportSpec,
};
use kx_mote::EffectPattern;
use kx_tool_registry::{SqliteToolRegistry, ToolRegistry};
use kx_warrant::{FsScope, NetScope, SecretScope};

/// A test sink that retains the registered capabilities so we can fire one.
#[derive(Default)]
struct CollectingSink(Mutex<Vec<Box<dyn Capability>>>);

impl CapabilitySink for CollectingSink {
    fn register_capability(&self, capability: Box<dyn Capability>) {
        self.0.lock().unwrap().push(capability);
    }
}

fn test_server_command() -> String {
    env!("CARGO_BIN_EXE_kx-mcp-test-stdio").to_string()
}

#[test]
fn dial_discover_register_fire_roundtrip() {
    let registry = Arc::new(SqliteToolRegistry::open_in_memory().unwrap());
    let store = SqliteConnectionStore::open_in_memory().unwrap();
    let sink = Arc::new(CollectingSink::default());
    let gateway = McpGateway::new(
        store,
        registry.clone(),
        sink.clone() as Arc<dyn CapabilitySink>,
        vec![],
    );

    // Register a stdio server pointing at the stateful test bin.
    let outcome = gateway
        .register_server(
            "tools",
            TransportSpec::Stdio {
                command: test_server_command(),
                args: vec![],
            },
            None,
            SessionMode::Stateless,
        )
        .unwrap();
    assert_eq!(outcome.health, ConnectionHealth::Connected);
    assert_eq!(outcome.discovered, 2, "echo + ping");
    assert_ne!(outcome.connection_id, [0u8; 16]);

    // The discovered tools are in the durable registry, namespaced by the server.
    let echo = registry
        .lookup(
            &kx_mote::ToolName("tools/echo".into()),
            &kx_mote::ToolVersion("1".into()),
        )
        .expect("tools/echo registered");
    assert!(matches!(echo.kind, kx_tool_registry::ToolKind::Mcp { .. }));
    // The remote-side schema (q: string, required) mapped into the typed schema.
    assert!(echo.input_schema.is_some());
    assert!(registry
        .lookup(
            &kx_mote::ToolName("tools/ping".into()),
            &kx_mote::ToolVersion("1".into())
        )
        .is_some());

    // Two capabilities were registered on the broker sink.
    assert_eq!(sink.0.lock().unwrap().len(), 2);

    // The server is listed + healthy.
    let servers = gateway.list_servers().unwrap();
    assert_eq!(servers.len(), 1);
    assert_eq!(servers[0].name, "tools");
    assert_eq!(servers[0].tool_count, 2);

    // `test` re-dials + initializes.
    assert!(gateway.test("tools").unwrap());

    // FIRE the registered echo capability directly (the live call path).
    let caps = sink.0.lock().unwrap();
    let echo_cap = caps
        .iter()
        .find(|c| c.name().0.ends_with("echo"))
        .expect("echo capability");
    let req = EffectRequest {
        payload: br#"{"q":"hi"}"#.to_vec(),
        pattern: EffectPattern::StageThenCommit,
        idempotency_key: None,
        net_scope: NetScope::None,
        fs_scope: FsScope::empty(),
        secret_scope: SecretScope::None,
    };
    let result = echo_cap.invoke(&req).expect("echo fires");
    let result_str = String::from_utf8(result).unwrap();
    assert!(
        result_str.contains(r#""echoed":{"q":"hi"}"#),
        "got: {result_str}"
    );
    drop(caps);

    // Deregister removes the connection + its namespaced tools.
    assert!(gateway.deregister_server("tools").unwrap());
    assert!(gateway.list_servers().unwrap().is_empty());
    assert!(registry
        .lookup(
            &kx_mote::ToolName("tools/echo".into()),
            &kx_mote::ToolVersion("1".into())
        )
        .is_none());
}

#[test]
fn unreachable_server_registers_with_unreachable_health() {
    let registry = Arc::new(SqliteToolRegistry::open_in_memory().unwrap());
    let store = SqliteConnectionStore::open_in_memory().unwrap();
    let sink = Arc::new(CollectingSink::default());
    let gateway = McpGateway::new(store, registry, sink as Arc<dyn CapabilitySink>, vec![]);

    // A stdio command that does not exist: register succeeds (stored) but health
    // is Unreachable, discovered = 0 — honest, never a fabricated success.
    let outcome = gateway
        .register_server(
            "dead",
            TransportSpec::Stdio {
                command: "/nonexistent/kx-mcp-no-such-binary".into(),
                args: vec![],
            },
            None,
            SessionMode::Stateless,
        )
        .unwrap();
    assert_eq!(outcome.health, ConnectionHealth::Unreachable);
    assert_eq!(outcome.discovered, 0);
    assert_eq!(gateway.list_servers().unwrap().len(), 1);
    // T-CONN: `test` AGREES with `register` on a dead server (both Unreachable/false).
    assert!(
        !gateway.test("dead").unwrap(),
        "test agrees with register that a dead server is unreachable"
    );
}

/// T-CONN regression: a server whose `initialize` SUCCEEDS but whose `tools/list`
/// FAILS must report the SAME reachability from `register` (the `add` path) and
/// `test`. Before the shared `probe`, `test` stopped at `initialize` (→ reachable)
/// while `register` went on to `tools/list` (→ unreachable) — the two disagreed.
#[test]
fn register_and_test_agree_on_an_initialize_only_server() {
    let registry = Arc::new(SqliteToolRegistry::open_in_memory().unwrap());
    let store = SqliteConnectionStore::open_in_memory().unwrap();
    let sink = Arc::new(CollectingSink::default());
    let gateway = McpGateway::new(store, registry, sink as Arc<dyn CapabilitySink>, vec![]);

    // The support bin handshakes (`initialize`) but errors on `tools/list`.
    let outcome = gateway
        .register_server(
            "halflive",
            TransportSpec::Stdio {
                command: test_server_command(),
                args: vec!["--tools-list-error".into()],
            },
            None,
            SessionMode::Stateless,
        )
        .unwrap();
    // register: the full handshake the gateway needs failed ⇒ Unreachable, 0 tools.
    assert_eq!(outcome.health, ConnectionHealth::Unreachable);
    assert_eq!(outcome.discovered, 0);
    // test: routes through the SAME probe ⇒ also unreachable. The two AGREE (the fix).
    assert!(
        !gateway.test("halflive").unwrap(),
        "test agrees with register: a server that can't list tools is unreachable"
    );
}

#[test]
fn internal_http_host_is_refused_at_admission() {
    let registry = Arc::new(SqliteToolRegistry::open_in_memory().unwrap());
    let store = SqliteConnectionStore::open_in_memory().unwrap();
    let sink = Arc::new(CollectingSink::default());
    let gateway = McpGateway::new(store, registry, sink as Arc<dyn CapabilitySink>, vec![]);

    // SSRF: the cloud metadata endpoint is refused at admission (never stored).
    let err = gateway
        .register_server(
            "evil",
            TransportSpec::Http {
                url: "http://169.254.169.254/latest/meta-data".into(),
                tls_required: false,
            },
            None,
            SessionMode::Stateless,
        )
        .unwrap_err();
    assert!(matches!(err, kx_mcp_gateway::GatewayError::HostRejected(_)));
    assert!(gateway.list_servers().unwrap().is_empty());
}

#[test]
fn userinfo_embedded_credentials_in_url_are_refused() {
    let registry = Arc::new(SqliteToolRegistry::open_in_memory().unwrap());
    let store = SqliteConnectionStore::open_in_memory().unwrap();
    let sink = Arc::new(CollectingSink::default());
    let gateway = McpGateway::new(store, registry, sink as Arc<dyn CapabilitySink>, vec![]);

    // review #4: a `user:pass@host` URL is refused at admission (never stored) so
    // a secret can't leak into connections.db / the wire / logs (D81).
    let err = gateway
        .register_server(
            "leaky",
            TransportSpec::Http {
                url: "https://user:secret@mcp.example.com/rpc".into(),
                tls_required: true,
            },
            None,
            SessionMode::Stateless,
        )
        .unwrap_err();
    assert!(matches!(err, kx_mcp_gateway::GatewayError::InvalidSpec(_)));
    assert!(gateway.list_servers().unwrap().is_empty());
}

#[test]
fn negotiates_old_and_new_protocol_versions() {
    // PR-6b-3: the client interoperates with BOTH an OLD (`2025-06-18`) and a NEW
    // (`2026-07-28` RC) server — `initialize` captures the negotiated version and
    // proceeds either way (never a hard gate). Driven over the REAL kx-mcp stdio
    // session seam against the parameterizable test server.
    use kx_mcp::{McpTransport, StdioTransport};

    let old = StdioTransport::new(test_server_command());
    let mut s_old = old.open_session().unwrap();
    assert_eq!(
        s_old.initialize(0).unwrap(),
        "2025-06-18",
        "old server negotiates down"
    );

    let new = StdioTransport::new(test_server_command())
        .arg("--protocol-version")
        .arg("2026-07-28");
    let mut s_new = new.open_session().unwrap();
    assert_eq!(s_new.initialize(0).unwrap(), "2026-07-28", "new RC server");
}

#[test]
fn stateful_session_reuses_one_connection_across_invokes() {
    // PR-6b-3: a connection registered `Stateful` reuses ONE long-lived session
    // across invokes — proven by the test server's per-process call counter
    // advancing 1 → 2 (a stateless capability would spawn a fresh process and
    // reset to 1 on each invoke).
    let registry = Arc::new(SqliteToolRegistry::open_in_memory().unwrap());
    let store = SqliteConnectionStore::open_in_memory().unwrap();
    let sink = Arc::new(CollectingSink::default());
    let gateway = McpGateway::new(
        store,
        registry,
        sink.clone() as Arc<dyn CapabilitySink>,
        vec![],
    );
    gateway
        .register_server(
            "tools",
            TransportSpec::Stdio {
                command: test_server_command(),
                args: vec![],
            },
            None,
            SessionMode::Stateful,
        )
        .unwrap();

    let caps = sink.0.lock().unwrap();
    let echo = caps
        .iter()
        .find(|c| c.name().0.ends_with("echo"))
        .expect("echo capability");
    let req = || EffectRequest {
        payload: br#"{"q":"x"}"#.to_vec(),
        pattern: EffectPattern::StageThenCommit,
        idempotency_key: None,
        net_scope: NetScope::None,
        fs_scope: FsScope::empty(),
        secret_scope: SecretScope::None,
    };
    let r1 = String::from_utf8(echo.invoke(&req()).expect("first invoke")).unwrap();
    let r2 = String::from_utf8(echo.invoke(&req()).expect("second invoke")).unwrap();
    assert!(
        r1.contains(r#""call":1"#),
        "first call on a fresh session: {r1}"
    );
    assert!(
        r2.contains(r#""call":2"#),
        "second call reuses the SAME session (counter advances): {r2}"
    );
}
