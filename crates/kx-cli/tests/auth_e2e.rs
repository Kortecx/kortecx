//! Auth posture over the wire: a deny-all port refuses every verb; a token-gated
//! port requires a valid bearer token (via `--token` or `--token-file`).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::HashMap;

use common::{argv, endpoint, json_ok, run_kx, start_gateway, stderr};
use tempfile::TempDir;

fn tokens() -> HashMap<String, String> {
    let mut m = HashMap::new();
    m.insert("s3cr3t".to_string(), "alice@acme".to_string());
    m
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn deny_all_refuses_every_verb() {
    let dir = TempDir::new().unwrap();
    // No --dev-allow-local, no tokens ⇒ deny-all.
    let running = start_gateway(&dir, false, HashMap::new()).await;
    let ep = endpoint(&running);

    let out = run_kx(argv(&[
        "invoke",
        "kx/recipes/echo",
        "--args",
        r#"{"topic":"x"}"#,
        "--endpoint",
        &ep,
    ]))
    .await;
    assert!(!out.status.success(), "deny-all refuses invoke");
    assert!(
        stderr(&out).contains("Unauthenticated"),
        "stderr: {}",
        stderr(&out)
    );

    running.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn token_gated_requires_a_valid_credential() {
    let dir = TempDir::new().unwrap();
    let running = start_gateway(&dir, false, tokens()).await;
    let ep = endpoint(&running);

    // No credential → Unauthenticated.
    let none = run_kx(argv(&[
        "invoke",
        "kx/recipes/echo",
        "--args",
        r#"{"topic":"x"}"#,
        "--endpoint",
        &ep,
    ]))
    .await;
    assert!(!none.status.success());
    assert!(stderr(&none).contains("Unauthenticated"));

    // Valid --token → the configured party holds a Use grant → runs to Committed.
    let with_tok = run_kx(argv(&[
        "invoke",
        "kx/recipes/echo",
        "--args",
        r#"{"topic":"incidents"}"#,
        "--wait",
        "--token",
        "s3cr3t",
        "--endpoint",
        &ep,
        "--json",
    ]))
    .await;
    assert_eq!(json_ok(&with_tok)["state"], "COMMITTED");

    running.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn token_file_authenticates() {
    let dir = TempDir::new().unwrap();
    let running = start_gateway(&dir, false, tokens()).await;
    let ep = endpoint(&running);

    // A token file (with a trailing newline, which is trimmed) authenticates.
    let tok_path = dir.path().join("token");
    std::fs::write(&tok_path, "s3cr3t\n").unwrap();

    let out = run_kx(argv(&[
        "invoke",
        "kx/recipes/echo",
        "--args",
        r#"{"topic":"incidents"}"#,
        "--wait",
        "--token-file",
        tok_path.to_str().unwrap(),
        "--endpoint",
        &ep,
        "--json",
    ]))
    .await;
    assert_eq!(json_ok(&out)["state"], "COMMITTED");

    running.shutdown().await.unwrap();
}
