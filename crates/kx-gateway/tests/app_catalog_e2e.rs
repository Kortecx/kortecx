//! POC-4 App-catalog end-to-end over a REAL bound tonic port. Drives the three
//! additive RPCs (`SaveApp` / `ListApps` / `GetApp`) through the live gateway +
//! the `apps.db` host store, proving BOTH halves of the seam deterministically
//! (GR16 #5 — never rely on the live model to cover a cross-component seam):
//!
//! - **save → list → get round trip**: the canonical envelope is stored, surfaced
//!   in the catalog summary, and read back byte-identically; `app_ref` is
//!   server-derived; an identical re-save re-reports `deduplicated`.
//! - **cross-party isolation** (two auth-token parties): Bob cannot see Alice's
//!   App (uniform not-found / empty list — no cross-party oracle); Bob saving the
//!   same handle makes BOB's OWN row, never mutates Alice's.
//! - **bad envelope ⇒ `InvalidArgument`**: a non-envelope payload is refused at the
//!   boundary (the host validates — the envelope carries no authority, SN-8).

#![cfg(feature = "embedded-worker")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::HashMap;
use std::net::SocketAddr;

use kx_proto::proto;
use kx_proto::proto::kx_gateway_client::KxGatewayClient;
use tonic::transport::Channel;
use tonic::{Code, Request};

use kx_gateway::start;

async fn client(addr: SocketAddr) -> KxGatewayClient<Channel> {
    common::connect_client(addr).await
}

fn with_bearer<T>(payload: T, token: &str) -> Request<T> {
    let mut req = Request::new(payload);
    req.metadata_mut()
        .insert("authorization", format!("Bearer {token}").parse().unwrap());
    req
}

fn two_party_tokens() -> HashMap<String, String> {
    HashMap::from([
        ("tok-alice".to_string(), "alice@acme".to_string()),
        ("tok-bob".to_string(), "bob@acme".to_string()),
    ])
}

/// A valid canonical `kortecx.app/v1` envelope authored via the kx-app type crate
/// — one agentic `@`-step granting the bundled echo tool.
fn app_envelope(name: &str) -> Vec<u8> {
    let blueprint = serde_json::json!({
        "seed": 0,
        "steps": [{
            "kind": "model",
            "prompt": "Use the echo tool.",
            "tool_contract": { "mcp-echo/echo": "1" }
        }]
    });
    let mut env = kx_app::AppEnvelope::new(name, blueprint);
    env.description = "demo app".to_string();
    env.tags = vec!["demo".to_string()];
    env.to_canonical_json().unwrap()
}

#[tokio::test]
async fn save_list_get_round_trip_and_dedup() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, false, two_party_tokens()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    let envelope = app_envelope("echo-app");
    let saved = c
        .save_app(with_bearer(
            proto::SaveAppRequest {
                handle: "team/apps/echo".into(),
                envelope_json: envelope.clone(),
            },
            "tok-alice",
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(!saved.deduplicated);
    assert_eq!(saved.handle, "team/apps/echo");
    assert_eq!(
        saved.app_ref.len(),
        16,
        "app_ref is the 16B server-derived id"
    );

    // list surfaces the summary.
    let listed = c
        .list_apps(with_bearer(
            proto::ListAppsRequest {
                limit: 0,
                after_handle: String::new(),
            },
            "tok-alice",
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(listed.apps.len(), 1);
    let summary = &listed.apps[0];
    assert_eq!(summary.handle, "team/apps/echo");
    assert_eq!(summary.name, "echo-app");
    assert_eq!(summary.step_count, 1);
    assert_eq!(summary.tags, vec!["demo".to_string()]);

    // get reads the canonical envelope back byte-identically.
    let got = c
        .get_app(with_bearer(
            proto::GetAppRequest {
                handle: "team/apps/echo".into(),
            },
            "tok-alice",
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(got.found);
    assert_eq!(
        got.envelope_json, envelope,
        "envelope round-trips byte-identically"
    );
    assert_eq!(got.summary.unwrap().app_ref, saved.app_ref);

    // identical re-save dedups (content-addressed identity).
    let again = c
        .save_app(with_bearer(
            proto::SaveAppRequest {
                handle: "team/apps/echo".into(),
                envelope_json: envelope,
            },
            "tok-alice",
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(again.deduplicated);
    assert_eq!(again.app_ref, saved.app_ref);
}

#[tokio::test]
async fn cross_party_isolation_uniform_not_found() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, false, two_party_tokens()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    c.save_app(with_bearer(
        proto::SaveAppRequest {
            handle: "team/apps/secret".into(),
            envelope_json: app_envelope("secret"),
        },
        "tok-alice",
    ))
    .await
    .unwrap();

    // Bob cannot see Alice's App (uniform not-found, empty list — no oracle).
    let bob_get = c
        .get_app(with_bearer(
            proto::GetAppRequest {
                handle: "team/apps/secret".into(),
            },
            "tok-bob",
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(!bob_get.found, "Bob cannot read Alice's App");
    let bob_list = c
        .list_apps(with_bearer(
            proto::ListAppsRequest {
                limit: 0,
                after_handle: String::new(),
            },
            "tok-bob",
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(bob_list.apps.is_empty(), "Bob lists none of Alice's");

    // Bob saving the SAME handle makes BOB's OWN row; Alice's is unchanged.
    c.save_app(with_bearer(
        proto::SaveAppRequest {
            handle: "team/apps/secret".into(),
            envelope_json: app_envelope("bobs-own"),
        },
        "tok-bob",
    ))
    .await
    .unwrap();
    let alice_get = c
        .get_app(with_bearer(
            proto::GetAppRequest {
                handle: "team/apps/secret".into(),
            },
            "tok-alice",
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(alice_get.found);
    assert_eq!(
        alice_get.summary.unwrap().name,
        "secret",
        "Alice's App is never mutated by Bob's same-handle save"
    );
}

#[tokio::test]
async fn bad_envelope_is_invalid_argument() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, false, two_party_tokens()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    let err = c
        .save_app(with_bearer(
            proto::SaveAppRequest {
                handle: "team/apps/bad".into(),
                envelope_json: b"{ not an envelope".to_vec(),
            },
            "tok-alice",
        ))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::InvalidArgument);
}
