//! D139 — the embedded web console, end to end against a real `start()`:
//! the third listener serves the compile-time-embedded SPA (root, the SPA
//! fallback, immutable assets, 404/405, HEAD) and the gRPC-web CORS allowlist
//! auto-extends with the console's OWN loopback origin.
//!
//! Compiled only under the `console` feature (the ui CI job builds `ui/dist`
//! first, then runs `cargo test -p kx-gateway --features console`).

#![cfg(all(feature = "console", feature = "embedded-worker"))]
#![allow(clippy::unwrap_used, clippy::expect_used)]
// Vite emits lowercase asset names by construction — the suffix probe below is
// a test convenience, not a filesystem-semantics claim.
#![allow(clippy::case_sensitive_file_extension_comparisons)]

mod common;

use std::collections::HashMap;

use kx_gateway::{start, ConsoleMode};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Issue one raw HTTP/1.1 request and return (status line, headers, body).
/// Hand-rolled on purpose: no new dev-deps, and it proves the console answers
/// plain socket-level HTTP (exactly what a browser sends).
async fn raw_http(
    addr: std::net::SocketAddr,
    method: &str,
    path: &str,
) -> (String, String, Vec<u8>) {
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let req = format!("{method} {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    let split = buf
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("a complete HTTP response head");
    let head = String::from_utf8_lossy(&buf[..split]).to_string();
    let body = buf[split + 4..].to_vec();
    let (status, headers) = head.split_once("\r\n").unwrap_or((head.as_str(), ""));
    (status.to_string(), headers.to_ascii_lowercase(), body)
}

#[tokio::test]
async fn the_console_serves_the_spa_and_extends_cors() {
    let dir = TempDir::new().unwrap();
    let mut cfg = common::gateway_config(&dir, true, HashMap::new());
    cfg.console_listen = ConsoleMode::Listen("127.0.0.1:0".parse().unwrap());
    let running = start(cfg).await.unwrap();
    let console = running
        .console_local_addr()
        .expect("a console-feature build with Listen mode binds the console");

    // Root → the SPA entrypoint, revalidating.
    let (status, headers, body) = raw_http(console, "GET", "/").await;
    assert!(status.contains("200"), "{status}");
    assert!(headers.contains("content-type: text/html"), "{headers}");
    assert!(headers.contains("cache-control: no-cache"), "{headers}");
    assert!(headers.contains("x-content-type-options: nosniff"));
    let index_body = body;
    assert!(!index_body.is_empty());

    // A client-side route serves the IDENTICAL entrypoint bytes (SPA fallback).
    let (status, _, body) = raw_http(console, "GET", "/chat").await;
    assert!(status.contains("200"), "{status}");
    assert_eq!(body, index_body, "/chat falls back to index.html");

    // A hashed asset (from the embedded manifest) is immutable + typed.
    let js = String::from_utf8_lossy(&index_body)
        .split('"')
        .find(|s| s.starts_with("/assets/") && s.ends_with(".js"))
        .expect("index.html references a hashed js asset")
        .to_string();
    let (status, headers, body) = raw_http(console, "GET", &js).await;
    assert!(status.contains("200"), "{js}: {status}");
    assert!(
        headers.contains("content-type: application/javascript"),
        "{headers}"
    );
    assert!(headers.contains("immutable"), "{headers}");
    assert!(!body.is_empty());

    // An extension-bearing miss is a real 404 (never the SPA page).
    let (status, _, _) = raw_http(console, "GET", "/definitely-not-here.js").await;
    assert!(status.contains("404"), "{status}");

    // HEAD answers headers-only; non-GET/HEAD is 405 with Allow.
    let (status, headers, body) = raw_http(console, "HEAD", "/").await;
    assert!(status.contains("200"), "{status}");
    assert!(headers.contains("content-type: text/html"));
    assert!(body.is_empty(), "HEAD has no body");
    let (status, headers, _) = raw_http(console, "POST", "/").await;
    assert!(status.contains("405"), "{status}");
    assert!(headers.contains("allow: get, head"), "{headers}");

    // The gRPC-web CORS allowlist auto-extended with the console's OWN origin:
    // a preflight from that origin is granted (mirrors tests/grpc_web_e2e.rs).
    let grpc = running.local_addr();
    let mut stream = tokio::net::TcpStream::connect(grpc).await.unwrap();
    let preflight = format!(
        "OPTIONS /kortecx.v1.KxGateway/GetProjection HTTP/1.1\r\nHost: {grpc}\r\n\
         Origin: http://127.0.0.1:{port}\r\n\
         Access-Control-Request-Method: POST\r\n\
         Access-Control-Request-Headers: content-type,x-grpc-web\r\n\
         Connection: close\r\n\r\n",
        port = console.port()
    );
    stream.write_all(preflight.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    let resp = String::from_utf8_lossy(&buf).to_ascii_lowercase();
    assert!(
        resp.contains(&format!(
            "access-control-allow-origin: http://127.0.0.1:{}",
            console.port()
        )),
        "the console's own origin is auto-granted on the gRPC-web port: {resp}"
    );

    running.shutdown().await.unwrap();
}

#[tokio::test]
async fn no_console_disables_the_listener() {
    let dir = TempDir::new().unwrap();
    let cfg = common::gateway_config(&dir, true, HashMap::new()); // Disabled in the fixture
    let running = start(cfg).await.unwrap();
    assert!(running.console_local_addr().is_none());
    running.shutdown().await.unwrap();
}
