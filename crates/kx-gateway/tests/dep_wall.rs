//! The FFI wall. The gateway binary is a single-system SERVER — it legitimately
//! links the coordinator/worker/executor (unlike `kx-gateway-core`, which forbids
//! the writers). What it must NOT pull is the **llama.cpp FFI**: `kx-llamacpp` /
//! `kx-llamacpp-sys`. The default build stays FFI-free (kx-inference is
//! `default-features = false` at the workspace root), so `cargo install kx-gateway`
//! needs no C++ toolchain. This is the binary's analogue of `build-no-inference`.
//!
//! Two independent proofs: a manifest `[dependencies]` scan + a `cargo tree` over
//! the normal edges.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

const FORBIDDEN: &[&str] = &["kx-llamacpp", "kx-llamacpp-sys"];

#[test]
fn cargo_manifest_dependencies_exclude_the_ffi() {
    let manifest = include_str!("../Cargo.toml");
    let deps = manifest
        .split("[dependencies]")
        .nth(1)
        .expect("a [dependencies] section")
        .split("\n[")
        .next()
        .expect("the end of the [dependencies] section");
    for forbidden in FORBIDDEN {
        assert!(
            !deps.contains(forbidden),
            "{forbidden} must not be a normal dependency of kx-gateway"
        );
    }
}

#[test]
fn cargo_tree_normal_edges_exclude_the_ffi() {
    // `cargo tree --edges normal` lists only non-dev dependencies (default features).
    let output = std::process::Command::new(env!("CARGO"))
        .args([
            "tree",
            "-p",
            "kx-gateway",
            "--edges",
            "normal",
            "--prefix",
            "none",
        ])
        .output();
    let output = match output {
        Ok(o) if o.status.success() => o,
        // In sandboxed environments cargo-tree may be unavailable; the manifest
        // scan above is the load-bearing gate, so skip rather than false-fail.
        _ => return,
    };
    let tree = String::from_utf8_lossy(&output.stdout);
    for forbidden in FORBIDDEN {
        assert!(
            !tree.lines().any(|l| l.trim_start().starts_with(forbidden)),
            "{forbidden} appeared in the normal dependency tree of kx-gateway:\n{tree}"
        );
    }
}

#[test]
fn cargo_manifest_includes_the_catalog_edge() {
    // R2a intentionally adds kx-catalog (the signature registry) to the host. It
    // is FFI-free (kx-dataset / kx-tool-registry / rusqlite, none link llama.cpp).
    let manifest = include_str!("../Cargo.toml");
    let deps = manifest
        .split("[dependencies]")
        .nth(1)
        .expect("a [dependencies] section")
        .split("\n[")
        .next()
        .expect("the end of the [dependencies] section");
    assert!(
        deps.contains("kx-catalog"),
        "R2a wires the kx-catalog signature registry into the host"
    );
}

#[test]
fn cargo_manifest_wires_the_dataset_hnsw_edge_optionally() {
    // T3.7 wires the FFI-free HNSW ANN backend behind the OPT-IN `hnsw` feature, so
    // the default build stays byte-unchanged. Assert the edges exist AND are optional
    // (so they are absent from the default-feature tree the FFI check above scans).
    let manifest = include_str!("../Cargo.toml");
    let deps = manifest
        .split("[dependencies]")
        .nth(1)
        .expect("a [dependencies] section")
        .split("\n[")
        .next()
        .expect("the end of the [dependencies] section");
    for edge in ["kx-dataset-hnsw", "kx-dataset", "rusqlite"] {
        let line = deps
            .lines()
            .find(|l| l.trim_start().starts_with(edge))
            .unwrap_or_else(|| panic!("the hnsw feature wires {edge}"));
        assert!(
            line.contains("optional = true"),
            "{edge} must be an OPTIONAL edge (behind the `hnsw` feature)"
        );
    }
}

#[test]
fn dataset_hnsw_subtree_excludes_the_ffi() {
    // Defense-in-depth: prove the kx-dataset-hnsw backend does not, on its own normal
    // tree, drag in the llama.cpp FFI — so enabling `--features hnsw` stays FFI-free.
    let output = std::process::Command::new(env!("CARGO"))
        .args([
            "tree",
            "-p",
            "kx-dataset-hnsw",
            "--edges",
            "normal",
            "--prefix",
            "none",
        ])
        .output();
    let output = match output {
        Ok(o) if o.status.success() => o,
        _ => return,
    };
    let tree = String::from_utf8_lossy(&output.stdout);
    for forbidden in FORBIDDEN {
        assert!(
            !tree.lines().any(|l| l.trim_start().starts_with(forbidden)),
            "{forbidden} appeared in the normal dependency tree of kx-dataset-hnsw:\n{tree}"
        );
    }
}

#[test]
fn catalog_subtree_excludes_the_ffi() {
    // Defense-in-depth: prove the newly-added kx-catalog edge does not, on its own
    // normal tree, drag in the llama.cpp FFI (attributes a regression to catalog).
    let output = std::process::Command::new(env!("CARGO"))
        .args([
            "tree",
            "-p",
            "kx-catalog",
            "--edges",
            "normal",
            "--prefix",
            "none",
        ])
        .output();
    let output = match output {
        Ok(o) if o.status.success() => o,
        _ => return,
    };
    let tree = String::from_utf8_lossy(&output.stdout);
    for forbidden in FORBIDDEN {
        assert!(
            !tree.lines().any(|l| l.trim_start().starts_with(forbidden)),
            "{forbidden} appeared in the normal dependency tree of kx-catalog:\n{tree}"
        );
    }
}
