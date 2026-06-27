//! T-CONNECTOR-AUTOGRANT: the `CallMcpTool` operator-diagnostic live-fire — register
//! the SDK reference connector via the `RegisterMcpServer` RPC, then fire one of its
//! tools through `CallMcpTool` and observe the real result. This is the DETERMINISTIC
//! (model-free) cross-surface witness that the dialed-connector firing path works
//! end-to-end through the broker — the same broker the agentic loop uses — so it
//! proves the connector + broker wiring independently of model nondeterminism.
//!
//! SN-8 is re-enforced server-side: a single-grant warrant is synthesized from the
//! tool's OWN registered scopes, and the args are validated against its inputSchema.
//! NOT a durable agentic effect (no journal fact) — an operator diagnostic, like
//! `TestMcpServer` / `DiscoverServerTools`.
//!
//! Needs the reference connector bin (`cargo build -p kx-extension-sdk`, or
//! `KX_CONNECTOR_EXAMPLE_PATH`); runtime-skips if absent. No model required.

#![cfg(feature = "mcp-gateway")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use kx_gateway::start;
use kx_proto::proto;
use kx_proto::proto::kx_gateway_client::KxGatewayClient;
use tonic::transport::Channel;

/// Locate the SDK reference connector bin (`kx-connector-example`).
fn reference_connector_bin() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("KX_CONNECTOR_EXAMPLE_PATH") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    let exe = std::env::current_exe().ok()?;
    for ancestor in exe.ancestors() {
        if ancestor.file_name().is_some_and(|n| n == "target") {
            for profile in ["debug", "release"] {
                let candidate = ancestor.join(profile).join("kx-connector-example");
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

async fn client(addr: SocketAddr) -> KxGatewayClient<Channel> {
    let endpoint = format!("http://{addr}");
    for _ in 0..100 {
        if let Ok(c) = KxGatewayClient::connect(endpoint.clone()).await {
            return c;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("client connects to the gateway at {endpoint}");
}

#[tokio::test(flavor = "multi_thread")]
async fn call_mcp_tool_fires_a_dialed_connector_tool() {
    let Some(conn_bin) = reference_connector_bin() else {
        eprintln!(
            "skipping: reference connector not built — run `cargo build -p kx-extension-sdk` \
             (or set KX_CONNECTOR_EXAMPLE_PATH)"
        );
        return;
    };

    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    // Dial the external reference connector (exposes refconn/echo + refconn/reverse).
    let reg = c
        .register_mcp_server(proto::RegisterMcpServerRequest {
            server_name: "refconn".to_string(),
            transport: "stdio".to_string(),
            endpoint: conn_bin.to_string_lossy().into_owned(),
            args: vec![],
            tls_required: false,
            credential_ref: String::new(),
            session_mode: "stateless".to_string(),
        })
        .await
        .expect("register the reference connector")
        .into_inner();
    assert_eq!(reg.health, "connected", "the connector dials cleanly");
    assert!(reg.discovered >= 2, "echo + reverse discovered");

    // Fire the UNIQUE `reverse` tool (no collision) with a real arg — the
    // deterministic positive control for the dialed-connector firing path.
    let resp = c
        .call_mcp_tool(proto::CallMcpToolRequest {
            server_name: "refconn".to_string(),
            remote_name: "reverse".to_string(),
            args_json: r#"{"text":"pong"}"#.to_string(),
        })
        .await
        .expect("CallMcpTool reaches the gateway")
        .into_inner();
    assert!(resp.ok, "the dialed tool fired (error: {})", resp.error);
    assert!(
        resp.result_json.contains("gnop"),
        "reverse('pong') -> 'gnop' is in the result: {}",
        resp.result_json
    );

    // SN-8 / fail-closed: an unregistered tool is a structured error, never a fire.
    let missing = c
        .call_mcp_tool(proto::CallMcpToolRequest {
            server_name: "refconn".to_string(),
            remote_name: "does-not-exist".to_string(),
            args_json: "{}".to_string(),
        })
        .await
        .expect("CallMcpTool reaches the gateway")
        .into_inner();
    assert!(!missing.ok, "an unregistered tool does not fire");
    assert!(
        !missing.error.is_empty(),
        "the refusal carries a diagnostic"
    );

    // Schema fail-closed: `reverse` requires a string `text`; a wrong-typed arg is
    // refused BEFORE the connector is dialed (the inputSchema gate).
    let bad_args = c
        .call_mcp_tool(proto::CallMcpToolRequest {
            server_name: "refconn".to_string(),
            remote_name: "reverse".to_string(),
            args_json: r#"{"text":123}"#.to_string(),
        })
        .await
        .expect("CallMcpTool reaches the gateway")
        .into_inner();
    assert!(!bad_args.ok, "a schema-invalid arg does not fire");

    running.shutdown().await.unwrap();
}

/// The STATEFUL firing posture fires too — deterministic (model-free) proof of the
/// reused-session path (the live stateful witness is subject to single-process Metal
/// flakiness; this isolates the firing path from the model entirely).
#[tokio::test(flavor = "multi_thread")]
async fn call_mcp_tool_fires_a_stateful_connector() {
    let Some(conn_bin) = reference_connector_bin() else {
        eprintln!("skipping: reference connector not built (cargo build -p kx-extension-sdk)");
        return;
    };
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    c.register_mcp_server(proto::RegisterMcpServerRequest {
        server_name: "scon".to_string(),
        transport: "stdio".to_string(),
        endpoint: conn_bin.to_string_lossy().into_owned(),
        args: vec![],
        tls_required: false,
        credential_ref: String::new(),
        session_mode: "stateful".to_string(),
    })
    .await
    .expect("register the stateful connector")
    .into_inner();

    // Fire twice on the SAME server — a stateful connector reuses one live session, so
    // a second fire must also succeed (exercises the session-reuse path).
    for text in ["pong", "kortecx"] {
        let resp = c
            .call_mcp_tool(proto::CallMcpToolRequest {
                server_name: "scon".to_string(),
                remote_name: "reverse".to_string(),
                args_json: format!(r#"{{"text":"{text}"}}"#),
            })
            .await
            .expect("CallMcpTool reaches the gateway")
            .into_inner();
        let reversed: String = text.chars().rev().collect();
        assert!(
            resp.ok && resp.result_json.contains(&reversed),
            "stateful fire of reverse('{text}') -> '{reversed}': ok={} result={} error={}",
            resp.ok,
            resp.result_json,
            resp.error
        );
    }

    running.shutdown().await.unwrap();
}
