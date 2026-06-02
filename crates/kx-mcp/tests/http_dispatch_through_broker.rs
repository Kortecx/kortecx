//! M5.2b — `McpCapability` over the real `HttpTransport` dispatches THROUGH the
//! real `LocalCapabilityBroker` (the authoritative warrant gate, including the
//! net_scope egress check) against a hermetic in-process HTTP server, and the
//! staged result carries the capability identity as provenance (D72).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::doc_markdown)]

mod common;

use std::sync::Arc;

use common::{
    effect_egress, http_capability, sample_mote, tool, warrant_granting_egress, HttpMode,
    MockHttpServer,
};
use kx_capability::{CapabilityBroker, LocalCapabilityBroker};
use kx_content::{ContentStore, InMemoryContentStore};

#[test]
fn http_dispatch_stages_echo_result_with_provenance() {
    let server = MockHttpServer::start(HttpMode::Echo);
    let (name, version) = tool();
    let store = Arc::new(InMemoryContentStore::new());
    let broker = LocalCapabilityBroker::new(store.clone());
    broker.register_capability(Box::new(http_capability(
        name.clone(),
        version.clone(),
        &server,
    )));

    let mote = sample_mote(&name, &version);
    let warrant = warrant_granting_egress(&name, &version);

    let handle = broker
        .dispatch(&mote, &warrant, &name, effect_egress(r#"{"q":"hello"}"#))
        .expect("HTTP MCP dispatch should succeed through the broker");

    // Provenance (D72): the handle records WHICH capability/version produced the bytes.
    assert_eq!(handle.capability, name);
    assert_eq!(handle.capability_version, version);

    // The staged bytes are the MCP server's `result` object, verbatim.
    let staged = store.get(&handle.staged_ref).unwrap();
    assert_eq!(&*staged, br#"{"echoed":{"q":"hello"}}"#);

    // The server actually received the POST (the egress happened, warrant-scoped).
    let reqs = server.captured();
    assert_eq!(reqs.len(), 1, "exactly one request reached the server");
    // The run-scoped idempotency key rode as the Idempotency-Key header (remote
    // exactly-once seam) — present and 64 lower-hex chars.
    let key = reqs[0]
        .header("idempotency-key")
        .expect("Idempotency-Key header is sent");
    assert_eq!(key.len(), 64);
    assert!(key.bytes().all(|b| b.is_ascii_hexdigit()));
}

#[test]
fn http_dispatch_is_deterministic_across_calls() {
    // Same args ⇒ byte-identical staged bytes ⇒ identical content ref (the property
    // that makes a recovery re-dispatch exactly-once at staging).
    let server = MockHttpServer::start(HttpMode::Echo);
    let (name, version) = tool();
    let store = Arc::new(InMemoryContentStore::new());
    let broker = LocalCapabilityBroker::new(store.clone());
    broker.register_capability(Box::new(http_capability(
        name.clone(),
        version.clone(),
        &server,
    )));
    let mote = sample_mote(&name, &version);
    let warrant = warrant_granting_egress(&name, &version);

    let a = broker
        .dispatch(&mote, &warrant, &name, effect_egress(r#"{"q":"x"}"#))
        .unwrap();
    let b = broker
        .dispatch(&mote, &warrant, &name, effect_egress(r#"{"q":"x"}"#))
        .unwrap();
    assert_eq!(a.staged_ref, b.staged_ref);
}
