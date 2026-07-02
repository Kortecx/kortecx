//! The leaf wall (Rule 1 / SN-8). `kx-skill` is a pure format TYPE crate — the
//! skill manifest shape only. It must NEVER link the journal-writer / runtime /
//! gateway / frozen-trio components, so a Mote/journal/digest change can never
//! reach it and it can never reach them. A skill is declarative by construction;
//! the crate that DEFINES the format must be structurally incapable of executing
//! or granting anything.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

const FORBIDDEN: &[&str] = &[
    "kx-executor",
    "kx-scheduler",
    "kx-inference",
    "kx-journal",
    "kx-projection",
    "kx-runtime",
    "kx-coordinator",
    "kx-worker",
    "kx-gateway",
    "kx-gateway-core",
    "kx-proto",
];

/// The minimal leaf dependency set (Cargo.toml `[dependencies]`).
const ALLOWED: &[&str] = &["serde", "serde_json", "thiserror"];

#[test]
fn cargo_manifest_dependencies_are_the_minimal_leaf_set() {
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
            "{forbidden} must not be a dependency of the kx-skill leaf crate"
        );
    }
    for line in deps.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let name = line.split_whitespace().next().unwrap_or("");
        assert!(
            ALLOWED.contains(&name),
            "kx-skill gained an unexpected dependency {name:?}; keep it a pure leaf"
        );
    }
}

#[test]
fn cargo_tree_normal_edges_exclude_writers() {
    let output = std::process::Command::new(env!("CARGO"))
        .args([
            "tree", "-p", "kx-skill", "--edges", "normal", "--prefix", "none",
        ])
        .output();
    let output = match output {
        Ok(o) if o.status.success() => o,
        _ => return, // cargo-tree unavailable in some sandboxes; the manifest scan is the gate.
    };
    let tree = String::from_utf8_lossy(&output.stdout);
    for forbidden in FORBIDDEN {
        assert!(
            !tree.lines().any(|l| l.trim_start().starts_with(forbidden)),
            "{forbidden} appeared in the normal dependency tree of kx-skill:\n{tree}"
        );
    }
}
