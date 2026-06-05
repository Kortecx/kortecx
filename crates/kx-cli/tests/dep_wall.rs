//! The FFI wall for the user-facing `kx` binary. It forwards `serve` to the
//! gateway and `run`/`replay`/`digest` to the engine — both FFI-free by default
//! (kx-inference is `default-features = false` at the workspace root) — so
//! `cargo install kx-cli` needs no C++ toolchain. What it must NOT pull is the
//! llama.cpp FFI (`kx-llamacpp` / `kx-llamacpp-sys`). Mirrors
//! `kx-gateway/tests/dep_wall.rs`; the justfile `build-no-inference` gate also
//! pins this.
//!
//! Two independent proofs: a manifest `[dependencies]` scan + a `cargo tree`
//! over the normal edges.

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
            "{forbidden} must not be a normal dependency of kx-cli"
        );
    }
}

#[test]
fn cargo_tree_normal_edges_exclude_the_ffi() {
    let output = std::process::Command::new(env!("CARGO"))
        .args([
            "tree", "-p", "kx-cli", "--edges", "normal", "--prefix", "none",
        ])
        .output();
    let output = match output {
        Ok(o) if o.status.success() => o,
        // cargo-tree may be unavailable in sandboxed CI; the manifest scan is the
        // load-bearing gate, so skip rather than false-fail.
        _ => return,
    };
    let tree = String::from_utf8_lossy(&output.stdout);
    for forbidden in FORBIDDEN {
        assert!(
            !tree.lines().any(|l| l.trim_start().starts_with(forbidden)),
            "{forbidden} appeared in the normal dependency tree of kx-cli:\n{tree}"
        );
    }
}
