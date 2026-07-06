//! SN-2 / Golden Rule 10 guard (mirrors the `dep_wall.rs` manifest-scan
//! pattern). The `just profile` HARNESS is public OSS, but the captured numbers
//! are a private corpus trend record. Two independent proofs that no result
//! file can leak into the *public* repo:
//!  1. the workspace `.gitignore` covers `/docs/benchmarks/`, and
//!  2. `git ls-files docs/benchmarks/` tracks nothing (the tripwire).
//!
//! Both are **OSS-only invariants**. In the PRIVATE corpus repo the benchmark
//! results are LEGITIMATELY tracked (`docs/benchmarks/**` is `[private_only]`, the
//! GR10 trend record) and its `.gitignore` is `[divergent]` — so both assertions are
//! *expected* to be false there. Like `kx-cli/tests/shared_boundary.rs`, the checks
//! are gated on the corpus sentinel `00-vision-and-principles.md` (tracked ONLY in
//! private) and skip in the private repo. Without this gate a `crates/**`-mirrored
//! shared test is silently RED on private while green on OSS (L-029 class).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::path::PathBuf;
use std::process::Command;

/// The repo root = this crate's manifest dir, two levels up (`crates/kx-profile`).
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

/// True in the PRIVATE corpus repo, detected by the sentinel `00-vision-and-principles.md`
/// (tracked only there). Returns `false` when git is unavailable — the OSS assertions
/// are the load-bearing gate, so defaulting to "not private" keeps them running.
fn is_private_corpus_repo(root: &PathBuf) -> bool {
    Command::new("git")
        .args(["ls-files", "--", "00-vision-and-principles.md"])
        .current_dir(root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .is_some_and(|o| !String::from_utf8_lossy(&o.stdout).trim().is_empty())
}

#[test]
fn gitignore_covers_docs_benchmarks() {
    // The private repo legitimately tracks benchmarks + keeps a divergent .gitignore.
    if is_private_corpus_repo(&repo_root()) {
        return;
    }
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
    let root = repo_root();
    // The private corpus repo IS where the results live (GR10) — skip the tripwire there.
    if is_private_corpus_repo(&root) {
        return;
    }
    // `git ls-files` lists only TRACKED files; nothing under docs/benchmarks/
    // may be committed to OSS. Skip if git is unavailable (the include_str!
    // scan above is the load-bearing gate).
    let output = Command::new("git")
        .args(["ls-files", "docs/benchmarks/"])
        .current_dir(&root)
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
