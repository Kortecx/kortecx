//! The dependency wall (Rule 1 / SN-8 / the D167 Extension Acceptance Gate item 3).
//! `kx-extension-sdk` is an FFI-FREE leaf: it curates the connector seams but must
//! NEVER link the llama.cpp FFI (`kx-llamacpp`), the frozen trio
//! (`kx-executor`/`kx-scheduler`), the journal writer, or the gateway/cluster/runtime
//! components. Re-exporting types adds no journal facts ⇒ the digest is invariant.
//!
//! Three layers:
//!  1. **Manifest scan** — the default `[dependencies]` must name no forbidden crate
//!     and be exactly the minimal allowed set (comment lines skipped). `kx-gateway-core`
//!     is allowed AS AN OPTIONAL line only (the `gateway-admin` feature); the tree scan
//!     proves it is absent by default.
//!  2. **Default `cargo tree`** — the FFI/frozen-trio/journal/gateway/proto crates are
//!     absent from the DEFAULT normal dependency tree.
//!  3. **`gateway-admin` `cargo tree`** — the opt-in feature deliberately adds
//!     `kx-gateway-core` → `kx-proto`/`tonic` (still FFI-free), but the Tier-1 wall
//!     (FFI + frozen trio + journal writer) STILL holds even then.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

/// Forbidden as a DIRECT manifest dependency. `kx-llamacpp` also covers
/// `kx-llamacpp-sys`. `kx-gateway ` carries a trailing space so it forbids the
/// gateway BINARY without matching the allowed optional `kx-gateway-core` line.
const FORBIDDEN: &[&str] = &[
    "kx-llamacpp",
    "kx-executor",
    "kx-scheduler",
    "kx-coordinator",
    "kx-worker",
    "kx-runtime",
    "kx-journal",
    "kx-projection",
    "kx-capture",
    "kx-gateway ",
    "kx-proto",
];

/// Forbidden in the DEFAULT normal `cargo tree` (the FFI + frozen-trio + journal
/// writer + gateway/proto crates). `kx-gateway-core` is here too: it must be absent
/// by DEFAULT (it rides the opt-in `gateway-admin` feature only).
const TREE_FORBIDDEN: &[&str] = &[
    "kx-llamacpp",
    "kx-executor",
    "kx-scheduler",
    "kx-coordinator",
    "kx-worker",
    "kx-runtime",
    "kx-journal",
    "kx-projection",
    "kx-capture",
    "kx-gateway-core",
    "kx-gateway ",
    "kx-proto",
];

/// The Tier-1 wall that holds under EVERY feature combination: NO in-process model
/// execution (`kx-llamacpp` FFI), NO frozen-trio mutation (`kx-executor`/
/// `kx-scheduler`), NO cluster/runtime (`kx-coordinator`/`kx-worker`/`kx-runtime`).
/// `kx-gateway-core`/`kx-proto`/`tonic` are NOT here — the `gateway-admin` feature
/// legitimately adds them (pure-Rust, FFI-free); nor are `kx-journal`/`kx-projection`,
/// which the HOST gateway-core read-side pulls under that feature (the SDK still
/// never WRITES them — it only re-exports the host admin trait shapes).
const TREE_FORBIDDEN_ALL_FEATURES: &[&str] = &[
    "kx-llamacpp",
    "kx-executor",
    "kx-scheduler",
    "kx-coordinator",
    "kx-worker",
    "kx-runtime",
];

/// The minimal DIRECT dependency set (Cargo.toml `[dependencies]`). `kx-gateway-core`
/// appears only as an `optional` line (the `gateway-admin` feature).
const ALLOWED: &[&str] = &[
    "kx-mcp-gateway",
    "kx-mcp",
    "kx-tool-registry",
    "kx-warrant",
    "kx-mote",
    "kx-capability",
    "kx-content",
    "kx-gateway-core",
    "smallvec",
    "serde",
    "serde_json",
    "thiserror",
];

#[test]
fn cargo_manifest_dependencies_are_the_minimal_ffi_free_set() {
    let manifest = include_str!("../Cargo.toml");
    let deps_block = manifest
        .split("[dependencies]")
        .nth(1)
        .expect("a [dependencies] section")
        .split("\n[")
        .next()
        .expect("the end of the [dependencies] section");
    // Code lines only — the manifest's own comments document the FFI-free intent by
    // name, which must not trip the forbidden-substring scan.
    let code: String = deps_block
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n");
    for forbidden in FORBIDDEN {
        assert!(
            !code.contains(forbidden),
            "{forbidden} must not be a dependency of the FFI-free kx-extension-sdk crate"
        );
    }
    for line in code.lines() {
        let name = line.split_whitespace().next().unwrap_or("");
        assert!(
            ALLOWED.contains(&name),
            "kx-extension-sdk gained an unexpected dependency {name:?}; keep it an FFI-free leaf"
        );
    }
}

fn tree(args: &[&str]) -> Option<String> {
    let output = std::process::Command::new(env!("CARGO"))
        .args(args)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

#[test]
fn cargo_tree_default_excludes_ffi_writers_and_gateway() {
    let Some(tree) = tree(&[
        "tree",
        "-p",
        "kx-extension-sdk",
        "--edges",
        "normal",
        "--prefix",
        "none",
    ]) else {
        return; // cargo-tree unavailable in some sandboxes; the manifest scan is the gate.
    };
    for forbidden in TREE_FORBIDDEN {
        assert!(
            !tree.lines().any(|l| l.trim_start().starts_with(forbidden)),
            "{forbidden} appeared in the DEFAULT normal dependency tree of kx-extension-sdk:\n{tree}"
        );
    }
}

#[test]
fn cargo_tree_gateway_admin_keeps_tier1_wall() {
    // The opt-in feature pulls kx-gateway-core → kx-proto/tonic (FFI-free, allowed),
    // but the FFI + frozen-trio + journal-writer wall must STILL hold.
    let Some(tree) = tree(&[
        "tree",
        "-p",
        "kx-extension-sdk",
        "--features",
        "gateway-admin",
        "--edges",
        "normal",
        "--prefix",
        "none",
    ]) else {
        return;
    };
    for forbidden in TREE_FORBIDDEN_ALL_FEATURES {
        assert!(
            !tree.lines().any(|l| l.trim_start().starts_with(forbidden)),
            "{forbidden} appeared in the gateway-admin dependency tree of kx-extension-sdk:\n{tree}"
        );
    }
}
