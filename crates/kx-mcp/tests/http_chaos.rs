//! M5.2b chaos — the HTTP transport fails CLOSED + promptly under adversarial
//! servers: a slow server is timed out by the watchdog, a redirect is refused, an
//! oversize body is capped. Every failure is a typed `CapabilityFailureReason`
//! (never a panic, never a partial stage).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::doc_markdown)]

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use common::{effect_egress, sample_mote, tool, warrant_granting_egress, HttpMode, MockHttpServer};
use kx_capability::{
    BrokerError, CapabilityBroker, CapabilityFailureReason, LocalCapabilityBroker,
};
use kx_content::InMemoryContentStore;
use kx_mcp::{HttpTransport, McpCapability};
use kx_mote::{ToolName, ToolVersion};
use kx_tool_registry::McpEndpointId;

fn broker_with(
    server: &MockHttpServer,
    name: &ToolName,
    version: &ToolVersion,
    cap_mut: impl FnOnce(McpCapability) -> McpCapability,
) -> LocalCapabilityBroker<Arc<InMemoryContentStore>> {
    let store = Arc::new(InMemoryContentStore::new());
    let broker = LocalCapabilityBroker::new(store);
    let transport = HttpTransport::new(&server.url(), &server.net_scope()).unwrap();
    let cap = McpCapability::new(
        name.clone(),
        version.clone(),
        McpEndpointId(server.url()),
        "echo",
        Box::new(transport),
    );
    broker.register_capability(Box::new(cap_mut(cap)));
    broker
}

#[test]
fn slow_server_is_timed_out_promptly() {
    let server = MockHttpServer::start(HttpMode::Slow(Duration::from_secs(5)));
    let (name, version) = tool();
    let broker = broker_with(&server, &name, &version, |c| c.with_wall_clock_ms(200));
    let mote = sample_mote(&name, &version);
    let warrant = warrant_granting_egress(&name, &version);

    let start = Instant::now();
    let err = broker
        .dispatch(&mote, &warrant, &name, effect_egress("{}"))
        .expect_err("a slow server must be abandoned");
    let elapsed = start.elapsed();

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
    assert!(
        elapsed < Duration::from_secs(2),
        "the watchdog must return within ~the budget, took {elapsed:?}"
    );
}

#[test]
fn cross_host_redirect_is_refused_and_nothing_staged() {
    let server = MockHttpServer::start(HttpMode::Redirect);
    let (name, version) = tool();
    let broker = broker_with(&server, &name, &version, |c| c);
    let mote = sample_mote(&name, &version);
    let warrant = warrant_granting_egress(&name, &version);

    let err = broker
        .dispatch(&mote, &warrant, &name, effect_egress("{}"))
        .expect_err("a redirect must be refused (no following)");
    assert!(
        matches!(
            err,
            BrokerError::CapabilityFailure {
                reason: CapabilityFailureReason::NetworkUnreachable,
                ..
            }
        ),
        "a 3xx is refused as unreachable, got {err:?}"
    );
}

#[test]
fn oversize_response_is_capped_and_refused() {
    let server = MockHttpServer::start(HttpMode::Big(8192));
    let (name, version) = tool();
    let broker = broker_with(&server, &name, &version, |c| {
        c.with_max_response_bytes(1024)
    });
    let mote = sample_mote(&name, &version);
    let warrant = warrant_granting_egress(&name, &version);

    let err = broker
        .dispatch(&mote, &warrant, &name, effect_egress("{}"))
        .expect_err("an oversize body must be refused");
    assert!(
        matches!(
            err,
            BrokerError::CapabilityFailure {
                reason: CapabilityFailureReason::InvalidResponse,
                ..
            }
        ),
        "oversize → InvalidResponse, got {err:?}"
    );
}

#[test]
fn server_protocol_error_is_surfaced_not_staged() {
    let server = MockHttpServer::start(HttpMode::Error);
    let (name, version) = tool();
    let broker = broker_with(&server, &name, &version, |c| c);
    let mote = sample_mote(&name, &version);
    let warrant = warrant_granting_egress(&name, &version);

    let err = broker
        .dispatch(&mote, &warrant, &name, effect_egress("{}"))
        .expect_err("a JSON-RPC error object must not be staged as a result");
    assert!(matches!(err, BrokerError::CapabilityFailure { .. }));
}
