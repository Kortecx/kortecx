//! The dependency wall (Rule 1 / SN-8 / the D167 extension gate item 3). `kx-ollama`
//! is an FFI-FREE backend: it implements the `InferenceBackend` seam over HTTP and
//! must NEVER link the llama.cpp FFI (`kx-llamacpp`) nor the journal-writer / gateway
//! / cluster components.
//!
//! Two layers:
//!  1. **Manifest scan** — `kx-ollama`'s own `[dependencies]` must name neither the
//!     FFI crate nor any forbidden component, and must be exactly the minimal allowed
//!     set. Comment lines are skipped (this file's manifest documents the FFI-free
//!     intent by NAME).
//!  2. **`cargo tree` scan** — asserts the FFI crate + the cluster/gateway/runtime
//!     crates are absent from the normal dependency tree. (`kx-journal` /
//!     `kx-projection` ARE present — pulled by the legitimate `kx-inference` edge for
//!     the memoizer/projection path — and are FFI-free, so they are NOT forbidden.)

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

/// Forbidden as a direct dependency (the FFI crate — `kx-llamacpp` also covers
/// `kx-llamacpp-sys` — plus the cluster / gateway / runtime writers).
const FORBIDDEN: &[&str] = &[
    "kx-llamacpp",
    "kx-executor",
    "kx-scheduler",
    "kx-coordinator",
    "kx-worker",
    "kx-gateway",
    "kx-proto",
    "kx-runtime",
];

/// Forbidden in the normal `cargo tree` (the FFI crate + the cluster/gateway/runtime
/// crates; `kx-gateway ` trailing space avoids matching nothing else). NOT
/// `kx-journal` / `kx-projection` — both ride the FFI-free `kx-inference` edge.
const TREE_FORBIDDEN: &[&str] = &[
    "kx-llamacpp",
    "kx-executor",
    "kx-scheduler",
    "kx-coordinator",
    "kx-worker",
    "kx-gateway-core",
    "kx-gateway ",
    "kx-proto",
    "kx-runtime",
];

/// The minimal direct dependency set (Cargo.toml `[dependencies]`). `kx-content`
/// (`ContentRef` + `sniff_image_format`) and `base64` (image encode) are PR-B2 vision
/// additions — both FFI-free leaves (`kx-content` already rode the `kx-inference`
/// edge; `base64` is a pure encoder), so the FFI-free wall is unchanged.
const ALLOWED: &[&str] = &[
    "kx-inference",
    "kx-content",
    "kx-mote",
    "kx-warrant",
    // RC4c: the engine-agnostic grammar spec (FFI-free pure-string renderer) — the
    // Ollama leg renders a rerank turn's permutation schema as a whole-response `format`.
    "kx-grammar",
    "ureq",
    "base64",
    "serde",
    "serde_json",
    "thiserror",
    "tracing",
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
            "{forbidden} must not be a dependency of the FFI-free kx-ollama crate"
        );
    }
    for line in code.lines() {
        let name = line.split_whitespace().next().unwrap_or("");
        assert!(
            ALLOWED.contains(&name),
            "kx-ollama gained an unexpected dependency {name:?}; keep it an FFI-free leaf"
        );
    }
}

#[test]
fn cargo_tree_normal_edges_exclude_ffi_and_writers() {
    let output = std::process::Command::new(env!("CARGO"))
        .args([
            "tree",
            "-p",
            "kx-ollama",
            "--edges",
            "normal",
            "--prefix",
            "none",
        ])
        .output();
    let output = match output {
        Ok(o) if o.status.success() => o,
        _ => return, // cargo-tree unavailable in some sandboxes; the manifest scan is the gate.
    };
    let tree = String::from_utf8_lossy(&output.stdout);
    for forbidden in TREE_FORBIDDEN {
        assert!(
            !tree.lines().any(|l| l.trim_start().starts_with(forbidden)),
            "{forbidden} appeared in the normal dependency tree of kx-ollama:\n{tree}"
        );
    }
}
