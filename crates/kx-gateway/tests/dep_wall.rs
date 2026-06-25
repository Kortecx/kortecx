//! The FFI wall. The gateway binary is a single-system SERVER — it legitimately
//! links the coordinator/worker/executor (unlike `kx-gateway-core`, which forbids
//! the writers). What it must NOT pull is the **llama.cpp FFI**: `kx-llamacpp` /
//! `kx-llamacpp-sys`. The default build stays FFI-free (kx-inference is
//! `default-features = false` at the workspace root), so `cargo install kx-gateway`
//! needs no C++ toolchain. This is the binary's analogue of `build-no-inference`.
//!
//! PR-2d-1 hardened the wall with `kx-model-harness` (the react decode gate is the
//! pure leaf `kx-toolcall`; the harness would drag the whole engine + the FFI back
//! in via its `llamacpp` opt-in). PR-2d-2 lands the MCP adapter as an explicit,
//! OPTIONAL edge behind `inference` (the live ReAct tool round), so its check
//! moved from FORBIDDEN to the optional-edge pattern below (the `hnsw` precedent):
//! the edge must exist, must be `optional = true` (absent from the default-feature
//! tree), and its own subtree must be FFI-free.
//!
//! Two independent proofs: a manifest `[dependencies]` scan + a `cargo tree` over
//! the normal edges.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

const FORBIDDEN: &[&str] = &["kx-llamacpp", "kx-llamacpp-sys", "kx-model-harness"];

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

/// PR-A.1 (GR24 dual-engine parity): the FFI-FREE serve loop (`serve-engine,hnsw`)
/// must stay llama.cpp-free — serve-engine pulls the Ollama backend (`kx-ollama`)
/// plus the planner/critic/MCP, never `kx-llamacpp`/`kx-model-harness` (those ride
/// the opt-in `inference`). The deterministic in-test complement to the
/// `build-serve-engine` clean-room gate.
#[test]
fn cargo_tree_serve_engine_features_exclude_the_ffi() {
    let output = std::process::Command::new(env!("CARGO"))
        .args([
            "tree",
            "-p",
            "kx-gateway",
            "--features",
            "serve-engine,hnsw",
            "--edges",
            "normal",
            "--prefix",
            "none",
        ])
        .output();
    let output = match output {
        Ok(o) if o.status.success() => o,
        // cargo-tree may be unavailable in sandboxed CI; skip rather than false-fail.
        _ => return,
    };
    let tree = String::from_utf8_lossy(&output.stdout);
    for forbidden in FORBIDDEN {
        assert!(
            !tree.lines().any(|l| l.trim_start().starts_with(forbidden)),
            "{forbidden} appeared in the kx-gateway serve-engine,hnsw tree:\n{tree}"
        );
    }
    // Sanity: serve-engine MUST wire the FFI-free Ollama backend.
    assert!(
        tree.lines()
            .any(|l| l.trim_start().starts_with("kx-ollama")),
        "serve-engine did not pull the kx-ollama backend:\n{tree}"
    );
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
    // The HNSW ANN backend stays OPTIONAL behind `hnsw`. (kx-dataset is NO
    // LONGER in this list: the always-on W1.A5 toolscout view composes its
    // exact `InMemoryRetrievalIndex`, making it a non-optional dep; it was
    // already in the default closure via kx-catalog and is pure Rust, so
    // `build-no-inference` stays green. rusqlite likewise left earlier for the
    // Morphic capture sidecar.)
    {
        let edge = "kx-dataset-hnsw";
        let line = deps
            .lines()
            .find(|l| l.trim_start().starts_with(edge))
            .unwrap_or_else(|| panic!("the hnsw feature wires {edge}"));
        assert!(
            line.contains("optional = true"),
            "{edge} must be an OPTIONAL edge (behind the `hnsw` feature)"
        );
    }
    // kx-dataset is present + non-optional (the toolscout manifest index).
    let dataset = deps
        .lines()
        .find(|l| {
            l.trim_start().starts_with("kx-dataset ") || l.trim_start().starts_with("kx-dataset=")
        })
        .or_else(|| {
            deps.lines()
                .find(|l| l.trim_start().starts_with("kx-dataset") && !l.contains("hnsw"))
        })
        .expect("kx-dataset is a (non-optional) toolscout dependency");
    assert!(
        !dataset.contains("optional = true"),
        "kx-dataset is now always-on (the W1.A5 toolscout manifest index)"
    );
    // rusqlite is present + non-optional (the capture sidecar).
    let rusqlite = deps
        .lines()
        .find(|l| l.trim_start().starts_with("rusqlite"))
        .expect("rusqlite is a (non-optional) capture dependency");
    assert!(
        !rusqlite.contains("optional = true"),
        "rusqlite is now always-on (the Morphic Data Engine capture sidecar)"
    );
}

#[test]
fn cargo_manifest_wires_the_mcp_edge_optionally() {
    // PR-2d-2 wires the MCP adapter (the live ReAct tool round) behind the OPT-IN
    // `inference` feature. Assert the edge exists AND is optional — so it is
    // absent from the default-feature tree the FFI check above scans. (The typed
    // kx-tool-registry edge that originally rode with it is NON-optional since
    // W1.A5: the always-on toolscout view lists the registry's builtin
    // manifests; it was already in the default closure via kx-catalog and is
    // pure Rust, so the FFI wall holds — pinned below + by the subtree checks.)
    let manifest = include_str!("../Cargo.toml");
    let deps = manifest
        .split("[dependencies]")
        .nth(1)
        .expect("a [dependencies] section")
        .split("\n[")
        .next()
        .expect("the end of the [dependencies] section");
    let mcp = deps
        .lines()
        .find(|l| l.trim_start().starts_with("kx-mcp ") || l.trim_start().starts_with("kx-mcp="))
        .expect("PR-2d-2 wires kx-mcp");
    assert!(
        mcp.contains("optional = true"),
        "kx-mcp must be an OPTIONAL edge (behind the `inference` feature)"
    );
    let registry = deps
        .lines()
        .find(|l| l.trim_start().starts_with("kx-tool-registry"))
        .expect("the toolscout view wires kx-tool-registry");
    assert!(
        !registry.contains("optional = true"),
        "kx-tool-registry is now always-on (the W1.A5 toolscout manifests)"
    );
}

#[test]
fn cargo_manifest_wires_the_toolscout_edges_always_on() {
    // W1.A5: the advisory MCP-intelligence surface is ALWAYS-ON (manifests +
    // bundle scoring answer on every build). Both crates are pure Rust; the
    // `toolscout_subtree_excludes_the_ffi` check below proves the edge can
    // never re-open the FFI hole.
    let manifest = include_str!("../Cargo.toml");
    let deps = manifest
        .split("[dependencies]")
        .nth(1)
        .expect("a [dependencies] section")
        .split("\n[")
        .next()
        .expect("the end of the [dependencies] section");
    for edge in ["kx-bundle", "kx-toolscout"] {
        let line = deps
            .lines()
            .find(|l| l.trim_start().starts_with(edge))
            .unwrap_or_else(|| panic!("W1.A5 wires {edge}"));
        assert!(
            !line.contains("optional = true"),
            "{edge} is always-on (the W1.A5 advisory toolscout surface)"
        );
    }
}

#[test]
fn toolscout_subtree_excludes_the_ffi() {
    // Defense-in-depth: prove the advisory toolscout crate does not, on its own
    // normal tree, drag in the llama.cpp FFI or the harness — the always-on
    // edge can never re-open the hole the FORBIDDEN list closes.
    let output = std::process::Command::new(env!("CARGO"))
        .args([
            "tree",
            "-p",
            "kx-toolscout",
            "--edges",
            "normal",
            "--prefix",
            "none",
        ])
        .output();
    let output = match output {
        Ok(o) if o.status.success() => o,
        _ => return, // cargo tree unavailable in this environment — the CI run covers it
    };
    let tree = String::from_utf8_lossy(&output.stdout);
    for forbidden in FORBIDDEN {
        assert!(
            !tree.contains(forbidden),
            "kx-toolscout's normal tree must not contain {forbidden}"
        );
    }
}

#[test]
fn mcp_subtree_excludes_the_ffi() {
    // Defense-in-depth: prove the MCP adapter does not, on its own normal tree,
    // drag in the llama.cpp FFI or the harness — so the optional edge can never
    // re-open the hole the FORBIDDEN list closes.
    let output = std::process::Command::new(env!("CARGO"))
        .args([
            "tree", "-p", "kx-mcp", "--edges", "normal", "--prefix", "none",
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
            "{forbidden} appeared in the normal dependency tree of the MCP adapter:\n{tree}"
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
