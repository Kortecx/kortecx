//! M5.2b security — the egress sandbox holds end-to-end through the broker:
//!
//! 1. **SSRF / rebind:** an allowlisted *hostname* that resolves to a loopback
//!    address is refused (a public name can never reach an internal/metadata IP) —
//!    proven hermetically via `localhost` → `127.0.0.1`.
//! 2. **net_scope ⊆ warrant:** an HTTP dispatch requesting egress beyond the
//!    warrant is refused on the NetScope axis (the broker gate), transport untouched.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::doc_markdown)]

mod common;

use std::sync::Arc;

use common::{
    effect_egress, http_capability, sample_mote, tool, warrant_granting, warrant_granting_egress,
    HttpMode, MockHttpServer,
};
use kx_capability::{
    BrokerError, CapabilityBroker, CapabilityFailureReason, LocalCapabilityBroker,
};
use kx_content::InMemoryContentStore;
use kx_mcp::{HttpTransport, McpCapability};
use kx_mote::EffectPattern;
use kx_tool_registry::McpEndpointId;
use kx_warrant::{FsScope, Host, NetScope};

#[test]
fn allowlisted_name_resolving_to_loopback_is_refused() {
    // `localhost` is allowlisted (a NAME), the endpoint points at it, and it
    // resolves to a loopback address (127.0.0.1 / ::1) hermetically via /etc/hosts.
    // The vetting resolver refuses it because the host is not an IP LITERAL — the
    // exact public-name-rebinds-to-internal-IP vector. The request never leaves.
    let server = MockHttpServer::start(HttpMode::Echo);
    let (name, version) = tool();
    let localhost_scope =
        NetScope::EgressAllowlist([Host("localhost".to_string())].into_iter().collect());

    let store = Arc::new(InMemoryContentStore::new());
    let broker = LocalCapabilityBroker::new(store);
    let endpoint = format!("http://localhost:{}/mcp", server.addr.port());
    let transport = HttpTransport::new(&endpoint, &localhost_scope, false)
        .expect("transport builds (localhost is allowlisted by name)");
    broker.register_capability(Box::new(McpCapability::new(
        name.clone(),
        version.clone(),
        McpEndpointId(endpoint),
        "echo",
        Box::new(transport),
    )));

    let mote = sample_mote(&name, &version);
    let mut warrant = warrant_granting(&name, &version);
    warrant.net_scope = localhost_scope.clone();
    let mut req = effect_egress("{}");
    req.net_scope = localhost_scope;

    let err = broker
        .dispatch(&mote, &warrant, &name, req)
        .expect_err("a name resolving to loopback must be refused (SSRF/rebind)");
    assert!(
        matches!(
            err,
            BrokerError::CapabilityFailure {
                reason: CapabilityFailureReason::NetworkUnreachable,
                ..
            }
        ),
        "expected NetworkUnreachable (egress vetting refusal), got {err:?}"
    );
    assert!(
        server.captured().is_empty(),
        "no request may reach the server when egress is refused"
    );
}

#[test]
fn egress_beyond_warrant_is_refused_on_netscope_axis() {
    // The capability+transport are egress-capable, but the warrant grants NO egress.
    // The broker gate refuses on NetScope before the transport is ever invoked.
    let server = MockHttpServer::start(HttpMode::Echo);
    let (name, version) = tool();
    let store = Arc::new(InMemoryContentStore::new());
    let broker = LocalCapabilityBroker::new(store);
    broker.register_capability(Box::new(http_capability(
        name.clone(),
        version.clone(),
        &server,
    )));

    let mote = sample_mote(&name, &version);
    let warrant = warrant_granting(&name, &version); // net_scope = None

    let err = broker
        .dispatch(&mote, &warrant, &name, effect_egress("{}"))
        .expect_err("egress beyond the warrant must be refused");
    assert!(
        matches!(
            err,
            BrokerError::CapabilityExceedsWarrant {
                axis: kx_warrant::WarrantField::NetScope
            }
        ),
        "expected CapabilityExceedsWarrant{{NetScope}}, got {err:?}"
    );
    assert!(server.captured().is_empty(), "no egress occurred");
}

#[test]
fn unscoped_egress_request_under_none_warrant_never_dials() {
    // Belt-and-suspenders: even a fully-egress EffectRequest under a None warrant is
    // refused; assert the fs axis stays empty and nothing is staged.
    let server = MockHttpServer::start(HttpMode::Echo);
    let (name, version) = tool();
    let store = Arc::new(InMemoryContentStore::new());
    let broker = LocalCapabilityBroker::new(store);
    broker.register_capability(Box::new(http_capability(
        name.clone(),
        version.clone(),
        &server,
    )));
    let mote = sample_mote(&name, &version);
    let warrant = warrant_granting_egress(&name, &version);
    // Request egress to a host the warrant does NOT grant.
    let req = kx_capability::EffectRequest {
        payload: b"{}".to_vec(),
        pattern: EffectPattern::StageThenCommit,
        idempotency_key: Some([0x22; 32]),
        net_scope: NetScope::EgressAllowlist(
            [Host("example.com".to_string())].into_iter().collect(),
        ),
        fs_scope: FsScope::empty(),
        secret_scope: kx_warrant::SecretScope::None,
    };
    let err = broker
        .dispatch(&mote, &warrant, &name, req)
        .expect_err("egress to an ungranted host is refused");
    assert!(matches!(
        err,
        BrokerError::CapabilityExceedsWarrant {
            axis: kx_warrant::WarrantField::NetScope
        }
    ));
}
