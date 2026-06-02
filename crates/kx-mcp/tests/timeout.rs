//! The transport wall-clock watchdog: a slow/hung MCP server is abandoned within
//! the per-call budget (it does NOT block the dispatch for the server's full
//! sleep), reaped without a zombie, and surfaced as a typed `Timeout` failure.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::sync::Arc;
use std::time::Instant;

use common::{effect, sample_mote, tool, warrant_granting, MOCK_SERVER};
use kx_capability::{
    BrokerError, CapabilityBroker, CapabilityFailureReason, LocalCapabilityBroker,
};
use kx_content::InMemoryContentStore;
use kx_mcp::{McpCapability, StdioTransport};
use kx_tool_registry::McpEndpointId;

#[test]
fn slow_server_is_timed_out_promptly() {
    let (name, version) = tool();
    // The server sleeps 60s before responding; the capability budgets 150ms.
    let transport = Box::new(
        StdioTransport::new(MOCK_SERVER)
            .env("KX_MCP_MOCK_MODE", "slow")
            .env("KX_MCP_MOCK_SLEEP_MS", "60000"),
    );
    let cap = McpCapability::new(
        name.clone(),
        version.clone(),
        McpEndpointId("stdio://mock".into()),
        "echo",
        transport,
    )
    .with_wall_clock_ms(150);

    let store = Arc::new(InMemoryContentStore::new());
    let broker = LocalCapabilityBroker::new(store);
    broker.register_capability(Box::new(cap));

    let mote = sample_mote(&name, &version);
    let warrant = warrant_granting(&name, &version);

    let started = Instant::now();
    let err = broker
        .dispatch(&mote, &warrant, &name, effect("{}"))
        .expect_err("a slow server must time out");
    let elapsed = started.elapsed();

    // The watchdog fired (Timeout), NOT the 60s server sleep completing.
    assert!(
        matches!(
            err,
            BrokerError::CapabilityFailure {
                reason: CapabilityFailureReason::Timeout,
                ..
            }
        ),
        "expected a Timeout failure, got {err:?}"
    );
    // And it returned promptly — the dispatch did not wait out the 60s sleep
    // (generous bound to avoid CI flakiness; the budget was 150ms).
    assert!(
        elapsed.as_secs() < 10,
        "dispatch should abandon the hung server promptly, took {elapsed:?}"
    );
}
