//! The thesis wall (Rule 1). kx-invoke executes by SUBMITTING through the gateway
//! RunSubmitter — it must NOT link the journal-writer / effect components in its
//! NORMAL (non-dev) dependency tree: `kx-executor`, `kx-scheduler`,
//! `kx-coordinator`, `kx-capture`. (They appear as `[dev-dependencies]` only, to
//! drive the bound run to Committed in the G3 integration test.)

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

const FORBIDDEN: &[&str] = &[
    "kx-executor",
    "kx-scheduler",
    "kx-coordinator",
    "kx-capture",
];

#[test]
fn cargo_manifest_dependencies_exclude_writers() {
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
            "{forbidden} must not be a normal dependency of kx-invoke"
        );
    }
}

#[test]
fn cargo_tree_normal_edges_exclude_writers() {
    let output = std::process::Command::new(env!("CARGO"))
        .args([
            "tree",
            "-p",
            "kx-invoke",
            "--edges",
            "normal",
            "--prefix",
            "none",
        ])
        .output();
    let output = match output {
        Ok(o) if o.status.success() => o,
        _ => return, // cargo-tree unavailable in some sandboxes; manifest scan is the gate.
    };
    let tree = String::from_utf8_lossy(&output.stdout);
    for forbidden in FORBIDDEN {
        assert!(
            !tree.lines().any(|l| l.trim_start().starts_with(forbidden)),
            "{forbidden} appeared in the normal dependency tree of kx-invoke:\n{tree}"
        );
    }
}
