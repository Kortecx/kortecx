//! W1a (T-OBS1 + T-OBS2) end-to-end: a REAL bound `kx serve` with both the
//! Prometheus `/metrics` endpoint AND the JSONL operator audit log enabled.
//!
//! - `metrics_endpoint_reflects_a_real_run`: scrape `/metrics` before a run
//!   (counters at zero), drive a PURE run to Committed, scrape again — the RED
//!   counters (`runs_registered_total`, `motes_committed_total`) advance, derived
//!   from the durable journal. `/health` answers, a bad method is 405.
//! - `serve_audit_log_captures_the_lifecycle`: the run writes
//!   `mote_dispatched` + `mote_committed` JSONL lines (the committed Mote's hex id),
//!   flushed deterministically on graceful shutdown.

#![cfg(feature = "embedded-worker")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use kx_gateway::{start, GatewayConfig};
use kx_proto::proto;
use kx_proto::proto::kx_gateway_client::KxGatewayClient;
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tonic::transport::Channel;

/// A dev config with the observability surfaces enabled: an ephemeral `/metrics`
/// listener + a JSONL audit log at `audit_path`.
fn obs_config(dir: &TempDir, audit_path: PathBuf) -> GatewayConfig {
    let mut cfg = common::gateway_config(dir, true, HashMap::new());
    cfg.metrics_listen = Some("127.0.0.1:0".parse().unwrap());
    cfg.audit_log = Some(audit_path);
    cfg
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

/// One raw HTTP/1.1 GET (status line, lowercased headers, body) — no new dev-deps
/// (mirrors `console_e2e::raw_http`); proves the endpoint answers plain HTTP, which
/// is exactly what a Prometheus scraper sends.
async fn raw_get(addr: SocketAddr, method: &str, path: &str) -> (String, String, String) {
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
    let body = String::from_utf8_lossy(&buf[split + 4..]).to_string();
    let (status, headers) = head.split_once("\r\n").unwrap_or((head.as_str(), ""));
    (status.to_string(), headers.to_ascii_lowercase(), body)
}

/// Extract the integer value of a single-sample Prometheus counter line.
fn metric_value(body: &str, name: &str) -> u64 {
    for line in body.lines() {
        if line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix(name) {
            // The sample is `name <value>` (no labels on these single-sample counters).
            if let Some(v) = rest.split_whitespace().next() {
                if let Ok(n) = v.parse::<u64>() {
                    return n;
                }
            }
        }
    }
    panic!("metric {name} not found in /metrics body:\n{body}");
}

async fn drive_pure_run(c: &mut KxGatewayClient<Channel>, seed: u8) -> ([u8; 32], Vec<u8>) {
    let mote = common::pure_mote(seed, &[]);
    let warrant = common::pure_warrant();
    let handle = c
        .submit_run(proto::SubmitRunRequest {
            recipe_fingerprint: vec![0x5a; 32],
            motes: vec![proto::SubmitMoteSpec {
                mote: Some(mote.into()),
                warrant: Some(warrant.into()),
                accept_at_least_once: false,
                react_seed: false,
            }],
        })
        .await
        .unwrap()
        .into_inner();
    let instance_id = handle.instance_id.clone();
    for _ in 0..100 {
        let view = c
            .get_projection(proto::GetProjectionRequest {
                instance_id: instance_id.clone(),
                at_seq: None,
            })
            .await
            .unwrap()
            .into_inner();
        if let Some(m) = view
            .motes
            .iter()
            .find(|m| m.state == proto::MoteSnapshotState::Committed as i32)
        {
            return (m.mote_id.clone().try_into().unwrap(), instance_id);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the submitted Mote never reached Committed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn metrics_endpoint_reflects_a_real_run() {
    let dir = TempDir::new().unwrap();
    let audit = dir.path().join("audit.jsonl");
    let running = start(obs_config(&dir, audit)).await.unwrap();
    let metrics = running
        .metrics_local_addr()
        .expect("--metrics-listen binds the /metrics endpoint");
    let mut c = client(running.local_addr()).await;

    // (1) Before any run: the endpoint answers 200 text/plain with zeroed counters.
    let (status, headers, body) = raw_get(metrics, "GET", "/metrics").await;
    assert!(status.contains("200"), "{status}");
    assert!(headers.contains("text/plain"), "{headers}");
    assert!(body.contains("kortecx_up 1"));
    assert_eq!(metric_value(&body, "kortecx_motes_committed_total"), 0);

    // (2) Drive a PURE run to Committed.
    drive_pure_run(&mut c, 1).await;

    // (3) The RED counters advance (derived from the durable journal; the fold tick
    //     runs every 250ms, so poll briefly for the post-commit snapshot).
    let mut committed = 0;
    for _ in 0..40 {
        let (_, _, body) = raw_get(metrics, "GET", "/metrics").await;
        committed = metric_value(&body, "kortecx_motes_committed_total");
        if committed >= 1 {
            assert!(
                metric_value(&body, "kortecx_runs_registered_total") >= 1,
                "a committed run also registered"
            );
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        committed >= 1,
        "kortecx_motes_committed_total never advanced"
    );

    // (4) Honest serving rules: /health pings, a non-GET is 405.
    let (status, _, body) = raw_get(metrics, "GET", "/health").await;
    assert!(status.contains("200"), "{status}");
    assert!(body.contains("ok"));
    let (status, _, _) = raw_get(metrics, "POST", "/metrics").await;
    assert!(status.contains("405"), "{status}");

    running.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn serve_audit_log_captures_the_lifecycle() {
    let dir = TempDir::new().unwrap();
    let audit_path = dir.path().join("audit.jsonl");
    let running = start(obs_config(&dir, audit_path.clone())).await.unwrap();
    let mut c = client(running.local_addr()).await;

    let (mote_id, _instance) = drive_pure_run(&mut c, 3).await;

    // Graceful shutdown flushes the buffered audit trail to disk (deterministic —
    // not the Drop-flush race).
    running.shutdown().await.unwrap();

    let log = std::fs::read_to_string(&audit_path).expect("the audit log was written");
    let mote_hex: String = mote_id.iter().map(|b| format!("{b:02x}")).collect();
    // The committed Mote appears as BOTH an admission (dispatched) and a durable
    // commit, each carrying its server-derived hex id (SN-8 — echoed, not recomputed).
    assert!(
        log.contains("\"type\":\"mote_dispatched\""),
        "audit log has the admission line:\n{log}"
    );
    assert!(
        log.contains("\"type\":\"mote_committed\""),
        "audit log has the commit line:\n{log}"
    );
    assert!(
        log.contains(&mote_hex),
        "audit log references the committed Mote {mote_hex}:\n{log}"
    );
    // Every non-empty line is a JSONL object carrying a wall-clock stamp (off the
    // digest — the kx-audit unit tests pin the full JSON shape; here we confirm the
    // serve trail's lines are well-formed objects with the audit envelope).
    for line in log.lines().filter(|l| !l.is_empty()) {
        assert!(
            line.starts_with('{') && line.ends_with('}'),
            "JSONL object: {line}"
        );
        assert!(
            line.contains("\"ts_ms\":"),
            "audit line carries a ts_ms: {line}"
        );
        assert!(
            line.contains("\"seq\":"),
            "audit line carries a seq: {line}"
        );
    }
}
