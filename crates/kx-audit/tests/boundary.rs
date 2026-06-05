//! Boundary lint (the audit wall's tripwire).
//!
//! `kx-audit` is an OFF-TRUTH-PATH, best-effort observability sink. The
//! guarantee-path crates — the frozen trio (`kx-scheduler`/`kx-executor`/
//! `kx-inference`) plus the journal / projection / memoizer truth-path set — MUST
//! NEVER depend on it nor import it. The real wall is the dependency graph (the
//! compiler enforces it every build); this test fails loudly the instant the wall
//! erodes — e.g. someone adds `kx-audit` to a guarantee-path crate to "conveniently"
//! emit audit events from inside the executor, which is itself the boundary
//! violation: the moment audit/observability influences scheduling/commit/identity,
//! the off-truth-path guarantee (and the `a6b5c679…` digest invariant) is gone.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};

/// Crates on the exactly-once guarantee / identity path. None may reach the audit
/// layer. Includes the full frozen trio (`kx-scheduler`/`kx-executor`/`kx-inference`).
const GUARANTEE_PATH_CRATES: [&str; 6] = [
    "kx-journal",
    "kx-scheduler",
    "kx-executor",
    "kx-inference",
    "kx-projection",
    "kx-memoizer",
];

/// `<repo>/crates` (`CARGO_MANIFEST_DIR` is `<repo>/crates/kx-audit`).
fn crates_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("kx-audit lives under crates/")
        .to_path_buf()
}

fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn guarantee_path_crates_do_not_depend_on_audit() {
    let crates = crates_dir();
    for krate in GUARANTEE_PATH_CRATES {
        // 1. Cargo.toml must not list kx-audit.
        let manifest = crates.join(krate).join("Cargo.toml");
        if let Ok(text) = fs::read_to_string(&manifest) {
            assert!(
                !text.contains("kx-audit"),
                "{krate}/Cargo.toml depends on kx-audit — the audit wall is breached"
            );
        }
        // 2. No source file may import it.
        let src = crates.join(krate).join("src");
        let mut files = Vec::new();
        collect_rs_files(&src, &mut files);
        for f in files {
            let text = fs::read_to_string(&f).unwrap();
            assert!(
                !text.contains("kx_audit") && !text.contains("kx-audit"),
                "{} imports kx-audit — the audit wall is breached",
                f.display()
            );
        }
    }
}
