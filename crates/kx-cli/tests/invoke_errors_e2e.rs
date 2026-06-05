//! Fail-closed error surfaces for `invoke`: an unknown handle is uniformly
//! permission-denied (no existence oracle on the execution surface); malformed
//! args are invalid-argument (server-side, fail-closed); client-side-invalid
//! JSON is rejected before the round trip.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::HashMap;

use common::{argv, endpoint, run_kx, start_gateway, stderr};
use tempfile::TempDir;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unknown_handle_is_permission_denied() {
    let dir = TempDir::new().unwrap();
    let running = start_gateway(&dir, true, HashMap::new()).await;
    let ep = endpoint(&running);

    let out = run_kx(argv(&[
        "invoke",
        "kx/recipes/does-not-exist",
        "--args",
        r#"{"topic":"x"}"#,
        "--endpoint",
        &ep,
    ]))
    .await;
    assert_eq!(out.status.code(), Some(1), "RPC error exits 1");
    assert!(
        stderr(&out).contains("PermissionDenied"),
        "stderr: {}",
        stderr(&out)
    );

    running.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn malformed_args_are_invalid_argument() {
    let dir = TempDir::new().unwrap();
    let running = start_gateway(&dir, true, HashMap::new()).await;
    let ep = endpoint(&running);

    // Valid JSON, but the wrong shape for the recipe's free-params (topic: number).
    let out = run_kx(argv(&[
        "invoke",
        "kx/recipes/echo",
        "--args",
        r#"{"topic":5}"#,
        "--endpoint",
        &ep,
    ]))
    .await;
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("InvalidArgument"),
        "stderr: {}",
        stderr(&out)
    );

    running.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn client_side_invalid_json_is_usage_exit_2() {
    let dir = TempDir::new().unwrap();
    let running = start_gateway(&dir, true, HashMap::new()).await;
    let ep = endpoint(&running);

    // `{` is not valid JSON — rejected client-side (exit 2) before any round trip.
    let out = run_kx(argv(&[
        "invoke",
        "kx/recipes/echo",
        "--args",
        "{",
        "--endpoint",
        &ep,
    ]))
    .await;
    assert_eq!(out.status.code(), Some(2), "client-side bad JSON exits 2");
    assert!(stderr(&out).to_lowercase().contains("json"));

    running.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bad_hex_instance_is_usage_exit_2() {
    // A wrong-length --instance is a client-side usage error (no server needed,
    // but a live endpoint proves it fails BEFORE connecting).
    let dir = TempDir::new().unwrap();
    let running = start_gateway(&dir, true, HashMap::new()).await;
    let ep = endpoint(&running);

    let out = run_kx(argv(&[
        "projection",
        "--instance",
        "abcd",
        "--endpoint",
        &ep,
    ]))
    .await;
    assert_eq!(out.status.code(), Some(2), "bad hex length exits 2");

    running.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn args_file_valid_runs_to_committed() {
    let dir = TempDir::new().unwrap();
    let running = start_gateway(&dir, true, HashMap::new()).await;
    let ep = endpoint(&running);

    let args_path = dir.path().join("args.json");
    std::fs::write(&args_path, r#"{"topic":"from-file"}"#).unwrap();

    let out = run_kx(argv(&[
        "invoke",
        "kx/recipes/echo",
        "--args-file",
        args_path.to_str().unwrap(),
        "--wait",
        "--endpoint",
        &ep,
        "--json",
    ]))
    .await;
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["state"], "COMMITTED");

    running.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn args_file_errors_and_mutual_exclusion() {
    let dir = TempDir::new().unwrap();
    let running = start_gateway(&dir, true, HashMap::new()).await;
    let ep = endpoint(&running);

    // Missing file → IO error (exit 1).
    let missing = run_kx(argv(&[
        "invoke",
        "kx/recipes/echo",
        "--args-file",
        "/no/such/file.json",
        "--endpoint",
        &ep,
    ]))
    .await;
    assert_eq!(
        missing.status.code(),
        Some(1),
        "missing --args-file is IO/exit 1"
    );

    // Invalid JSON in the file → client-side usage (exit 2), no round trip.
    let bad_path = dir.path().join("bad.json");
    std::fs::write(&bad_path, "{").unwrap();
    let bad = run_kx(argv(&[
        "invoke",
        "kx/recipes/echo",
        "--args-file",
        bad_path.to_str().unwrap(),
        "--endpoint",
        &ep,
    ]))
    .await;
    assert_eq!(bad.status.code(), Some(2), "invalid JSON in file exits 2");

    // --args and --args-file together → mutual-exclusion usage error (exit 2).
    let both = run_kx(argv(&[
        "invoke",
        "kx/recipes/echo",
        "--args",
        "{}",
        "--args-file",
        bad_path.to_str().unwrap(),
        "--endpoint",
        &ep,
    ]))
    .await;
    assert_eq!(
        both.status.code(),
        Some(2),
        "--args + --args-file is exit 2"
    );
    assert!(stderr(&both).contains("mutually exclusive"));

    running.shutdown().await.unwrap();
}
