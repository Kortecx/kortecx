//! SN-2 / Golden Rule 10 guard (mirrors the `dep_wall.rs` manifest-scan
//! pattern). The `just profile` HARNESS is public OSS, but the captured numbers
//! are a private corpus trend record. Two independent proofs that no result
//! file can leak into the public repo:
//!  1. the workspace `.gitignore` covers `/docs/benchmarks/`, and
//!  2. `git ls-files docs/benchmarks/` tracks nothing (the tripwire).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::path::PathBuf;
use std::process::Command;

/// The repo root = this crate's manifest dir, two levels up (`crates/kx-profile`).
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

#[test]
fn gitignore_covers_docs_benchmarks() {
    // include_str! embeds at compile time — robust against the test's CWD.
    let gitignore = include_str!("../../../.gitignore");
    assert!(
        gitignore.lines().any(|l| l.trim() == "/docs/benchmarks/"),
        "the workspace .gitignore must cover /docs/benchmarks/ so profiling \
         results (Golden Rule 10 trend record) never leak into the public OSS repo"
    );
}

#[test]
fn no_benchmark_result_is_tracked() {
    // `git ls-files` lists only TRACKED files; nothing under docs/benchmarks/
    // may be committed to OSS. Skip if git is unavailable (the include_str!
    // scan above is the load-bearing gate).
    let output = Command::new("git")
        .args(["ls-files", "docs/benchmarks/"])
        .current_dir(repo_root())
        .output();
    let Ok(output) = output else {
        return; // git not available in this sandbox — the scan above still gates
    };
    if !output.status.success() {
        return;
    }
    let tracked = String::from_utf8_lossy(&output.stdout);
    assert!(
        tracked.trim().is_empty(),
        "no benchmark result may be tracked in the OSS repo (SN-2); found:\n{tracked}"
    );
}
