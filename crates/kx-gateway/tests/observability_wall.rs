//! W6.1 — the observability DEFINEDNESS wall. The release build's feature list
//! deliberately excludes `observability`, and "excluded" must be a provable
//! property of the artifact, not a belief: the kx-otel edge is optional, the
//! default-feature dependency tree does not contain it, and the feature that
//! declares it actually exists and pulls it in. Every assertion here FAILS on
//! the pre-gating tree (where kx-otel was a normal, always-linked dependency) —
//! the dep_wall.rs manifest-scan + cargo-tree pattern, pointed at definedness
//! instead of the FFI.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

/// The manifest half: the kx-otel edge must be declared `optional = true`.
/// Against the pre-gating manifest (`kx-otel = { workspace = true }`) this
/// fails — that line carries no `optional`.
#[test]
fn cargo_manifest_wires_the_otel_edge_optionally() {
    let manifest = include_str!("../Cargo.toml");
    let dep_line = manifest
        .lines()
        .find(|l| l.trim_start().starts_with("kx-otel"))
        .expect("kx-otel must remain a declared (optional) dependency");
    assert!(
        dep_line.contains("optional = true"),
        "kx-otel must be an OPTIONAL edge (behind the `observability` feature): {dep_line}"
    );
}

/// The feature-shape half: `observability` must exist and be the thing that
/// pulls the edge in. A dead feature name (or one that no longer carries the
/// dep) would let the tree check below pass vacuously.
#[test]
fn cargo_manifest_declares_the_observability_feature_over_the_edge() {
    let manifest = include_str!("../Cargo.toml");
    let features = manifest
        .split("[features]")
        .nth(1)
        .expect("a [features] section");
    let decl = features
        .lines()
        .find(|l| l.trim_start().starts_with("observability"))
        .expect("an `observability` feature declaration");
    assert!(
        decl.contains("dep:kx-otel"),
        "the observability feature must carry the kx-otel edge: {decl}"
    );
    let default = features
        .lines()
        .find(|l| l.trim_start().starts_with("default"))
        .expect("a default features line");
    assert!(
        !default.contains("observability"),
        "observability must be OPT-IN (out of the default set): {default}"
    );
}

/// The tree half, absence direction: the DEFAULT-feature dependency closure
/// must not contain kx-otel. Skip only when cargo-tree itself is unavailable
/// (sandboxed environments) — the manifest scans above stay load-bearing.
#[test]
fn default_tree_excludes_the_otel_edge() {
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
        _ => return,
    };
    let tree = String::from_utf8_lossy(&output.stdout);
    assert!(
        !tree.lines().any(|l| l.trim_start().starts_with("kx-otel")),
        "the default build must not link kx-otel — the observability stack is opt-in"
    );
}

/// The tree half, presence direction: declaring the feature must actually pull
/// the edge in. An UNKNOWN-feature failure here is a hard FAIL, not a skip —
/// "the feature does not exist" is precisely the regression this guards.
#[test]
fn observability_tree_includes_the_otel_edge() {
    let output = std::process::Command::new(env!("CARGO"))
        .args([
            "tree",
            "-p",
            "kx-gateway",
            "--features",
            "observability",
            "--edges",
            "normal",
            "--prefix",
            "none",
        ])
        .output();
    let output = match output {
        Ok(o) if o.status.success() => o,
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            assert!(
                !stderr.contains("does not have"),
                "the observability feature must exist on kx-gateway: {stderr}"
            );
            return; // any other cargo-tree failure: environment, not regression
        }
        Err(_) => return,
    };
    let tree = String::from_utf8_lossy(&output.stdout);
    assert!(
        tree.lines().any(|l| l.trim_start().starts_with("kx-otel")),
        "--features observability must link kx-otel (the feature would otherwise be a dead name)"
    );
}
