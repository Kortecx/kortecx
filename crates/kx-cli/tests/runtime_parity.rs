//! `run` / `replay` / `digest` forwarding parity + global flags. No server: the
//! engine verbs are local. Proves the unified `kx` reproduces the kx-runtime
//! engine's output (the projection-digest invariant is preserved by reusing the
//! engine VERBATIM) and that usage/version/help behave.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::process::{Command, Output};

use tempfile::TempDir;

fn kx(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_kx"))
        .args(args)
        .env("RUST_LOG", "warn")
        .output()
        .expect("spawn kx")
}

#[test]
fn run_then_digest_agree_on_the_projection_digest() {
    let dir = TempDir::new().unwrap();
    let journal = dir.path().join("kx.db");
    let content = dir.path().join("blobs");
    let (j, c) = (journal.to_str().unwrap(), content.to_str().unwrap());

    // run: "<hex> (<committed>/<total> committed)".
    let run = kx(&["run", "--journal", j, "--content", c]);
    assert!(
        run.status.success(),
        "run failed: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    let run_line = String::from_utf8_lossy(&run.stdout);
    let run_hex = run_line.split_whitespace().next().expect("a digest token");
    assert_eq!(run_hex.len(), 64, "digest is 64 hex chars");
    assert!(run_line.contains("committed"));

    // digest: "<hex>" — must equal run's digest (same on-disk journal).
    let dig = kx(&["digest", "--journal", j, "--content", c]);
    assert!(dig.status.success());
    let dig_hex = String::from_utf8_lossy(&dig.stdout).trim().to_string();
    assert_eq!(
        run_hex, dig_hex,
        "run and digest agree on the projection digest"
    );

    // --json forms parse + carry the same digest.
    let dig_json = kx(&["digest", "--journal", j, "--content", c, "--json"]);
    let v: serde_json::Value = serde_json::from_slice(&dig_json.stdout).unwrap();
    assert_eq!(v["digest"], dig_hex);

    let run_json = kx(&["run", "--journal", j, "--content", c, "--json"]);
    let rv: serde_json::Value = serde_json::from_slice(&run_json.stdout).unwrap();
    assert_eq!(rv["digest"], dig_hex);
    assert!(rv["committed"].is_number() && rv["total"].is_number());
}

#[test]
fn missing_required_flag_is_usage_exit_2() {
    let out = kx(&["run", "--content", "/tmp/c"]); // no --journal
    assert_eq!(out.status.code(), Some(2), "usage error exits 2");
    let err = String::from_utf8_lossy(&out.stderr).to_lowercase();
    assert!(err.contains("journal"), "names the missing flag: {err}");
}

#[test]
fn version_and_help_and_unknown() {
    let ver = kx(&["--version"]);
    assert!(ver.status.success());
    assert!(String::from_utf8_lossy(&ver.stdout).starts_with("kx "));

    let help = kx(&["--help"]);
    assert!(help.status.success());
    let h = String::from_utf8_lossy(&help.stdout);
    assert!(h.contains("invoke") && h.contains("serve") && h.contains("digest"));

    // Empty argv is treated as help (exit 0).
    let empty = kx(&[]);
    assert!(empty.status.success());

    // `help <verb>` prints verb help.
    let hv = kx(&["help", "invoke"]);
    assert!(hv.status.success());
    assert!(String::from_utf8_lossy(&hv.stdout).contains("--wait"));

    // Unknown command is a usage error (exit 2).
    let unknown = kx(&["frobnicate"]);
    assert_eq!(unknown.status.code(), Some(2));
}
