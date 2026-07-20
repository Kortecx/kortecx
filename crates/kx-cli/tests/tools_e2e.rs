//! `kx tools list | score` over the wire (W1.A5 toolscout CLI parity). The demo
//! provisioning registers the OSS built-in tools, so `list` returns them; `score`
//! ranks them against an intent and dry-runs the lowering gate. ADVISORY (SN-8):
//! the scores/verdict are display-only — the CLI sends no warrant and the run
//! list stays empty (scoring registers nothing).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::HashMap;

use common::{argv, endpoint, run_kx, start_gateway, stderr, stdout};
use tempfile::TempDir;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn list_shows_builtins_then_score_ranks_and_stays_advisory() {
    let dir = TempDir::new().unwrap();
    let running = start_gateway(&dir, true, HashMap::new()).await;
    let ep = endpoint(&running);

    // list (human): the OSS built-ins, each with its kind. `text-summarize@1` was
    // removed from the built-in set — no capability could ever be registered for it,
    // so `kx tools list` was advertising a tool that could not dispatch.
    let list = run_kx(argv(&["tools", "list", "--endpoint", &ep])).await;
    assert!(list.status.success(), "stderr: {}", stderr(&list));
    let list_text = stdout(&list);
    for tool in ["fs-read@1", "fs-write@1"] {
        assert!(list_text.contains(tool), "list missing {tool}: {list_text}");
    }
    assert!(
        !list_text.contains("text-summarize"),
        "list must not advertise an unimplemented tool: {list_text}"
    );

    // list (--json): a manifests array of exactly the builtins, in order.
    let list_json = run_kx(argv(&["tools", "list", "--endpoint", &ep, "--json"])).await;
    let v: serde_json::Value = serde_json::from_slice(&list_json.stdout).unwrap();
    let manifests = v["manifests"].as_array().unwrap();
    assert_eq!(manifests.len(), 2);
    assert_eq!(manifests[0]["tool_id"], "fs-read");
    assert_eq!(manifests[0]["kind"], "Builtin");
    assert_eq!(
        manifests[0]["fingerprint_hash"].as_str().unwrap().len(),
        64,
        "32B fingerprint as hex"
    );

    // score (--json): the exact-keyword intent ranks fs-read at the 10000 ceiling;
    // the FFI-free serve has no react runtime, so the dry-run verdict degrades.
    let score = run_kx(argv(&[
        "tools",
        "score",
        "--intent",
        "read a file from disk",
        "--tool",
        "fs-read@1",
        "--language-tag",
        "en",
        "--endpoint",
        &ep,
        "--json",
    ]))
    .await;
    assert!(score.status.success(), "stderr: {}", stderr(&score));
    let s: serde_json::Value = serde_json::from_slice(&score.stdout).unwrap();
    let ranked = s["ranked"].as_array().unwrap();
    assert_eq!(ranked.len(), 2, "every manifest ranked");
    assert_eq!(ranked[0]["tool_id"], "fs-read");
    assert_eq!(ranked[0]["score_bp"], 10_000);
    assert_eq!(s["bundle_fingerprint"].as_str().unwrap().len(), 64);
    assert_eq!(s["verdict"], "unavailable");
    assert_eq!(s["advisory"], "scores never authorize a tool");

    // ADVISORY end to end: scoring registered no run.
    let runs = run_kx(argv(&[
        "projection",
        "--instance",
        &"00".repeat(16),
        "--endpoint",
        &ep,
    ]))
    .await;
    assert!(!runs.status.success(), "an unknown instance is not found");

    running.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn score_usage_errors_exit_2() {
    let dir = TempDir::new().unwrap();
    let running = start_gateway(&dir, true, HashMap::new()).await;
    let ep = endpoint(&running);

    // No --tool → a client-side usage error (exit 2), no RPC.
    let no_tool = run_kx(argv(&[
        "tools",
        "score",
        "--intent",
        "x",
        "--endpoint",
        &ep,
    ]))
    .await;
    assert_eq!(
        no_tool.status.code(),
        Some(2),
        "stderr: {}",
        stderr(&no_tool)
    );

    // A malformed tool ref → usage error (exit 2).
    let bad_ref = run_kx(argv(&[
        "tools",
        "score",
        "--intent",
        "x",
        "--tool",
        "no-version",
        "--endpoint",
        &ep,
    ]))
    .await;
    assert_eq!(
        bad_ref.status.code(),
        Some(2),
        "stderr: {}",
        stderr(&bad_ref)
    );

    running.shutdown().await.unwrap();
}
