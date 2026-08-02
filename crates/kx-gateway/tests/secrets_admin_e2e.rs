//! An operator stores a credential by NAME, a run uses it without the value ever
//! appearing, and revoking it stops the next run rather than quietly changing what it
//! sends.
//!
//! ## The scenario, named before the code (Rule 47)
//!
//! An operator stores a credential the runtime needs to reach an external service. They
//! list what they hold and see names and timestamps — never values. A run authenticates
//! with it. Then they revoke it, and the next run must STOP: an operator who deletes a
//! credential has said "this runtime may no longer use it", and a runtime that keeps
//! calling the third party without it is doing something they did not ask for.
//!
//! `list_secret_names` and `delete_secret` had no test caller at any layer — the only
//! non-generated references were the handler definitions and the CLI verb — and
//! `put_secret`'s three callers all discarded the result inside an ignore-gated file.
//!
//! ## The claim this file MEASURED rather than assumed
//!
//! The brief that scoped this work predicted that deleting a secret makes the next run
//! refuse rather than silently skip. The source said the opposite and documented it: the
//! HTTP path filtered an unresolvable credential out of its header list, and the
//! doc-comment called that deliberate — *the runtime never fabricates a credential; the
//! server fails its own auth.* Rather than trust either reading, a probe ran the
//! scenario and recorded what happened:
//!
//! ```text
//!   AFTER delete -> ok=false  error="… Other(\"MCP error -32001: unauthorized …\")"
//!   AFTER delete -> captures 4 -> 5   (a NEW capture means the request EGRESSED)
//!   AFTER delete -> saw_auth=false
//! ```
//!
//! The request left the process carrying no credential, and the only diagnosis was the
//! far end's generic rejection — which reads as *your token is wrong* and sends an
//! operator to the remote service's settings instead of their own store. That is a
//! failure that looks exactly like the safe state, so the transports now REFUSE a named
//! credential that does not resolve, before anything is sent, naming it. This file is
//! the guard on that.
//!
//! ## Where the guard had to go, which is not where it first went
//!
//! The refusal was first added to `HttpTransport::call` — the obvious home, and the
//! wrong copy. The output of the probe above did not move by a single character, which
//! is the signal. `HttpSession` carries its own cloned credentials and its own header
//! resolution, and the connector path dispatches through the SESSION. A guard that had
//! been written to assert the fix rather than to reproduce the incident would have been
//! green over a live defect.
//!
//! ```text
//!   cargo test -p kx-gateway --features mcp-gateway --test secrets_admin_e2e
//! ```

#![cfg(feature = "mcp-gateway")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::HashMap;
use std::net::SocketAddr;

use kx_gateway::start;
use kx_proto::proto;
use kx_proto::proto::kx_gateway_client::KxGatewayClient;
use tonic::transport::Channel;

const SERVER: &str = "fleet";
const SECRET: &str = "SECRETS_E2E_FLEET_TOKEN";

async fn client(addr: SocketAddr) -> KxGatewayClient<Channel> {
    common::connect_client(addr).await
}

async fn list_names(c: &mut KxGatewayClient<Channel>) -> Vec<proto::SecretName> {
    c.list_secret_names(proto::ListSecretNamesRequest {
        limit: 0,
        after_name: String::new(),
    })
    .await
    .expect("ListSecretNames reaches the gateway")
    .into_inner()
    .names
}

async fn put(c: &mut KxGatewayClient<Channel>, name: &str, value: &str) -> bool {
    c.put_secret(proto::PutSecretRequest {
        name: name.to_string(),
        value: value.to_string(),
    })
    .await
    .expect("PutSecret reaches the gateway")
    .into_inner()
    .stored
}

async fn delete(c: &mut KxGatewayClient<Channel>, name: &str) -> bool {
    c.delete_secret(proto::DeleteSecretRequest {
        name: name.to_string(),
    })
    .await
    .expect("DeleteSecret reaches the gateway")
    .into_inner()
    .removed
}

async fn call_get(c: &mut KxGatewayClient<Channel>) -> proto::CallMcpToolResponse {
    c.call_mcp_tool(proto::CallMcpToolRequest {
        server_name: SERVER.to_string(),
        remote_name: "get".to_string(),
        args_json: r#"{"vessel":"kestrel"}"#.to_string(),
    })
    .await
    .expect("CallMcpTool reaches the gateway")
    .into_inner()
}

/// ★ The governance view: what an operator stores, sees, and removes. `ListSecretNames`
/// carries names and timestamps and there is no field on the response that could carry a
/// value; deleting reports whether anything was there.
#[tokio::test(flavor = "multi_thread")]
async fn the_three_secret_rpcs_round_trip_and_the_listing_never_carries_a_value() {
    std::env::set_var("KX_SERVE_OLLAMA", "off");
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    // The precondition, asserted rather than assumed: an empty store. If the store were
    // pre-populated every assertion below would be about someone else's rows.
    assert!(
        list_names(&mut c).await.is_empty(),
        "a fresh runtime holds no secrets"
    );

    assert!(put(&mut c, "ALPHA", "value-alpha").await, "stored");
    assert!(put(&mut c, "BETA", "value-beta").await, "stored");

    let names = list_names(&mut c).await;
    assert_eq!(
        names.iter().map(|n| n.name.as_str()).collect::<Vec<_>>(),
        ["ALPHA", "BETA"],
        "both names are listed, in name order"
    );
    for row in &names {
        assert!(
            row.created_unix_ms > 0 && row.updated_unix_ms >= row.created_unix_ms,
            "each row carries usable timestamps: {row:?}"
        );
    }
    // The value cannot appear in the listing because the response type has no field for
    // one. Assert it over the encoded bytes so this stays true if a field is ever added.
    let encoded = format!("{names:?}");
    assert!(
        !encoded.contains("value-alpha") && !encoded.contains("value-beta"),
        "no stored value may appear in the governance view: {encoded}"
    );

    assert!(
        delete(&mut c, "ALPHA").await,
        "an existing secret is removed"
    );
    assert_eq!(
        list_names(&mut c)
            .await
            .iter()
            .map(|n| n.name.as_str())
            .collect::<Vec<_>>(),
        ["BETA"],
        "the deleted name is gone and its sibling is untouched"
    );
    // The ACCEPTING control for the assertion above, one variable changed: deleting the
    // same name again reports NOT-removed. Without it, `removed == true` would be
    // consistent with an implementation that always says true.
    assert!(
        !delete(&mut c, "ALPHA").await,
        "deleting an absent secret reports not-removed rather than a fabricated success"
    );
}

/// ★ A connector naming a credential that was never stored does not come up, and nothing
/// is sent while it fails.
///
/// The refusal lands at the DIAL, which is earlier than this test originally asserted —
/// it was written expecting registration to succeed and the first call to refuse. That is
/// the better place for it: the operator hears at registration, when they are looking,
/// rather than at the first run. The assertion follows the measurement.
#[tokio::test(flavor = "multi_thread")]
async fn a_connector_naming_an_unstored_credential_never_comes_up_and_sends_nothing() {
    let fleet = common::bench_http::BenchHttpServer::start();
    std::env::set_var("KX_SERVE_TOOL_HOST_ALLOWLIST", fleet.host());
    // The resolve chain is store-then-ENVIRONMENT. Leaving the env arm populated would
    // answer the question this test asks, from a source it is not manipulating.
    std::env::remove_var(SECRET);

    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    let sent_before = fleet.captured().len();
    let reg = c
        .register_mcp_server(proto::RegisterMcpServerRequest {
            server_name: SERVER.to_string(),
            transport: "http".to_string(),
            endpoint: fleet.url(),
            args: vec![],
            tls_required: false,
            credential_ref: SECRET.to_string(),
            session_mode: "stateless".to_string(),
        })
        .await
        .expect("register the HTTP connector")
        .into_inner();
    assert_eq!(
        reg.health, "unreachable",
        "a connector whose named credential is absent must not report itself connected"
    );
    assert_eq!(
        reg.discovered, 0,
        "and it discovers nothing — a half-registered connector with tools nobody can \
         fire is worse than one that plainly did not come up"
    );
    assert_eq!(
        fleet.captured().len(),
        sent_before,
        "nothing may be SENT while the credential is missing: an uncredentialed request \
         to a third party is not the call the operator authorised"
    );

    // The ACCEPTING CONTROL, one variable changed: store the credential and the IDENTICAL
    // registration comes up. Without it, "unreachable" would be consistent with a broken
    // fixture, a bad allowlist, or an endpoint that was never up.
    assert!(put(&mut c, SECRET, common::bench_http::BENCH_HTTP_BEARER).await);
    let ok = c
        .register_mcp_server(proto::RegisterMcpServerRequest {
            server_name: SERVER.to_string(),
            transport: "http".to_string(),
            endpoint: fleet.url(),
            args: vec![],
            tls_required: false,
            credential_ref: SECRET.to_string(),
            session_mode: "stateless".to_string(),
        })
        .await
        .expect("register the HTTP connector")
        .into_inner();
    assert_eq!(
        ok.health, "connected",
        "the same registration, with the credential stored, connects"
    );
}

/// ★ The scenario end to end: stored by name, used without the value appearing, revoked,
/// and the next call stops.
///
/// The accepting control is the SAME call before the revoke — it authenticates and
/// returns the record. Without it, "the call failed" after a delete would be consistent
/// with a connector that never worked at all.
#[tokio::test(flavor = "multi_thread")]
async fn revoking_a_credential_stops_the_next_call_instead_of_sending_it_uncredentialed() {
    let fleet = common::bench_http::BenchHttpServer::start();
    std::env::set_var("KX_SERVE_TOOL_HOST_ALLOWLIST", fleet.host());
    std::env::remove_var(SECRET);

    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    assert!(
        put(&mut c, SECRET, common::bench_http::BENCH_HTTP_BEARER).await,
        "the credential is stored through the RPC under test"
    );

    c.register_mcp_server(proto::RegisterMcpServerRequest {
        server_name: SERVER.to_string(),
        transport: "http".to_string(),
        endpoint: fleet.url(),
        args: vec![],
        tls_required: false,
        credential_ref: SECRET.to_string(),
        session_mode: "stateless".to_string(),
    })
    .await
    .expect("register the HTTP connector");

    // (1) THE ACCEPTING CONTROL. The credential resolves and the call authenticates.
    let before = call_get(&mut c).await;
    assert!(before.ok, "the credentialed call fires: {}", before.error);
    assert!(
        before.result_json.contains("Ilma Rask"),
        "the real record comes back: {}",
        before.result_json
    );
    assert!(
        fleet.captured().iter().any(|r| r.auth_ok),
        "the Authorization header reached the far end — asserted through a flag the \
         fixture records, so proving injection never puts the secret in an assertion"
    );
    // The value never appears in what the operator or the model can read.
    assert!(
        !before
            .result_json
            .contains(common::bench_http::BENCH_HTTP_BEARER)
            && !before.error.contains(common::bench_http::BENCH_HTTP_BEARER),
        "the credential's value must not appear in a tool result or a diagnostic"
    );

    // (2) The one variable: the operator revokes it.
    assert!(delete(&mut c, SECRET).await, "the credential is removed");
    let sent_before = fleet.captured().len();

    let after = call_get(&mut c).await;
    assert!(!after.ok, "the next call must not succeed after a revoke");
    assert!(
        after.error.contains(SECRET),
        "the refusal NAMES the revoked credential rather than surfacing the far end's \
         generic rejection; got {}",
        after.error
    );
    assert_eq!(
        fleet.captured().len(),
        sent_before,
        "NOTHING may be sent after the revoke. This is the assertion the whole file \
         exists for: before the fix a new capture appeared here with saw_auth=false, so \
         a revoked credential silently became an unauthenticated call to a third party"
    );
    assert!(
        !after.error.contains(common::bench_http::BENCH_HTTP_BEARER),
        "the refusal must name the credential without quoting its value"
    );
}
