//! D80 — `McpCapability` dispatches THROUGH the real `LocalCapabilityBroker`
//! (the authoritative warrant gate) over the real `StdioTransport`, and the
//! staged result carries the MCP capability identity as provenance (D72).

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::sync::Arc;

use common::{effect, sample_mote, tool, warrant_granting, MOCK_SERVER};
use kx_capability::{BrokerError, CapabilityBroker, LocalCapabilityBroker};
use kx_content::{ContentStore, InMemoryContentStore};
use kx_mcp::{McpCapability, StdioTransport};
use kx_mote::ToolName;
use kx_tool_registry::McpEndpointId;
use kx_warrant::{NetScope, WarrantField};

fn echo_capability(name: ToolName, version: kx_mote::ToolVersion) -> McpCapability {
    let transport = Box::new(StdioTransport::new(MOCK_SERVER)); // default echo mode
    McpCapability::new(
        name,
        version,
        McpEndpointId("stdio://mock".into()),
        "echo",
        transport,
    )
}

#[test]
fn dispatches_through_broker_stages_result_with_provenance() {
    let (name, version) = tool();
    let store = Arc::new(InMemoryContentStore::new());
    let broker = LocalCapabilityBroker::new(store.clone());
    broker.register_capability(Box::new(echo_capability(name.clone(), version.clone())));

    let mote = sample_mote(&name, &version);
    let warrant = warrant_granting(&name, &version);

    let handle = broker
        .dispatch(&mote, &warrant, &name, effect(r#"{"q":"hello"}"#))
        .expect("MCP dispatch should succeed through the broker");

    // Provenance (D72): the handle records WHICH capability/version produced the bytes.
    assert_eq!(handle.capability, name);
    assert_eq!(handle.capability_version, version);

    // The staged bytes are the MCP server's `result` object, verbatim.
    let staged = store.get(&handle.staged_ref).unwrap();
    assert_eq!(&*staged, br#"{"echoed":{"q":"hello"}}"#);
}

#[test]
fn refused_when_capability_not_in_tool_contract() {
    // A Mote whose tool_contract does NOT declare the tool ⇒ UnknownCapability,
    // even though the capability is registered + granted (the broker gate holds).
    let (name, version) = tool();
    let store = Arc::new(InMemoryContentStore::new());
    let broker = LocalCapabilityBroker::new(store);
    broker.register_capability(Box::new(echo_capability(name.clone(), version.clone())));

    let other = ToolName("not-declared".into());
    let mote = sample_mote(&other, &version); // contract declares `not-declared`, not `mcp-echo`
    let warrant = warrant_granting(&name, &version);

    let err = broker
        .dispatch(&mote, &warrant, &name, effect("{}"))
        .expect_err("a tool not in the contract must be refused");
    assert!(matches!(err, BrokerError::UnknownCapability { .. }));
}

#[test]
fn refused_when_net_scope_exceeds_warrant() {
    // The broker gate enforces request.net_scope ⊆ warrant.net_scope. An MCP
    // dispatch that requests egress under a None-egress warrant is refused on the
    // NetScope axis (the M5.2b HTTP path relies on exactly this gate).
    let (name, version) = tool();
    let store = Arc::new(InMemoryContentStore::new());
    let broker = LocalCapabilityBroker::new(store);
    broker.register_capability(Box::new(echo_capability(name.clone(), version.clone())));

    let mote = sample_mote(&name, &version);
    let warrant = warrant_granting(&name, &version); // net_scope = None
    let mut req = effect("{}");
    req.net_scope = NetScope::EgressAllowlist(
        [kx_warrant::Host("example.com".into())]
            .into_iter()
            .collect(),
    );

    let err = broker
        .dispatch(&mote, &warrant, &name, req)
        .expect_err("egress beyond the warrant must be refused");
    assert!(matches!(
        err,
        BrokerError::CapabilityExceedsWarrant {
            axis: WarrantField::NetScope
        }
    ));
}
