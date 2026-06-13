//! The `--wait` agent path: one command in, one parseable result out. Covers the
//! committed result (inline + `--out`), distinct-args exactly-once, and the
//! timeout/exit-3 shape.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::HashMap;

use common::{argv, endpoint, json_ok, run_kx, start_gateway, stdout};
use tempfile::TempDir;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn invoke_wait_returns_committed_result() {
    let dir = TempDir::new().unwrap();
    let running = start_gateway(&dir, true, HashMap::new()).await;
    let ep = endpoint(&running);

    let out = run_kx(argv(&[
        "invoke",
        "kx/recipes/echo",
        "--args",
        r#"{"topic":"incidents"}"#,
        "--wait",
        "--endpoint",
        &ep,
        "--json",
    ]))
    .await;
    let v = json_ok(&out);
    assert_eq!(v["state"], "COMMITTED");
    // GR15: `echo` is a TRUE echo — it commits its bound `topic` verbatim as
    // text, never a fabricated placeholder. result_utf8 carries the topic.
    let expected = b"incidents".to_vec();
    assert_eq!(v["result_len"].as_u64().unwrap() as usize, expected.len());
    assert_eq!(v["result_hex"], kx_cli::hex::encode(&expected));
    assert_eq!(
        v["result_utf8"].as_str().unwrap(),
        "incidents",
        "echo commits its topic verbatim as text"
    );

    running.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn invoke_wait_out_writes_raw_bytes() {
    let dir = TempDir::new().unwrap();
    let running = start_gateway(&dir, true, HashMap::new()).await;
    let ep = endpoint(&running);
    let out_path = dir.path().join("result.bin");

    let out = run_kx(argv(&[
        "invoke",
        "kx/recipes/echo",
        "--args",
        r#"{"topic":"save-me"}"#,
        "--wait",
        "--out",
        out_path.to_str().unwrap(),
        "--endpoint",
        &ep,
        "--json",
    ]))
    .await;
    let v = json_ok(&out);
    assert_eq!(v["state"], "COMMITTED");
    // With --out the payload is NOT inlined.
    assert!(
        v.get("result_hex").is_none(),
        "payload not inlined under --out"
    );
    let written = std::fs::read(&out_path).unwrap();
    // GR15: echo commits its bound `topic` verbatim (no fabricated placeholder).
    assert_eq!(written, b"save-me");

    running.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn distinct_args_yield_distinct_committed_results() {
    let dir = TempDir::new().unwrap();
    let running = start_gateway(&dir, true, HashMap::new()).await;
    let ep = endpoint(&running);

    let a = json_ok(
        &run_kx(argv(&[
            "invoke",
            "kx/recipes/echo",
            "--args",
            r#"{"topic":"alpha"}"#,
            "--wait",
            "--endpoint",
            &ep,
            "--json",
        ]))
        .await,
    );
    let b = json_ok(
        &run_kx(argv(&[
            "invoke",
            "kx/recipes/echo",
            "--args",
            r#"{"topic":"bravo"}"#,
            "--wait",
            "--endpoint",
            &ep,
            "--json",
        ]))
        .await,
    );
    assert_eq!(a["instance_id"], b["instance_id"], "same recipe ⇒ one run");
    assert_ne!(
        a["terminal_mote_id"], b["terminal_mote_id"],
        "distinct args ⇒ distinct committed Motes (exactly-once-per-input)"
    );

    running.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn wait_timeout_zero_is_committed_or_running() {
    // --timeout-secs 0 polls once then decides. If the worker already committed,
    // exit 0 (COMMITTED) is CORRECT; otherwise exit 3 (RUNNING, resumable). Either
    // is valid — assert the contract, not a flaky race.
    let dir = TempDir::new().unwrap();
    let running = start_gateway(&dir, true, HashMap::new()).await;
    let ep = endpoint(&running);

    let out = run_kx(argv(&[
        "invoke",
        "kx/recipes/echo",
        "--args",
        r#"{"topic":"race"}"#,
        "--wait",
        "--timeout-secs",
        "0",
        "--endpoint",
        &ep,
        "--json",
    ]))
    .await;
    let code = out.status.code();
    assert!(
        matches!(code, Some(0) | Some(3)),
        "exit 0 (committed) or 3 (running); got {code:?}: {}",
        stdout(&out)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    if code == Some(3) {
        assert_eq!(v["state"], "RUNNING");
        assert_eq!(v["timed_out"], true);
    } else {
        assert_eq!(v["state"], "COMMITTED");
    }

    running.shutdown().await.unwrap();
}
