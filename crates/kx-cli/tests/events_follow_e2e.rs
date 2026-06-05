//! R5 — `kx events --follow` consumes the gateway's LIVE TAIL: ONE open stream
//! that keeps delivering deltas as the journal advances (no 250ms re-poll). This
//! spawns the long-running follow subprocess against an in-process gateway, reads
//! its stdout until the committed delta appears (delivered live by the tail), then
//! kills it (the follow path never exits on its own).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::HashMap;
use std::process::Stdio;
use std::time::Duration;

use common::{argv, endpoint, json_ok, run_kx, start_gateway};
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::time::timeout;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn events_follow_streams_the_live_committed_delta() {
    let dir = TempDir::new().unwrap();
    let running = start_gateway(&dir, true, HashMap::new()).await;
    let ep = endpoint(&running);

    // Submit the demo run; grab its instance id (hex). The embedded worker commits
    // the PURE Mote shortly after.
    let submit = json_ok(&run_kx(argv(&["submit", "--demo", "--endpoint", &ep, "--json"])).await);
    let instance = submit["instance_id"]
        .as_str()
        .expect("submit --json carries instance_id")
        .to_string();

    // `events --follow` opens ONE live stream and does not exit on its own.
    let mut child = Command::new(env!("CARGO_BIN_EXE_kx"))
        .args([
            "events",
            "--instance",
            &instance,
            "--endpoint",
            &ep,
            "--follow",
        ])
        .env("RUST_LOG", "warn")
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn kx events --follow");
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();

    // The live tail delivers the committed delta on the open stream; the CLI prints
    // it. (Generous timeout; the kill below guarantees the test terminates.)
    let first = timeout(Duration::from_secs(20), async {
        loop {
            match lines.next_line().await.unwrap() {
                Some(line) if !line.trim().is_empty() => return Some(line),
                Some(_) => {}        // skip blank lines
                None => return None, // process ended unexpectedly (would be a bug)
            }
        }
    })
    .await
    .expect("did not time out waiting for a live delta line");

    // --follow never exits; kill it, then tear down.
    let _ = child.start_kill();
    let _ = child.wait().await;
    running.shutdown().await.unwrap();

    let line = first.expect("events --follow printed a delta before the process ended");
    assert!(
        line.to_lowercase().contains("committed"),
        "the live follow stream printed the committed delta: {line:?}"
    );
}
