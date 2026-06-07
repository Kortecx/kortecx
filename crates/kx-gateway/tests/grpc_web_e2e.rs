//! R9.5 — gRPC-web shim + deny-by-default CORS, end-to-end.
//!
//! Proves the browser-facing wire over a real `kx serve` gateway:
//!   1. a gRPC-web unary RPC (HTTP/1.1, `application/grpc-web+proto`) round-trips
//!      through the `GrpcWebLayer` to a real RPC and returns `grpc-status: 0`;
//!   2. CORS is **deny-by-default** — an unlisted origin (and the no-`--cors-origin`
//!      default) gets NO `access-control-allow-origin`, a listed origin gets it
//!      echoed back (never `*`);
//!   3. an `OPTIONS` preflight is answered by the CORS layer WITHOUT the bearer
//!      auth interceptor (a preflight is not an auth oracle), even on a deny-all
//!      gateway;
//!   4. native HTTP/2 gRPC clients are UNAFFECTED (`accept_http1(true)` is
//!      additive) — a full `SubmitRun → Committed` still works through the layers.
//!
//! The gRPC-web/CORS requests are issued as raw HTTP/1.1 over a `TcpStream` (no new
//! client dep): the 5-byte gRPC length-prefixed frame is trivial for the
//! empty-request `ListSignatures` RPC, and the response is asserted on the status
//! line + CORS headers + the gRPC trailer.

#![cfg(feature = "embedded-worker")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Duration;

use kx_gateway::{start, GatewayConfig};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

mod common;

/// The unary RPC we exercise over gRPC-web: an EMPTY request, so its gRPC frame is
/// just the 5-byte header `00 00 00 00 00` (flag 0 + big-endian length 0).
const LIST_SIGNATURES_PATH: &str = "/kortecx.v1.KxGateway/ListSignatures";

/// A dev-allow-local gateway config rooted at `dir`, with the given CORS allowlist.
fn cfg_with_cors(dir: &TempDir, cors_origins: Vec<String>) -> GatewayConfig {
    let mut cfg = common::gateway_config(dir, true, HashMap::new());
    cfg.cors_origins = cors_origins;
    cfg
}

/// Send one raw HTTP/1.1 request to `addr` and return the full response bytes.
///
/// Reads until the peer half-closes or a short inter-read timeout elapses (hyper
/// keep-alive holds the socket open, so the timeout is what bounds a single
/// response). Adequate for the small gRPC-web / preflight responses asserted here.
async fn http1_request(addr: SocketAddr, request: &[u8]) -> Vec<u8> {
    // `start()` returns before the listener binds (the serve task spawns), so retry
    // the connect briefly — mirrors `common::connect_client`.
    let mut stream = {
        let mut last = None;
        for _ in 0..200 {
            match TcpStream::connect(addr).await {
                Ok(s) => {
                    last = Some(s);
                    break;
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(10)).await,
            }
        }
        last.expect("connect to gateway (server bound)")
    };
    stream.write_all(request).await.expect("write request");
    stream.flush().await.expect("flush");

    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        match tokio::time::timeout(Duration::from_millis(400), stream.read(&mut chunk)).await {
            Ok(Ok(0)) => break,                              // peer closed
            Ok(Ok(n)) => buf.extend_from_slice(&chunk[..n]), // got bytes, keep reading
            Ok(Err(e)) => panic!("read error: {e}"),
            Err(_) => break, // inter-read timeout: the response is complete
        }
        // A complete grpc-web/preflight response is small; cap to avoid a runaway.
        if buf.len() > 1 << 20 {
            break;
        }
    }
    buf
}

/// Build a raw HTTP/1.1 request: ASCII headers + an optional binary body.
fn http1_message(
    method: &str,
    path: &str,
    addr: SocketAddr,
    headers: &[(&str, &str)],
    body: &[u8],
) -> Vec<u8> {
    let mut req = format!("{method} {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n");
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    req.push_str(&format!("Content-Length: {}\r\n\r\n", body.len()));
    let mut bytes = req.into_bytes();
    bytes.extend_from_slice(body);
    bytes
}

/// Lowercase the header block (everything up to the first blank line) for
/// case-insensitive header assertions; the body is asserted separately.
fn lower(resp: &[u8]) -> String {
    String::from_utf8_lossy(resp).to_lowercase()
}

#[tokio::test]
async fn grpc_web_unary_round_trips() {
    let dir = TempDir::new().unwrap();
    let running = start(cfg_with_cors(&dir, vec!["http://localhost:5173".into()]))
        .await
        .unwrap();
    let addr = running.local_addr();

    // The empty ListSignatures request: one gRPC frame, header-only (length 0).
    let frame = [0u8, 0, 0, 0, 0];
    let req = http1_message(
        "POST",
        LIST_SIGNATURES_PATH,
        addr,
        &[
            ("content-type", "application/grpc-web+proto"),
            ("x-grpc-web", "1"),
        ],
        &frame,
    );
    let resp = http1_request(addr, &req).await;
    let text = lower(&resp);

    assert!(
        text.starts_with("http/1.1 200"),
        "gRPC-web unary returns HTTP 200; got:\n{text}"
    );
    assert!(
        text.contains("content-type: application/grpc-web"),
        "the response is gRPC-web framed; got:\n{text}"
    );
    // A successful RPC carries the gRPC trailer grpc-status:0 (tonic-web encodes
    // trailers in a trailer frame as ASCII). Normalize spaces before matching.
    let normalized = text.replace(' ', "");
    assert!(
        normalized.contains("grpc-status:0"),
        "the RPC succeeded end-to-end (grpc-status:0); got:\n{text}"
    );

    running.shutdown().await.unwrap();
}

#[tokio::test]
async fn cors_allows_a_listed_origin() {
    let dir = TempDir::new().unwrap();
    let origin = "http://localhost:5173";
    let running = start(cfg_with_cors(&dir, vec![origin.into()]))
        .await
        .unwrap();
    let addr = running.local_addr();

    let req = http1_message(
        "OPTIONS",
        LIST_SIGNATURES_PATH,
        addr,
        &[
            ("origin", origin),
            ("access-control-request-method", "POST"),
            (
                "access-control-request-headers",
                "content-type,x-grpc-web,authorization",
            ),
        ],
        &[],
    );
    let text = lower(&http1_request(addr, &req).await);

    assert!(
        text.contains(&format!("access-control-allow-origin: {origin}")),
        "a listed origin is echoed in the preflight grant; got:\n{text}"
    );
    // Never a wildcard.
    assert!(
        !text.contains("access-control-allow-origin: *"),
        "the grant is the explicit origin, never a wildcard; got:\n{text}"
    );

    running.shutdown().await.unwrap();
}

#[tokio::test]
async fn cors_denies_an_unlisted_origin() {
    let dir = TempDir::new().unwrap();
    // Allowlist one origin; request from a DIFFERENT one.
    let running = start(cfg_with_cors(&dir, vec!["http://localhost:5173".into()]))
        .await
        .unwrap();
    let addr = running.local_addr();

    let req = http1_message(
        "OPTIONS",
        LIST_SIGNATURES_PATH,
        addr,
        &[
            ("origin", "https://evil.example.com"),
            ("access-control-request-method", "POST"),
        ],
        &[],
    );
    let text = lower(&http1_request(addr, &req).await);

    assert!(
        !text.contains("access-control-allow-origin"),
        "an unlisted origin gets NO cross-origin grant; got:\n{text}"
    );

    running.shutdown().await.unwrap();
}

#[tokio::test]
async fn cors_deny_by_default_when_allowlist_empty() {
    let dir = TempDir::new().unwrap();
    // No --cors-origin at all (the default posture).
    let running = start(cfg_with_cors(&dir, Vec::new())).await.unwrap();
    let addr = running.local_addr();

    let req = http1_message(
        "OPTIONS",
        LIST_SIGNATURES_PATH,
        addr,
        &[
            ("origin", "http://localhost:5173"),
            ("access-control-request-method", "POST"),
        ],
        &[],
    );
    let text = lower(&http1_request(addr, &req).await);

    assert!(
        !text.contains("access-control-allow-origin"),
        "deny-by-default: with no allowlist, no browser origin is granted; got:\n{text}"
    );

    running.shutdown().await.unwrap();
}

#[tokio::test]
async fn preflight_is_not_an_auth_oracle() {
    // A DENY-ALL gateway (no --dev-allow-local, no tokens): every real RPC is
    // refused, but a CORS preflight must still be answered by the CORS layer
    // (outermost, before the auth interceptor) — proving the preflight cannot be
    // used to probe authentication state.
    let dir = TempDir::new().unwrap();
    let origin = "http://localhost:5173";
    let mut cfg = common::gateway_config(&dir, false, HashMap::new()); // deny-all
    cfg.cors_origins = vec![origin.into()];
    let running = start(cfg).await.unwrap();
    let addr = running.local_addr();

    let req = http1_message(
        "OPTIONS",
        LIST_SIGNATURES_PATH,
        addr,
        &[
            ("origin", origin),
            ("access-control-request-method", "POST"),
        ],
        &[],
    );
    let text = lower(&http1_request(addr, &req).await);

    // The preflight is granted (2xx) and carries the allow-origin — it never went
    // through the bearer interceptor, so it is not a 401/identity oracle.
    assert!(
        text.starts_with("http/1.1 2"),
        "the preflight is answered by CORS (2xx), not the auth interceptor; got:\n{text}"
    );
    assert!(
        text.contains(&format!("access-control-allow-origin: {origin}")),
        "the preflight grant is independent of auth; got:\n{text}"
    );

    running.shutdown().await.unwrap();
}

#[tokio::test]
async fn native_h2_grpc_still_works_through_the_layers() {
    // Back-compat: `accept_http1(true)` + the gRPC-web/CORS layers must not break a
    // native HTTP/2 tonic client. A full SubmitRun → Committed still works.
    let dir = TempDir::new().unwrap();
    let running = start(cfg_with_cors(&dir, vec!["http://localhost:5173".into()]))
        .await
        .unwrap();
    let addr = running.local_addr();

    let mut client = common::connect_client(addr).await;
    let instance_id = common::submit_pure_run(&mut client, 0x42).await;
    let (_mote_id, _result_ref) = common::await_committed(&mut client, &instance_id).await;

    running.shutdown().await.unwrap();
}
