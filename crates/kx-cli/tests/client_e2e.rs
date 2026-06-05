//! End-to-end witnesses over a REAL bound port: the operator hosts a gateway
//! in-process, an analyst drives the `kx` CLIENT verbs as subprocesses. Covers
//! the real-life flow — invoke a recipe, inspect the projection, fetch the
//! committed result, tail events — plus `submit --demo` and a restart.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::HashMap;

use common::{argv, endpoint, json_ok, poll_committed, run_kx, start_gateway, stdout};
use tempfile::TempDir;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn invoke_projection_content_events_flow() {
    let dir = TempDir::new().unwrap();
    let running = start_gateway(&dir, true, HashMap::new()).await;
    let ep = endpoint(&running);

    // (1) invoke (async handle).
    let inv = run_kx(argv(&[
        "invoke",
        "kx/recipes/echo",
        "--args",
        r#"{"topic":"incidents"}"#,
        "--endpoint",
        &ep,
        "--json",
    ]))
    .await;
    let inv = json_ok(&inv);
    let instance = inv["instance_id"].as_str().unwrap().to_string();
    let terminal = inv["terminal_mote_id"].as_str().unwrap().to_string();
    assert_eq!(instance.len(), 32, "16B instance id");
    assert_eq!(terminal.len(), 64, "32B terminal mote id");

    // (2) projection: poll until the terminal Mote commits.
    let mote = poll_committed(&ep, &instance, &terminal).await;
    let result_ref = mote["result_ref"].as_str().unwrap().to_string();
    assert_eq!(result_ref.len(), 64);

    // (3) content: the human path writes RAW bytes == the worker's demo result.
    let content = run_kx(argv(&[
        "content",
        "--ref",
        &result_ref,
        "--instance",
        &instance,
        "--endpoint",
        &ep,
    ]))
    .await;
    assert!(content.status.success(), "content failed");
    let mote_bytes = kx_cli::hex::decode_fixed::<32>(&terminal).unwrap();
    assert_eq!(
        content.stdout,
        kx_gateway::demo_pure_result(&mote_bytes),
        "content returns the exact committed bytes"
    );

    // (4) events: NDJSON to head carries the Committed delta for the Mote.
    let events = run_kx(argv(&[
        "events",
        "--instance",
        &instance,
        "--endpoint",
        &ep,
        "--json",
    ]))
    .await;
    assert!(events.status.success());
    let saw_committed = stdout(&events).lines().any(|line| {
        serde_json::from_str::<serde_json::Value>(line)
            .ok()
            .is_some_and(|v| v["kind"] == "committed" && v["mote_id"] == terminal)
    });
    assert!(saw_committed, "events reported the Committed delta");

    running.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn submit_demo_commits() {
    let dir = TempDir::new().unwrap();
    let running = start_gateway(&dir, true, HashMap::new()).await;
    let ep = endpoint(&running);

    let out = run_kx(argv(&["submit", "--demo", "--endpoint", &ep, "--json"])).await;
    let v = json_ok(&out);
    assert_eq!(v["instance_id"].as_str().unwrap().len(), 32);
    assert_eq!(v["recipe_fingerprint"].as_str().unwrap().len(), 64);

    running.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn committed_run_survives_a_restart() {
    let dir = TempDir::new().unwrap();
    let instance = {
        let running = start_gateway(&dir, true, HashMap::new()).await;
        let ep = endpoint(&running);
        let inv = json_ok(
            &run_kx(argv(&[
                "invoke",
                "kx/recipes/echo",
                "--args",
                r#"{"topic":"durable"}"#,
                "--wait",
                "--endpoint",
                &ep,
                "--json",
            ]))
            .await,
        );
        assert_eq!(inv["state"], "COMMITTED");
        let id = inv["instance_id"].as_str().unwrap().to_string();
        running.shutdown().await.unwrap();
        id
    };

    // A fresh server on the same journal + content re-serves the committed run.
    let running = start_gateway(&dir, true, HashMap::new()).await;
    let ep = endpoint(&running);
    let view = json_ok(
        &run_kx(argv(&[
            "projection",
            "--instance",
            &instance,
            "--endpoint",
            &ep,
            "--json",
        ]))
        .await,
    );
    let any_committed = view["motes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|m| m["state"] == "COMMITTED");
    assert!(any_committed, "the committed run survives a restart");

    running.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn content_out_writes_committed_bytes_to_file() {
    let dir = TempDir::new().unwrap();
    let running = start_gateway(&dir, true, HashMap::new()).await;
    let ep = endpoint(&running);

    // Run a recipe to completion, then fetch its result with `content --out`.
    let inv = json_ok(
        &run_kx(argv(&[
            "invoke",
            "kx/recipes/echo",
            "--args",
            r#"{"topic":"to-disk"}"#,
            "--endpoint",
            &ep,
            "--json",
        ]))
        .await,
    );
    let instance = inv["instance_id"].as_str().unwrap().to_string();
    let terminal = inv["terminal_mote_id"].as_str().unwrap().to_string();
    let mote = poll_committed(&ep, &instance, &terminal).await;
    let result_ref = mote["result_ref"].as_str().unwrap().to_string();

    let out_path = dir.path().join("fetched.bin");
    let out = run_kx(argv(&[
        "content",
        "--ref",
        &result_ref,
        "--instance",
        &instance,
        "--out",
        out_path.to_str().unwrap(),
        "--endpoint",
        &ep,
    ]))
    .await;
    assert!(out.status.success(), "stderr: {}", stdout(&out));
    let mote_bytes = kx_cli::hex::decode_fixed::<32>(&terminal).unwrap();
    assert_eq!(
        std::fs::read(&out_path).unwrap(),
        kx_gateway::demo_pure_result(&mote_bytes),
        "content --out writes the exact committed bytes"
    );

    running.shutdown().await.unwrap();
}
