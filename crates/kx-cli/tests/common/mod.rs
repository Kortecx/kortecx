//! Shared E2E harness: host an in-process gateway via `kx_gateway::start` (a
//! deterministic `:0` bound addr + graceful shutdown) and drive the `kx` CLIENT
//! verbs as subprocesses against it. The server runs in-process so the gateway's
//! tokio tasks (coordinator + worker) keep progressing while the test thread
//! awaits each `kx` subprocess (which runs on a blocking thread). Mirrors
//! `kx-gateway/tests/common` for the config shape.

#![allow(
    dead_code,
    unreachable_pub,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::pedantic
)]

use std::collections::HashMap;
use std::process::Output;
use std::time::Duration;

use kx_gateway::{start, GatewayConfig, RunningGateway};
use tempfile::TempDir;

/// A gateway config rooted at `dir` (ephemeral journal + content + catalog).
#[must_use]
pub fn gateway_config(
    dir: &TempDir,
    dev_allow_local: bool,
    auth_tokens: HashMap<String, String>,
) -> GatewayConfig {
    GatewayConfig {
        listen: "127.0.0.1:0".parse().unwrap(),
        ws_listen: "127.0.0.1:0".parse().unwrap(),
        journal_path: dir.path().join("kx.db"),
        content_root: dir.path().join("blobs"),
        max_lease: 16,
        dev_allow_local,
        auth_tokens,
        catalog_dir: None,
        tls: None,
        cors_origins: Vec::new(),
    }
}

/// Start an in-process gateway and wait until its port accepts connections (the
/// serve task may bind just after `start` returns the resolved addr; the `kx`
/// client connects once and fails fast, so the harness — which owns the server
/// lifecycle — synchronizes here).
pub async fn start_gateway(
    dir: &TempDir,
    dev_allow_local: bool,
    auth_tokens: HashMap<String, String>,
) -> RunningGateway {
    let running = start(gateway_config(dir, dev_allow_local, auth_tokens))
        .await
        .expect("gateway starts");
    let addr = running.local_addr();
    for _ in 0..500 {
        if tokio::net::TcpStream::connect(addr).await.is_ok() {
            return running;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("gateway never started accepting on {addr}");
}

/// The `http://addr` endpoint of a running gateway.
#[must_use]
pub fn endpoint(running: &RunningGateway) -> String {
    format!("http://{}", running.local_addr())
}

/// Build an owned-`String` argv from string slices.
#[must_use]
pub fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| (*s).to_string()).collect()
}

/// Run the `kx` binary with `args` (on a blocking thread so the in-process
/// gateway keeps serving). `RUST_LOG=warn` keeps stderr quiet.
pub async fn run_kx(args: Vec<String>) -> Output {
    tokio::task::spawn_blocking(move || {
        std::process::Command::new(env!("CARGO_BIN_EXE_kx"))
            .args(&args)
            .env("RUST_LOG", "warn")
            .output()
            .expect("spawn kx")
    })
    .await
    .expect("join kx subprocess")
}

/// stdout of an output as a `String`.
#[must_use]
pub fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// stderr of an output as a `String`.
#[must_use]
pub fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// Parse stdout as JSON, asserting the verb succeeded.
pub fn json_ok(out: &Output) -> serde_json::Value {
    assert!(
        out.status.success(),
        "expected success; stderr={}",
        stderr(out)
    );
    serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("stdout is not JSON ({e}): {}", stdout(out)))
}

/// Poll `kx projection --json` until `mote_hex` is COMMITTED; return its mote
/// object. Fails the test on timeout.
pub async fn poll_committed(
    endpoint: &str,
    instance_hex: &str,
    mote_hex: &str,
) -> serde_json::Value {
    for _ in 0..100 {
        let out = run_kx(argv(&[
            "projection",
            "--instance",
            instance_hex,
            "--endpoint",
            endpoint,
            "--json",
        ]))
        .await;
        let v = json_ok(&out);
        if let Some(motes) = v["motes"].as_array() {
            if let Some(m) = motes
                .iter()
                .find(|m| m["mote_id"] == mote_hex && m["state"] == "COMMITTED")
            {
                return m.clone();
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("mote {mote_hex} never reached COMMITTED");
}
