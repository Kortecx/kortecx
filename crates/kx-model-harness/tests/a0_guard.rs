//! A0 — the default-preservation guard.
//!
//! The `run_with_seams` seam must NOT change the canonical demo: `kx_runtime::run`
//! still drives the stub seams to the byte-identical projection digest
//! `a6b5c679…` with 8/8 committed. If this fails, the seam altered the truth path.

#![allow(clippy::unwrap_used)]

use kx_runtime::config::Mode;
use kx_runtime::RuntimeConfig;

/// The canonical projection digest after the P4.2 v5 bump (the campaign control).
const CONTROL_DIGEST: &str = "a6b5c67939f14bfcbd125f7461b2bd0e481f6ee2fc98c1ab638730e2d2ace2e9";

#[test]
fn seam_preserves_canonical_demo_digest() {
    let dir = tempfile::tempdir().unwrap();
    let config = RuntimeConfig {
        journal_path: dir.path().join("j.sqlite"),
        content_root: dir.path().join("c"),
        mode: Mode::Run,
        crash_at: None,
    };
    let outcome = kx_runtime::run(&config).unwrap();
    assert_eq!(outcome.committed, 8, "committed count");
    assert_eq!(outcome.total, 8, "total count");
    assert_eq!(
        outcome.digest.to_hex(),
        CONTROL_DIGEST,
        "run_with_seams must preserve the canonical demo digest (default-preserving seam)"
    );
}
