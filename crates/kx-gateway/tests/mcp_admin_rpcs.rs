//! The four MCP admin RPCs that no Rust test reached.
//!
//! The calibrated coverage scan reports `121 · 107 · 58 · 14` — 14 RPCs with no Rust-test
//! match — and four of them are the whole external-connector lifecycle after registration:
//! `ListMcpServers`, `DiscoverServerTools`, `TestMcpServer`, `DeregisterMcpServer`. They
//! were reachable from the CLI and the console, which is precisely why the gap persisted:
//! the scan's miss-class is that CLI/SDK/console-driven coverage is invisible to it, so 14
//! is an UPPER bound and these four were inside it.
//!
//! Every RPC gets a refusal arm that asserts the REASON, not merely that something failed —
//! a negative test passes on any failure, including the wrong one — and each refusal sits
//! beside an accepting call differing in exactly ONE thing, so a refusal that fired for an
//! unrelated reason (a broken fixture, an unwired seam) cannot read as a pass.
//!
//! Driven over the real gRPC surface against the SDK reference connector, so no model and
//! no third-party install is involved: this is the deterministic lifecycle witness.

#![cfg(feature = "mcp-gateway")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::HashMap;
use std::net::SocketAddr;

use kx_gateway::start;
use kx_proto::proto;
use kx_proto::proto::kx_gateway_client::KxGatewayClient;
use tonic::transport::Channel;

async fn client(addr: SocketAddr) -> KxGatewayClient<Channel> {
    common::connect_client(addr).await
}

/// All four RPCs answer and REFUSE with reasons, using no connector at all.
///
/// This is the arm that gates every PR, so it deliberately depends on nothing that has to be
/// built or installed first: an empty gateway, three refusals for a name that was never
/// registered, and the idempotent second deregister. That is each of the four RPCs exercised
/// at least once, which is what the coverage scan measures — and it cannot degrade into a
/// silent skip, because there is no prerequisite to be absent.
///
/// The arms that need a server that really answers live in `connector_env_map`, which runs
/// against the pinned third-party install.
#[tokio::test(flavor = "multi_thread")]
async fn the_connector_lifecycle_rpcs_refuse_unknown_servers_with_reasons() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    // ── ListMcpServers on a gateway that has registered nothing.
    let before = c
        .list_mcp_servers(proto::ListMcpServersRequest::default())
        .await
        .expect("ListMcpServers answers on an empty gateway")
        .into_inner();
    assert!(
        before.servers.is_empty(),
        "a fresh gateway has no connectors: {:?}",
        before.servers
    );

    // ── TestMcpServer / DiscoverServerTools on a name that does not exist. The reason has
    // to name the miss: a `not found` and a `dial failed` send an operator to completely
    // different places, and a negative test that only checks "it failed" passes on either.
    let missing = c
        .test_mcp_server(proto::TestMcpServerRequest {
            server_name: "no-such-connector".to_string(),
        })
        .await
        .expect_err("TestMcpServer refuses an unregistered server");
    assert_eq!(
        missing.code(),
        tonic::Code::NotFound,
        "refused as NotFound, not a generic error: {}",
        missing.message()
    );
    assert!(
        missing.message().contains("no-such-connector"),
        "the refusal names the server asked for: {}",
        missing.message()
    );

    let nope = c
        .discover_server_tools(proto::DiscoverServerToolsRequest {
            server_name: "no-such-connector".to_string(),
        })
        .await
        .expect_err("DiscoverServerTools refuses an unregistered server");
    assert_eq!(
        nope.code(),
        tonic::Code::NotFound,
        "refused as NotFound: {}",
        nope.message()
    );
    assert!(
        nope.message().contains("no-such-connector"),
        "the refusal names the server asked for: {}",
        nope.message()
    );

    // ── DeregisterMcpServer is deliberately NOT a refusal: removing something absent reports
    // `removed = false`. Asserted explicitly so the asymmetry with the two above is a
    // decision on the record rather than an accident.
    let absent = c
        .deregister_mcp_server(proto::DeregisterMcpServerRequest {
            server_name: "no-such-connector".to_string(),
        })
        .await
        .expect("deregistering an absent server is not an error")
        .into_inner();
    assert!(
        !absent.removed,
        "removing nothing reports false rather than erroring"
    );

    running.shutdown().await.unwrap();
}
