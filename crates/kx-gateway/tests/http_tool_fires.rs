//! The model-free witness that an **HTTP-transport** MCP tool fires end to end and
//! returns its record.
//!
//! ## Why this file exists
//!
//! `bench-v1`'s `http` family scored **0 on both engines for its entire existence**, and
//! the failure was attributed in turn to the model, to the loop, and to result-carrying.
//! It was none of those. Nothing could say which, because **no deterministic test asserted
//! that an HTTP MCP tool fires at all**: `call_mcp_tool.rs` — the cross-surface live-fire
//! witness — registers `transport: "stdio"` at every site. So the one transport the
//! runtime uses to reach the outside world was proven only by a model-driven benchmark,
//! whose zero is a verdict and not a diagnosis.
//!
//! That is the same shape as every other absence found alongside it: a capability with no
//! coverage can be wholly dead indefinitely, because there is no number to move. `stdio`
//! cannot stand in for `http` here — it cannot fail an auth check, cannot paginate, and
//! cannot return a structured protocol error, which are precisely the paths a real
//! integration exercises.
//!
//! ## What it asserts
//!
//! Against the SAME fixture the benchmark uses, through the SAME `RegisterMcpServer` →
//! broker → `CallMcpTool` path the agentic loop fires through:
//!
//! 1. an HTTP connector registers and discovers its tools;
//! 2. the `Authorization` header, named by `credential_ref` and resolved at dispatch,
//!    actually ARRIVES — asserted without the secret entering the assertion;
//! 3. a tool call returns the real record;
//! 4. **pagination works**: page one is fetched with NO cursor, its `next_cursor` is read
//!    out of the result and passed back, and the fact on page TWO comes back. This is the
//!    property `http-paginated-roster` measures, and it was unreachable because an
//!    explicit `null` for an optional parameter was type-checked instead of read as
//!    absence;
//! 5. a structured JSON-RPC error is surfaced as a refusal carrying its diagnostic —
//!    the fail-closed control, so a green suite cannot mean "everything succeeds".

#![cfg(feature = "mcp-gateway")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Duration;

use kx_gateway::start;
use kx_proto::proto;
use kx_proto::proto::kx_gateway_client::KxGatewayClient;
use tonic::transport::Channel;

const SERVER: &str = "fleet";

async fn client(addr: SocketAddr) -> KxGatewayClient<Channel> {
    let endpoint = format!("http://{addr}");
    for _ in 0..50 {
        if let Ok(c) = KxGatewayClient::connect(endpoint.clone()).await {
            return c;
        }
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
    panic!("client connects to the gateway at {endpoint}");
}

/// Read `next_cursor` out of a tool result without interpreting the rest of it — the
/// exact move a paginating agent has to make.
fn next_cursor(result_json: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(result_json)
        .ok()?
        .get("next_cursor")?
        .as_str()
        .map(ToString::to_string)
}

#[tokio::test(flavor = "multi_thread")]
async fn an_http_connector_tool_fires_pages_and_surfaces_its_errors() {
    let fleet = common::bench_http::BenchHttpServer::start();

    // Both are read at REGISTRATION, not at call time: the allowlist is what turns a
    // loopback literal from refused into deliberately reachable, and the credential is
    // resolved from this NAME at dispatch (the secret never travels through the wire).
    std::env::set_var("KX_SERVE_TOOL_HOST_ALLOWLIST", fleet.host());
    std::env::set_var(
        common::bench_http::BENCH_HTTP_CRED_ENV,
        common::bench_http::BENCH_HTTP_BEARER,
    );

    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    // (1) Register over HTTP — the transport the runtime reaches the world with.
    let reg = c
        .register_mcp_server(proto::RegisterMcpServerRequest {
            server_name: SERVER.to_string(),
            transport: "http".to_string(),
            endpoint: fleet.url(),
            args: vec![],
            tls_required: false,
            credential_ref: common::bench_http::BENCH_HTTP_CRED_ENV.to_string(),
            session_mode: "stateless".to_string(),
        })
        .await
        .expect("register the HTTP connector")
        .into_inner();
    assert_eq!(
        reg.health, "connected",
        "the HTTP connector must dial cleanly"
    );
    assert!(
        reg.discovered >= 2,
        "page + get discovered, got {}",
        reg.discovered
    );

    // (2) The credential ARRIVED. Asserted via a flag the fixture records, so proving
    //     injection happened never puts the secret in an assertion.
    assert!(
        fleet.captured().iter().any(|r| r.auth_ok),
        "the Authorization header never reached the fixture — a credential that is named \
         but not injected makes every later call a 401, which reads as a tool failure"
    );

    // (3) A real call returns the real record.
    let got = c
        .call_mcp_tool(proto::CallMcpToolRequest {
            server_name: SERVER.to_string(),
            remote_name: "get".to_string(),
            args_json: r#"{"vessel":"kestrel"}"#.to_string(),
        })
        .await
        .expect("CallMcpTool reaches the gateway")
        .into_inner();
    assert!(got.ok, "the HTTP tool must fire (error: {})", got.error);
    assert!(
        got.result_json.contains("Ilma Rask"),
        "the record the server holds must come back: {}",
        got.result_json
    );

    // (4) PAGINATION, the property `http-paginated-roster` measures.
    //     Page one is fetched with NO cursor. An agent that has not paged yet has no
    //     cursor to send, and `{"cursor": null}` is how that is spelled in JSON — the
    //     form that was type-checked into a refusal, making page one unrepresentable and
    //     pagination structurally impossible. Both spellings must behave identically.
    for (spelling, args) in [("omitted", "{}"), ("explicit null", r#"{"cursor":null}"#)] {
        let page1 = c
            .call_mcp_tool(proto::CallMcpToolRequest {
                server_name: SERVER.to_string(),
                remote_name: "page".to_string(),
                args_json: args.to_string(),
            })
            .await
            .expect("CallMcpTool reaches the gateway")
            .into_inner();
        assert!(
            page1.ok,
            "page one with a {spelling} cursor must fire — an agent that has not paged \
             yet has no cursor, so refusing this makes pagination unreachable (error: {})",
            page1.error
        );

        let cursor = next_cursor(&page1.result_json).unwrap_or_else(|| {
            panic!(
                "page one must carry a next_cursor ({spelling}): {}",
                page1.result_json
            )
        });

        let page2 = c
            .call_mcp_tool(proto::CallMcpToolRequest {
                server_name: SERVER.to_string(),
                remote_name: "page".to_string(),
                args_json: format!(r#"{{"cursor":"{cursor}"}}"#),
            })
            .await
            .expect("CallMcpTool reaches the gateway")
            .into_inner();
        assert!(page2.ok, "page two must fire (error: {})", page2.error);
        assert!(
            page2.result_json.contains("TRESTLE-62"),
            "the fact that exists ONLY on page two must come back ({spelling}) — that is \
             what proves the chain actually paginated rather than guessed: {}",
            page2.result_json
        );
        assert!(
            !page1.result_json.contains("TRESTLE-62"),
            "page one must NOT already carry the page-two fact, or this asserts nothing"
        );
    }

    // (5) The fail-closed control. A structured JSON-RPC error is a REFUSAL carrying a
    //     diagnostic, not a fire and not a silent empty result — otherwise every
    //     assertion above could pass on a server that answers everything.
    let missing = c
        .call_mcp_tool(proto::CallMcpToolRequest {
            server_name: SERVER.to_string(),
            remote_name: "get".to_string(),
            args_json: r#"{"vessel":"no-such-vessel"}"#.to_string(),
        })
        .await
        .expect("CallMcpTool reaches the gateway")
        .into_inner();
    assert!(!missing.ok, "an unknown vessel must not report success");
    assert!(
        !missing.error.is_empty(),
        "the refusal must carry a diagnostic — a bare failure tells a caller nothing \
         about whether to change an argument or give up"
    );
}
