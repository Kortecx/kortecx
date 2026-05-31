//! Boundary lint (the wall's tripwire).
//!
//! `kx-capture` is an OPT-IN, OFF-TRUTH-PATH projection. The guarantee-path
//! crates MUST NEVER depend on it nor import the capture projection. The real
//! wall is the dependency graph (the compiler enforces it every build); this test
//! is a tripwire that fails loudly the instant the wall erodes — e.g. someone
//! adds a `kx-capture` dependency to a guarantee-path crate to "conveniently"
//! capture thinking inside the executor (which is itself the boundary violation:
//! the moment capture/retention influences scheduling/commit/identity, the
//! reuse-the-action-never-the-thinking guarantee is gone).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};

/// Crates on the exactly-once guarantee / identity path. None may reach the
/// capture layer.
const GUARANTEE_PATH_CRATES: [&str; 5] = [
    "kx-journal",
    "kx-scheduler",
    "kx-executor",
    "kx-projection",
    "kx-memoizer",
];

/// `<repo>/crates` (`CARGO_MANIFEST_DIR` is `<repo>/crates/kx-capture`).
fn crates_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("kx-capture lives under crates/")
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
fn guarantee_path_crates_do_not_depend_on_capture() {
    let crates = crates_dir();
    for krate in GUARANTEE_PATH_CRATES {
        // 1. Cargo.toml must not list kx-capture.
        let manifest = crates.join(krate).join("Cargo.toml");
        if let Ok(text) = fs::read_to_string(&manifest) {
            assert!(
                !text.contains("kx-capture"),
                "{krate}/Cargo.toml depends on kx-capture — the capture wall is breached"
            );
        }
        // 2. No source file may import it.
        let src = crates.join(krate).join("src");
        let mut files = Vec::new();
        collect_rs_files(&src, &mut files);
        for f in files {
            let text = fs::read_to_string(&f).unwrap();
            assert!(
                !text.contains("kx_capture") && !text.contains("kx-capture"),
                "{} imports kx-capture — the capture wall is breached",
                f.display()
            );
        }
    }
}
