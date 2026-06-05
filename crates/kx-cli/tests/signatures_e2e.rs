//! The `signatures` catalog verbs over the wire. The demo provisioning seeds a
//! recipe but no signatures, so the registry starts empty: `list` is empty,
//! `get` of an unknown id is not-found (the catalog is a public discovery
//! surface — not collapsed like the execution path), and `register` of a
//! malformed manifest is invalid-argument (fail-closed).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::HashMap;

use common::{argv, endpoint, run_kx, start_gateway, stderr, stdout};
use tempfile::TempDir;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn list_is_empty_then_get_unknown_then_register_garbage() {
    let dir = TempDir::new().unwrap();
    let running = start_gateway(&dir, true, HashMap::new()).await;
    let ep = endpoint(&running);

    // list: empty registry (human form says so; JSON form is an empty array).
    let list = run_kx(argv(&["signatures", "list", "--endpoint", &ep])).await;
    assert!(list.status.success(), "stderr: {}", stderr(&list));
    assert!(stdout(&list).contains("no signatures"));

    let list_json = run_kx(argv(&["signatures", "list", "--endpoint", &ep, "--json"])).await;
    let v: serde_json::Value = serde_json::from_slice(&list_json.stdout).unwrap();
    assert_eq!(v["signatures"].as_array().unwrap().len(), 0);

    // get an unknown id → not_found (exit 1). The discovery surface is honest
    // about existence (unlike the execution surface).
    let get = run_kx(argv(&[
        "signatures",
        "get",
        "--id",
        &"ab".repeat(32),
        "--endpoint",
        &ep,
    ]))
    .await;
    assert_eq!(get.status.code(), Some(1));
    assert!(
        stderr(&get).contains("NotFound"),
        "stderr: {}",
        stderr(&get)
    );

    // register a malformed manifest → invalid_argument (exit 1), fail-closed.
    let manifest = dir.path().join("garbage.bin");
    std::fs::write(&manifest, b"not a valid signature manifest").unwrap();
    let reg = run_kx(argv(&[
        "signatures",
        "register",
        "--manifest-file",
        manifest.to_str().unwrap(),
        "--endpoint",
        &ep,
    ]))
    .await;
    assert_eq!(reg.status.code(), Some(1));
    assert!(
        stderr(&reg).contains("InvalidArgument"),
        "stderr: {}",
        stderr(&reg)
    );

    running.shutdown().await.unwrap();
}
