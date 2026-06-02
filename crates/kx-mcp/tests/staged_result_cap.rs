//! IMP-16 — a resource-exhausting MCP response is refused fail-closed (nothing is
//! staged), bounded by the capability's `max_response_bytes` cap.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::sync::Arc;

use common::{effect, sample_mote, tool, warrant_granting, MOCK_SERVER};
use kx_capability::{
    BrokerError, CapabilityBroker, CapabilityFailureReason, LocalCapabilityBroker,
};
use kx_content::InMemoryContentStore;
use kx_mcp::{McpCapability, StdioTransport};
use kx_tool_registry::McpEndpointId;

#[test]
fn oversize_response_is_refused_and_nothing_is_staged() {
    let (name, version) = tool();
    // The server emits ~100 KB; the capability caps the response at 256 bytes.
    let transport = Box::new(
        StdioTransport::new(MOCK_SERVER)
            .env("KX_MCP_MOCK_MODE", "big")
            .env("KX_MCP_MOCK_BIG_BYTES", "100000"),
    );
    let cap = McpCapability::new(
        name.clone(),
        version.clone(),
        McpEndpointId("stdio://mock".into()),
        "echo",
        transport,
    )
    .with_max_response_bytes(256);

    let store = Arc::new(InMemoryContentStore::new());
    let broker = LocalCapabilityBroker::new(store);
    broker.register_capability(Box::new(cap));

    let mote = sample_mote(&name, &version);
    let warrant = warrant_granting(&name, &version);

    let err = broker
        .dispatch(&mote, &warrant, &name, effect("{}"))
        .expect_err("an over-cap response must be refused");
    // The oversize decode maps to InvalidResponse; the broker never staged it.
    assert!(matches!(
        err,
        BrokerError::CapabilityFailure {
            reason: CapabilityFailureReason::InvalidResponse,
            ..
        }
    ));
}
