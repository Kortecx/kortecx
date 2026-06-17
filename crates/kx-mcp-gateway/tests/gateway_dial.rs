//! End-to-end: dial a STATEFUL stdio MCP server (the `kx-mcp-test-stdio` support
//! bin) over the real `kx-mcp` session seam, discover its tools, register them
//! into the durable registry + the broker sink, and fire one tool through the
//! registered [`McpSessionCapability`]. Exercises the full PR-6b-1 dial path.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex};

use kx_capability::{Capability, EffectRequest};
use kx_mcp_gateway::{
    CapabilitySink, ConnectionHealth, McpGateway, SqliteConnectionStore, TransportSpec,
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
        )
        .unwrap();
    assert_eq!(outcome.health, ConnectionHealth::Unreachable);
    assert_eq!(outcome.discovered, 0);
    assert_eq!(gateway.list_servers().unwrap().len(), 1);
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
        )
        .unwrap_err();
    assert!(matches!(err, kx_mcp_gateway::GatewayError::InvalidSpec(_)));
    assert!(gateway.list_servers().unwrap().is_empty());
}
