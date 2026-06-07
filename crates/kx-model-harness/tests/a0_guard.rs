//! A0 — the default-preservation guard.
//!
//! The `run_with_seams` seam must NOT change the canonical demo: `kx_runtime::run`
//! still drives the stub seams to the byte-identical projection digest
//! `7d22d4bd…` with 8/8 committed. If this fails, the seam altered the truth path.

#![allow(clippy::unwrap_used)]

use kx_runtime::config::Mode;
use kx_runtime::RuntimeConfig;

/// The canonical projection digest after the P4.2 v5 bump (the campaign control).
const CONTROL_DIGEST: &str = "7d22d4bdfc6f68a4311f40b20f3fe7c67f4c5d2b352f3bff8722b439e94a5af9";

#[test]
fn seam_preserves_canonical_demo_digest() {
    let dir = tempfile::tempdir().unwrap();
    let config = RuntimeConfig {
        journal_path: dir.path().join("j.sqlite"),
        content_root: dir.path().join("c"),
        mode: Mode::Run,
        crash_at: None,
        // M2.2b: force checkpoint writing mid-run (cadence 2 over an 8-Mote
        // demo) — the canonical product digest must be UNCHANGED with the
        // discardable checkpoint live (the checkpoint never touches the truth
        // path). This is test T9 of the M2.2b matrix.
        checkpoint_every: Some(2),
        audit_log: None,
    };
    let outcome = kx_runtime::run(&config).unwrap();
    assert_eq!(outcome.committed, 8, "committed count");
    assert_eq!(outcome.total, 8, "total count");
    assert_eq!(
        outcome.digest.to_hex(),
        CONTROL_DIGEST,
        "run_with_seams must preserve the canonical demo digest (default-preserving seam, \
         checkpointing on)"
    );
}
