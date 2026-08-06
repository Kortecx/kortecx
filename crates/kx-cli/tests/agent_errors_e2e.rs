//! Error CLASSIFICATION for `kx agent run` — which failures are the user's invocation
//! and which are the serve's missing preconditions.
//!
//! The distinction is not cosmetic. `CliError::Usage` makes `render_error` print the
//! ENTIRE usage block before the message, so the one sentence saying what to do scrolls
//! off behind a wall of syntax. A well-formed command that the serve cannot satisfy is
//! not a usage error, and must not be rendered as one.
//!
//! Each arm below is paired with a one-variable control that DOES belong in the other
//! class, so a pass means the two are told apart — not merely that some error occurred.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::HashMap;

use common::{argv, endpoint, run_kx, start_gateway, stderr};
use tempfile::TempDir;

/// The first line of the usage block (`cli.rs`), i.e. what a Usage-classified error
/// prints ahead of its own message.
const USAGE_BANNER: &str = "usage: kx <command> [args]";

/// Write a throwaway file to stand in for an image. The upload happens BEFORE the
/// vision probe, so the path must exist or the test would pass for the wrong reason.
fn image_path(dir: &TempDir) -> String {
    let p = dir.path().join("frame.png");
    std::fs::write(&p, b"\x89PNG\r\n\x1a\n").unwrap();
    p.to_string_lossy().into_owned()
}

/// `--image` against a serve with no vision model: a well-formed invocation that this
/// serve cannot satisfy. The remedy is to serve a vision-capable model, so the message
/// must lead — not follow the whole usage block.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn no_vision_model_is_a_failed_precondition_not_a_usage_error() {
    let dir = TempDir::new().unwrap();
    let img = image_path(&dir);
    let running = start_gateway(&dir, true, HashMap::new()).await;
    let ep = endpoint(&running);

    let out = run_kx(argv(&[
        "agent",
        "run",
        "--goal",
        "what is in this image?",
        "--image",
        &img,
        "--endpoint",
        &ep,
    ]))
    .await;
    let err = stderr(&out);

    assert_eq!(
        out.status.code(),
        Some(1),
        "a serve precondition exits 1, not 2: {err}"
    );
    assert!(
        err.contains("FailedPrecondition"),
        "the failure must name its class: {err}"
    );
    // Assert the REASON, so an upload failure or a transport error cannot pass as this.
    assert!(
        err.contains("no vision model is served"),
        "the message must say what is missing: {err}"
    );
    assert!(
        !err.contains(USAGE_BANNER),
        "a well-formed command must NOT print the usage block ahead of its remedy: {err}"
    );

    running.shutdown().await.unwrap();
}

/// The one-variable control: same verb, same image, PLUS `--dataset`. That combination
/// really is malformed — there is no agentic vision-RAG recipe — so it stays a usage
/// error and SHOULD print the usage block. Without this arm the assertion above would
/// pass for a CLI that had simply stopped classifying anything as usage.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn image_plus_dataset_remains_a_usage_error() {
    let dir = TempDir::new().unwrap();
    let img = image_path(&dir);
    let running = start_gateway(&dir, true, HashMap::new()).await;
    let ep = endpoint(&running);

    let out = run_kx(argv(&[
        "agent",
        "run",
        "--goal",
        "what is in this image?",
        "--image",
        &img,
        "--dataset",
        "docs",
        "--endpoint",
        &ep,
    ]))
    .await;
    let err = stderr(&out);

    assert_eq!(
        out.status.code(),
        Some(2),
        "a malformed invocation exits 2: {err}"
    );
    assert!(
        err.contains(USAGE_BANNER),
        "a usage error SHOULD carry the usage block: {err}"
    );
    assert!(
        err.contains("--image and --dataset cannot be combined"),
        "and must still say which flags conflict: {err}"
    );

    running.shutdown().await.unwrap();
}

/// An unknown flag is the plainest usage error there is — the second control, pinning
/// that the classification still fires on the case nobody disputes.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_unknown_flag_is_a_usage_error() {
    let dir = TempDir::new().unwrap();
    let running = start_gateway(&dir, true, HashMap::new()).await;
    let ep = endpoint(&running);

    let out = run_kx(argv(&[
        "agent",
        "run",
        "--goal",
        "hi",
        "--nope",
        "1",
        "--endpoint",
        &ep,
    ]))
    .await;
    assert_eq!(out.status.code(), Some(2), "stderr: {}", stderr(&out));
    assert!(
        stderr(&out).contains(USAGE_BANNER),
        "stderr: {}",
        stderr(&out)
    );

    running.shutdown().await.unwrap();
}
