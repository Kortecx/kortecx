//! RW-B security: the host allowlist is re-vetted on REDIAL, not only at admission.
//!
//! `register_server` vets `KX_SERVE_TOOL_HOST_ALLOWLIST` at first admission, but both
//! `discover` and `redial_persisted` re-dial persisted connections through the shared
//! `dial_and_register` helper. Before the fix that helper never re-checked the allowlist,
//! so a host an operator REMOVED from the allowlist stayed fully dialable across restart —
//! an allowlist tightening was not retroactive. These tests persist a connection whose host
//! was allowed at registration, then exercise the two re-dial paths under a TIGHTENED
//! allowlist and assert the host is refused (`HostRejected`) before any dial. SN-8: a redial
//! is a fresh admission, not an inherited grant.
//!
//! Fully hermetic: with the fix the vet fires before `probe`, so no test here touches the
//! network. The positive control's dial targets a `.invalid` host (RFC 6761 — never resolves)
//! so it fails fast at dial rather than the gate.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex};

use kx_capability::Capability;
use kx_mcp_gateway::{
    connection_id_of, CapabilitySink, Connection, ConnectionHealth, GatewayError, McpGateway,
    SessionMode, SqliteConnectionStore, TransportSpec,
};
use kx_tool_registry::SqliteToolRegistry;

/// A sink that just counts registered capabilities — a refused redial must register none.
#[derive(Default)]
struct CountingSink(Mutex<usize>);

impl CapabilitySink for CountingSink {
    fn register_capability(&self, _capability: Box<dyn Capability>) {
        *self.0.lock().unwrap() += 1;
    }
}

/// Persist a durable HTTP connection as if a prior allow-listed dial had succeeded.
fn persist_http(store: &SqliteConnectionStore, name: &str, url: &str) {
    store
        .upsert(&Connection {
            id: connection_id_of(name),
            name: name.to_string(),
            transport: TransportSpec::Http {
                url: url.to_string(),
                tls_required: true,
            },
            credential_ref: None,
            health: ConnectionHealth::Connected,
            tool_count: 1,
            session_mode: SessionMode::Stateless,
        })
        .unwrap();
}

fn gateway_with_allowlist(
    store: SqliteConnectionStore,
    sink: Arc<CountingSink>,
    allowlist: Vec<String>,
) -> McpGateway {
    let registry = Arc::new(SqliteToolRegistry::open_in_memory().unwrap());
    McpGateway::new(store, registry, sink as Arc<dyn CapabilitySink>, allowlist)
}

#[test]
fn discover_of_a_host_removed_from_the_allowlist_is_refused() {
    let store = SqliteConnectionStore::open_in_memory().unwrap();
    // Registered while `mcp.example.com` was on the allowlist.
    persist_http(&store, "svc", "https://mcp.example.com/rpc");
    let sink = Arc::new(CountingSink::default());
    // The operator has since TIGHTENED the allowlist — the host is no longer on it.
    let gateway = gateway_with_allowlist(store, sink.clone(), vec!["allowed.example.org".into()]);

    let err = gateway.discover("svc").unwrap_err();
    assert!(
        matches!(err, GatewayError::HostRejected(_)),
        "a redial of a host removed from the allowlist must be refused before dial, got {err:?}"
    );
    // Refused before `probe`: nothing registered (SN-8: no live capability ⇒ unfireable),
    // and the folded health is Unreachable.
    assert_eq!(
        *sink.0.lock().unwrap(),
        0,
        "a refused redial must register no tool capability"
    );
    assert_eq!(
        gateway.list_servers().unwrap()[0].health,
        ConnectionHealth::Unreachable
    );
}

#[test]
fn redial_persisted_marks_a_now_disallowed_host_unreachable() {
    let store = SqliteConnectionStore::open_in_memory().unwrap();
    persist_http(&store, "svc", "https://mcp.example.com/rpc");
    let sink = Arc::new(CountingSink::default());
    let gateway = gateway_with_allowlist(store, sink.clone(), vec!["allowed.example.org".into()]);

    // The startup re-dial is fail-soft (never aborts serve) but must fold Unreachable and
    // re-register nothing for a host that is no longer admissible.
    gateway.redial_persisted().unwrap();
    assert_eq!(
        *sink.0.lock().unwrap(),
        0,
        "a now-disallowed host must contribute no capability on restart"
    );
    assert_eq!(
        gateway.list_servers().unwrap()[0].health,
        ConnectionHealth::Unreachable
    );
}

#[test]
fn a_still_allowed_host_passes_the_redial_gate() {
    let store = SqliteConnectionStore::open_in_memory().unwrap();
    // A `.invalid` host (RFC 6761 — never resolves) that IS on the allowlist.
    persist_http(&store, "svc", "https://mcp.invalid/rpc");
    let sink = Arc::new(CountingSink::default());
    let gateway = gateway_with_allowlist(store, sink, vec!["mcp.invalid".into()]);

    // The re-vet must not over-reject: an allowed host passes the gate and fails later at
    // the dial (unresolvable), NOT at the allowlist — so the error is anything but HostRejected.
    let err = gateway.discover("svc").unwrap_err();
    assert!(
        !matches!(err, GatewayError::HostRejected(_)),
        "a still-allowed host must clear the allowlist re-vet (it fails at dial, not the gate), got {err:?}"
    );
}
