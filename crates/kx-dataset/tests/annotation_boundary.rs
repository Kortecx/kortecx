//! Boundary lint (the wall's tripwire).
//!
//! `AnnotationStore` is an advisory, off-truth-path projection. The guarantee-path
//! crates (`kx-executor`, `kx-projection`, `kx-scheduler`) MUST NEVER depend on
//! `kx-dataset` nor import the annotation projection. The real wall is the
//! dependency graph (the compiler enforces it every build); this test is a tripwire
//! that fails loudly the instant the wall erodes — e.g. someone adds a `kx-dataset`
//! dependency to move curation "closer" to the executor (which is itself the SN-8
//! violation).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};

/// Crates on the exactly-once guarantee path. None may reach the annotation layer.
const GUARANTEE_PATH_CRATES: [&str; 3] = ["kx-executor", "kx-projection", "kx-scheduler"];

/// `<repo>/crates` (`CARGO_MANIFEST_DIR` is `<repo>/crates/kx-dataset`).
fn crates_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("kx-dataset lives under crates/")
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
fn guarantee_path_crates_do_not_depend_on_kx_dataset() {
    let crates = crates_dir();
    for guard in GUARANTEE_PATH_CRATES {
        let cargo = crates.join(guard).join("Cargo.toml");
        let toml =
            fs::read_to_string(&cargo).unwrap_or_else(|e| panic!("read {}: {e}", cargo.display()));
        assert!(
            !toml.contains("kx-dataset"),
            "{guard}/Cargo.toml depends on kx-dataset. The annotation projection is an \
             off-truth-path advisory layer; the guarantee path MUST NOT reach it. \
             Moving curation 'closer' to the executor is itself the SN-8 violation."
        );
    }
}

#[test]
fn guarantee_path_crates_do_not_reference_the_dataset_or_annotation_layer() {
    let crates = crates_dir();
    for guard in GUARANTEE_PATH_CRATES {
        let src = crates.join(guard).join("src");
        let mut files = Vec::new();
        collect_rs_files(&src, &mut files);
        assert!(
            !files.is_empty(),
            "expected source files under {}",
            src.display()
        );
        for f in &files {
            let body = fs::read_to_string(f).unwrap_or_default();
            assert!(
                !body.contains("kx_dataset") && !body.contains("AnnotationStore"),
                "{} references the dataset / annotation layer — forbidden on the \
                 guarantee path (SN-8 wall: a usefulness score must never reach a gate)",
                f.display()
            );
        }
    }
}
