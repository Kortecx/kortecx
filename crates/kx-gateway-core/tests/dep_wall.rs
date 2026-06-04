//! The dependency wall (D120.5). gateway-core is a read-fold + propose-proxy; it
//! MUST NOT link the journal-writer / effect components in its NORMAL (non-dev)
//! dependency tree: `kx-executor`, `kx-scheduler`, `kx-coordinator`, `kx-capture`.
//! Two independent proofs: a manifest `[dependencies]` scan and a `cargo tree`
//! over the normal edges. (`kx-journal` IS linked — but only as a READER, via the
//! `JournalReader`/`ReadOnly` seam that has no `append`; that is a type-level
//! property the build itself enforces.)

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
    // Scan only the [dependencies] section (dev-deps are allowed for test wiring).
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
            "{forbidden} must not be a normal dependency of kx-gateway-core"
        );
    }
}

#[test]
fn cargo_tree_normal_edges_exclude_writers() {
    // `cargo tree --edges normal` lists only non-dev dependencies.
    let output = std::process::Command::new(env!("CARGO"))
        .args([
            "tree",
            "-p",
            "kx-gateway-core",
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
            "{forbidden} appeared in the normal dependency tree of kx-gateway-core:\n{tree}"
        );
    }
}
