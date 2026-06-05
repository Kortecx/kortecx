//! R5 — the WebSocket `StreamEvents` bridge (the browser live-tail surface),
//! end-to-end over a real bound WS port.
//!
//! - `ws_streams_committed_delta_as_json`: a committed delta arrives as a JSON
//!   text frame with lowercase-hex ids (the browser-ergonomic wire).
//! - `ws_handshake_denied_without_auth`: deny-all rejects the WS upgrade.
//! - `ws_token_auth_accepts_good_rejects_bad`: the SAME `PrincipalResolver` gates
//!   the handshake (valid bearer accepted, invalid rejected).
//! - `ws_missing_instance_is_rejected`: a malformed query is a handshake error.

#![cfg(feature = "embedded-worker")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::HashMap;
use std::time::Duration;

use futures_util::StreamExt;
use kx_gateway::start;
use serde_json::Value;
use tempfile::TempDir;
use tokio::time::timeout;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

use common::{await_committed, connect_client, gateway_config, submit_pure_run};

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0x0f) as usize] as char);
    }
    s
}

#[tokio::test]
async fn ws_streams_committed_delta_as_json() {
    let dir = TempDir::new().unwrap();
    let running = start(gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = connect_client(running.local_addr()).await;
    let instance = submit_pure_run(&mut c, 1).await;
    let (mote_id, _) = await_committed(&mut c, &instance).await;

    let url = format!(
        "ws://{}/v1/events?instance={}&since=0",
        running.ws_local_addr(),
        hex(&instance)
    );
    let (mut ws, _resp) = connect_async(url)
        .await
        .expect("dev-allow-local accepts the WS handshake");

    let saw = timeout(Duration::from_secs(5), async {
        while let Some(message) = ws.next().await {
            if let Ok(Message::Text(text)) = message {
                let frame: Value =
                    serde_json::from_str(&text).expect("each WS frame is valid JSON");
                if let Some(deltas) = frame["deltas"].as_array() {
                    for delta in deltas {
                        if delta["type"] == "committed" {
                            // The wire renders ids as 64-char lowercase hex.
                            assert_eq!(delta["mote_id"], hex(&mote_id));
                            assert_eq!(delta["mote_id"].as_str().unwrap().len(), 64);
                            assert_eq!(delta["nd_class"], "pure");
                            return true;
                        }
                    }
                }
            }
        }
        false
    })
    .await
    .expect("did not time out reading WS frames");
    assert!(
        saw,
        "the WS bridge delivered the committed delta as hex JSON"
    );

    running.shutdown().await.unwrap();
}

#[tokio::test]
async fn ws_handshake_denied_without_auth() {
    let dir = TempDir::new().unwrap();
    // Deny-all (no --dev-allow-local, no tokens).
    let running = start(gateway_config(&dir, false, HashMap::new()))
        .await
        .unwrap();
    let url = format!(
        "ws://{}/v1/events?instance={}&since=0",
        running.ws_local_addr(),
        "ab".repeat(16)
    );
    assert!(
        connect_async(url).await.is_err(),
        "deny-all rejects the WS handshake (Rule 8c — no silent open door)"
    );
    running.shutdown().await.unwrap();
}

#[tokio::test]
async fn ws_token_auth_accepts_good_rejects_bad() {
    let dir = TempDir::new().unwrap();
    let mut tokens = HashMap::new();
    tokens.insert("tok-good".to_string(), "alice@acme".to_string());
    let running = start(gateway_config(&dir, false, tokens)).await.unwrap();
    let base = format!(
        "ws://{}/v1/events?instance={}&since=0",
        running.ws_local_addr(),
        "ab".repeat(16)
    );

    // Valid bearer → the handshake (auth) passes (the stream then closes on the
    // unowned instance, but the AUTH gate accepted the upgrade).
    let mut good = base.clone().into_client_request().unwrap();
    good.headers_mut()
        .insert("authorization", "Bearer tok-good".parse().unwrap());
    assert!(
        connect_async(good).await.is_ok(),
        "a valid bearer token passes the WS handshake"
    );

    // Invalid bearer → rejected at the handshake (uniform, no oracle).
    let mut bad = base.into_client_request().unwrap();
    bad.headers_mut()
        .insert("authorization", "Bearer nope".parse().unwrap());
    assert!(
        connect_async(bad).await.is_err(),
        "an invalid bearer token is rejected at the WS handshake"
    );

    running.shutdown().await.unwrap();
}

#[tokio::test]
async fn ws_missing_instance_is_rejected() {
    let dir = TempDir::new().unwrap();
    let running = start(gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    // No ?instance → a malformed-query handshake rejection.
    let url = format!("ws://{}/v1/events?since=0", running.ws_local_addr());
    assert!(
        connect_async(url).await.is_err(),
        "a missing ?instance is a handshake error"
    );
    running.shutdown().await.unwrap();
}
