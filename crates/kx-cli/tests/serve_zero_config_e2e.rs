//! Zero-config `kx serve`: with only `--dev-allow-local` (and ephemeral `:0`
//! ports for test hermeticity) the runtime starts WITHOUT explicit `--journal`
//! / `--content` / `--catalog-dir`. The CLI auto-resolves a durable layout under
//! `$KX_DATA_DIR` (sandboxed to a TempDir here), creates it, and the gateway
//! prints a startup banner reporting every resolved path + endpoint. This test
//! drives the REAL `kx serve` binary as a long-running subprocess, parses the
//! banner from stderr to learn the bound gRPC port, asserts the durable stores
//! were created under the sandbox, and proves a run is submittable + readable
//! against the auto-resolved runtime. A companion test pins the secure-by-default
//! posture: a bare `kx serve` with NO auth flag fails closed (exit 2).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::process::Stdio;
use std::time::Duration;

use common::{argv, json_ok, run_kx};
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::time::timeout;

/// Strip ANSI CSI sequences (`ESC [ … m`) — the tracing fmt layer colorizes
/// fields (`with_ansi` defaults on, even to a pipe), which would otherwise split
/// a `key=value` token (`journal\x1b[0m\x1b[2m=…`).
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            for d in chars.by_ref() {
                if d == 'm' {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Extract a whitespace-delimited `key=value` field's value from a tracing line.
fn banner_field<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let pat = format!("{key}=");
    let start = line.find(&pat)? + pat.len();
    let rest = &line[start..];
    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    Some(&rest[..end])
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn zero_config_serve_auto_resolves_layout_and_runs() {
    let data_dir = TempDir::new().unwrap();

    // Only `--dev-allow-local` is the operator-supplied flag; `:0` ports keep the
    // test hermetic (the banner reports the resolved ephemeral gRPC port). No
    // `--journal` / `--content` / `--catalog-dir` — those auto-resolve under
    // KX_DATA_DIR.
    let mut child = Command::new(env!("CARGO_BIN_EXE_kx"))
        .args([
            "serve",
            "--dev-allow-local",
            "--listen",
            "127.0.0.1:0",
            "--ws-listen",
            "127.0.0.1:0",
            "--no-console",
        ])
        .env("RUST_LOG", "info")
        .env("KX_DATA_DIR", data_dir.path())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn kx serve");

    let mut lines = BufReader::new(child.stderr.take().unwrap()).lines();

    // Read stderr until the startup banner appears (generous timeout; the kill
    // below guarantees termination).
    let banner = timeout(Duration::from_secs(30), async {
        loop {
            match lines.next_line().await.unwrap() {
                Some(line) => {
                    let clean = strip_ansi(&line);
                    if clean.contains("kx-gateway STARTUP") {
                        return Some(clean);
                    }
                    // else: an earlier INFO line (listener ready, etc.)
                }
                None => return None, // the server exited before the banner — a bug
            }
        }
    })
    .await
    .expect("did not time out waiting for the startup banner")
    .expect("kx serve printed the startup banner before exiting");

    // The banner reports every resolved field.
    for key in [
        "data_dir",
        "journal",
        "content_dir",
        "catalog_dir",
        "catalog_db",
        "telemetry_db",
        "capture_db",
        "uploads_db",
        "grpc_endpoint",
        "ws_endpoint",
        "auth_mode",
        "connect_hint",
    ] {
        assert!(
            banner.contains(key),
            "banner is missing the {key} field: {banner}"
        );
    }
    assert!(
        banner.contains("dev-allow-local"),
        "banner reports the dev auth mode: {banner}"
    );

    // The auto-resolved durable layout was created under the sandbox base.
    let base = data_dir.path();
    assert!(base.join("kx.db").exists(), "journal auto-created");
    assert!(
        base.join("content").is_dir(),
        "content store dir auto-created"
    );
    assert!(
        base.join("catalog").join("catalog.db").exists(),
        "catalog sidecar auto-created under the catalog dir"
    );
    // The banner's resolved paths point under the sandbox.
    let banner_journal = banner_field(&banner, "journal").expect("journal field");
    assert!(
        banner_journal.starts_with(base.to_str().unwrap()),
        "the resolved journal lives under KX_DATA_DIR: {banner_journal}"
    );

    // Parse the bound gRPC endpoint and prove a run is submittable + readable
    // against the zero-config runtime.
    let grpc = banner_field(&banner, "grpc_endpoint").expect("grpc_endpoint field");
    let addr = grpc.trim_start_matches("http://").to_string();
    for _ in 0..500 {
        if tokio::net::TcpStream::connect(&addr).await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    // dev-allow-local ⇒ no token needed on loopback. `--wait` polls to terminal.
    let inv = json_ok(
        &run_kx(argv(&[
            "invoke",
            "kx/recipes/echo",
            "--args",
            r#"{"topic":"x"}"#,
            "--wait",
            "--endpoint",
            grpc,
            "--json",
        ]))
        .await,
    );
    assert_eq!(
        inv["state"].as_str(),
        Some("COMMITTED"),
        "the run committed against the zero-config runtime: {inv}"
    );
    assert!(
        inv["instance_id"].as_str().is_some_and(|s| !s.is_empty()),
        "the committed outcome carries the run instance_id: {inv}"
    );

    let _ = child.start_kill();
    let _ = child.wait().await;
}

/// Secure-by-default: a bare `kx serve` with no auth posture fails closed (exit
/// 2) and names the remediation flag — we never silently open a no-auth server.
#[test]
fn bare_serve_without_auth_posture_fails_closed() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_kx"))
        .args(["serve"])
        .env("RUST_LOG", "warn")
        .output()
        .expect("spawn kx serve");
    assert_eq!(
        out.status.code(),
        Some(2),
        "no auth posture is a config error (exit 2)"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("--dev-allow-local"),
        "the error names the remediation flag: {stderr}"
    );
}
