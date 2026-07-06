//! The OSS ↔ private boundary as a TEST, not a convention (PR-W). The companion
//! to `.github/workflows/leak-check.yml`: that scans a PR diff in CI; this fails
//! the build locally + in CI if the boundary invariants drift. Mirrors the
//! shell-out-and-skip-if-unavailable pattern of `dep_wall.rs`.
//!
//! Four invariants (single source of truth: `shared-paths.toml` at the repo root):
//!  (a) the OSS public repo tracks ZERO `[private_only]` paths. Gated on repo
//!      identity — the private corpus repo LEGITIMATELY tracks them, so the
//!      assertion runs only when the corpus sentinel is NOT tracked.
//!  (b) the manifest `[private_only].paths` covers the known private roots
//!      (catches an accidentally-weakened manifest).
//!  (c) root `Cargo.toml` still `exclude`s `kx-cloud` (K0 — cloud stays outside
//!      the projection workspace, so it can never move the canonical digest).
//!  (d) EVERY tracked file is classified by the manifest — `[shared].include`,
//!      `[private_only].paths`, or `[divergent].paths`. The include-side twin of
//!      (a): an UNCLASSIFIED path is invisible to `just port` (never carried) AND
//!      to `just cmp-shared` (never compared), so it silently escapes the mirror
//!      in EITHER direction. Runs in BOTH repos. L-029 (`shared-paths-include-under-inclusion`).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::path::{Path, PathBuf};
use std::process::Command;

/// Run `git` at `root`; return stdout on success, `None` if git is unavailable
/// or the command failed (sandboxed CI / no repo) — skip rather than false-fail.
fn git(root: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .ok()?;
    if out.status.success() {
        Some(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        None
    }
}

fn repo_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // Prefer git's own answer; fall back to crates/kx-cli -> repo root.
    if let Some(top) = git(&manifest_dir, &["rev-parse", "--show-toplevel"]) {
        let p = PathBuf::from(top.trim());
        if p.is_dir() {
            return p;
        }
    }
    manifest_dir.join("..").join("..").canonicalize().unwrap()
}

/// Minimal line-scan parser for `key = [ "a", "b", ... ]` inside `[section]`.
/// The manifest keeps one element per line; a full TOML dep is intentionally
/// avoided (kx-cli has no toml dep, and this mirrors `dep_wall`'s string scan).
fn manifest_array(manifest: &str, section: &str, key: &str) -> Vec<String> {
    let header = format!("[{section}]");
    let mut out = Vec::new();
    let (mut in_sec, mut in_arr) = (false, false);
    for line in manifest.lines() {
        let t = line.trim_start();
        if t.starts_with('[') {
            in_sec = t == header;
            in_arr = false;
        }
        if in_sec && !in_arr && t.starts_with(key) {
            let rest = t[key.len()..].trim_start();
            if rest.starts_with('=') {
                in_arr = true;
            }
        }
        if in_arr {
            let mut s = line;
            while let Some(start) = s.find('"') {
                let after = &s[start + 1..];
                if let Some(end) = after.find('"') {
                    out.push(after[..end].to_string());
                    s = &after[end + 1..];
                } else {
                    break;
                }
            }
            if line.contains(']') {
                in_arr = false;
            }
        }
    }
    out
}

/// Read the manifest, or `None` if it is absent. `shared-paths.toml` is
/// `[private_only]` (2026-07-06): it lives ONLY in the private repo — the OSS
/// public repo does not carry it, so manifest-reading tests SKIP there (the
/// boundary is enforced from the private side: `just port` refuses private
/// paths, `cmp-shared`, and these tests running in the private repo).
fn read_manifest(root: &Path) -> Option<String> {
    std::fs::read_to_string(root.join("shared-paths.toml")).ok()
}

/// (c) K0 — root `Cargo.toml` must keep `kx-cloud` excluded from the workspace.
#[test]
fn root_cargo_excludes_kx_cloud() {
    let root = repo_root();
    let cargo = std::fs::read_to_string(root.join("Cargo.toml")).expect("root Cargo.toml");
    let exclude = cargo
        .split("exclude")
        .nth(1)
        .and_then(|s| s.split(']').next())
        .expect("an `exclude = [ ... ]` array in the root Cargo.toml");
    assert!(
        exclude.contains("\"kx-cloud\""),
        "K0 VIOLATION: root Cargo.toml must keep `kx-cloud` in workspace `exclude` \
         (cloud stays outside the projection workspace so it can never move the digest)"
    );
}

/// (b) the manifest must cover the known private roots — a weakened manifest
/// would silently widen what can leak into OSS.
#[test]
fn manifest_covers_known_private_roots() {
    let root = repo_root();
    let manifest = match read_manifest(&root) {
        Some(m) => m,
        None => return, // shared-paths.toml is [private_only] — absent in OSS; skip (private-side enforced)
    };
    let private = manifest_array(&manifest, "private_only", "paths");
    const REQUIRED: &[&str] = &[
        "kx-cloud/**",
        "docs/design/**",
        "docs/suggestions/**",
        "docs/analysis/**",
        "docs/plans/**",
        "docs/benchmarks/**",
        // The three private ledgers: the master feature-ledger (spans both lanes;
        // the public per-repo `feature-ledger.toml` is the [shared] one), plus the
        // bug- and learning-ledgers. All private_only — an OSS PR must never track them.
        "docs/feature-ledger.md",
        "docs/bug-ledger.md",
        "docs/learning-ledger.md",
        "CLAUDE.md",
        "WARNINGS.md",
        "00-*.md",
        "07-*.md",
    ];
    for req in REQUIRED {
        assert!(
            private.iter().any(|p| p == req),
            "shared-paths.toml [private_only].paths is missing the required private root `{req}`"
        );
    }
}

/// (a) the OSS public repo must track ZERO `[private_only]` paths. Skipped in the
/// private corpus repo (it legitimately tracks them) and when git is unavailable.
#[test]
fn oss_repo_tracks_no_private_paths() {
    let root = repo_root();
    // Repo identity: the corpus sentinel `00-vision-and-principles.md` is tracked
    // ONLY in the private corpus repo. If git is unavailable, skip.
    let sentinel = match git(&root, &["ls-files", "--", "00-vision-and-principles.md"]) {
        Some(s) => s,
        None => return, // no git / not a repo — the CI leak-check is the backstop
    };
    if !sentinel.trim().is_empty() {
        return; // private corpus repo — private paths are expected here
    }

    let manifest = match read_manifest(&root) {
        Some(m) => m,
        None => return, // shared-paths.toml is [private_only] — absent in OSS; skip (private-side enforced)
    };
    let private = manifest_array(&manifest, "private_only", "paths");
    // Ask git's own pathspec engine to list any tracked private path.
    let mut args: Vec<String> = vec!["ls-files".into(), "--".into()];
    for p in &private {
        args.push(format!(":(glob){p}"));
    }
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let tracked = match git(&root, &arg_refs) {
        Some(s) => s,
        None => return,
    };
    let hits: Vec<&str> = tracked.lines().filter(|l| !l.trim().is_empty()).collect();
    assert!(
        hits.is_empty(),
        "SN-2 LEAK: the OSS repo tracks private-only path(s):\n  {}",
        hits.join("\n  ")
    );
}

/// (d) EVERY tracked file must be classified by shared-paths.toml. An unclassified
/// path is never carried by `just port` and never compared by `just cmp-shared`, so
/// it silently drifts the mirror in either direction (L-029). Reuses git's own glob
/// engine — the union of the three classes' `:(glob)` pathspecs must cover every
/// tracked file. Runs in BOTH repos; skipped only when git is unavailable.
#[test]
fn every_tracked_file_is_classified() {
    use std::collections::BTreeSet;
    let root = repo_root();
    let all = match git(&root, &["ls-files"]) {
        Some(s) => s,
        None => return, // no git — the CI leak-check is the backstop
    };
    let manifest = match read_manifest(&root) {
        Some(m) => m,
        None => return, // shared-paths.toml is [private_only] — absent in OSS; skip (private-side enforced)
    };
    // Union of [shared].include + [private_only].paths + [divergent].paths as git
    // glob pathspecs. `divergent` is authoritative here too — a divergent file is
    // classified (intentionally per-repo), just never ported.
    let mut args: Vec<String> = vec!["ls-files".into(), "--".into()];
    for (section, key) in [
        ("shared", "include"),
        ("private_only", "paths"),
        ("divergent", "paths"),
    ] {
        for p in manifest_array(&manifest, section, key) {
            args.push(format!(":(glob){p}"));
        }
    }
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let classified = match git(&root, &arg_refs) {
        Some(s) => s,
        None => return,
    };
    let classified_set: BTreeSet<&str> = classified
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let unclassified: Vec<&str> = all
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .filter(|l| !classified_set.contains(l))
        .collect();
    assert!(
        unclassified.is_empty(),
        "L-029 UNDER-INCLUSION: shared-paths.toml classifies neither [shared] / \
         [private_only] / [divergent] for {} tracked path(s) — each silently escapes \
         the mirror (never ported, never cmp-shared'd):\n  {}",
        unclassified.len(),
        unclassified.join("\n  ")
    );
}
