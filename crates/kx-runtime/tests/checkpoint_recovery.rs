//! M2.2b — live checkpoint recovery wiring (in-process).
//!
//! Complements the subprocess `kill_and_replay` checkpoint scenarios: these run
//! the engine in-process so they can inspect the persisted sidecar and drive the
//! `kx_projection` recovery API directly, asserting the structured
//! [`CheckpointOutcome`] and the bit-identical (state-digest) guarantee.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::path::Path;

use kx_journal::{Journal, SqliteJournal};
use kx_projection::{CheckpointOutcome, FoldCheckpoint, Projection};
use kx_runtime::checkpoint_io;
use kx_runtime::config::Mode;
use kx_runtime::RuntimeConfig;

fn cfg(dir: &Path, checkpoint_every: Option<u64>) -> RuntimeConfig {
    RuntimeConfig {
        journal_path: dir.join("journal.sqlite"),
        content_root: dir.join("content"),
        mode: Mode::Run,
        crash_at: None,
        checkpoint_every,
    }
}

/// **T6** — first run, no pre-existing sidecar: recovery full-folds the (empty)
/// journal and the run completes 8/8. With the cadence on, a fresh sidecar is
/// left behind (the graceful-completion checkpoint).
#[test]
fn first_run_no_sidecar_completes_and_writes_one() {
    let dir = tempfile::tempdir().unwrap();
    let c = cfg(dir.path(), Some(2));
    assert!(!checkpoint_io::sidecar_path(&c.journal_path).exists());

    let outcome = kx_runtime::run(&c).unwrap();
    assert_eq!((outcome.committed, outcome.total), (8, 8));

    // The completed run leaves a usable sidecar folded through the final head.
    let sidecar = checkpoint_io::sidecar_path(&c.journal_path);
    let cp = checkpoint_io::read_checkpoint(&sidecar).expect("a sidecar after a completed run");
    let head = SqliteJournal::open(&c.journal_path)
        .unwrap()
        .current_seq()
        .unwrap();
    assert_eq!(cp.journal_offset(), head, "graceful checkpoint at the head");
}

/// **T8 (disabled)** — `checkpoint_every: None` writes no sidecar at all.
#[test]
fn cadence_none_writes_no_sidecar() {
    let dir = tempfile::tempdir().unwrap();
    let c = cfg(dir.path(), None);
    let outcome = kx_runtime::run(&c).unwrap();
    assert_eq!((outcome.committed, outcome.total), (8, 8));
    assert!(
        !checkpoint_io::sidecar_path(&c.journal_path).exists(),
        "no sidecar must be written when checkpointing is disabled"
    );
}

/// The graceful-completion sidecar is at the journal head, so a fresh recovery
/// reports `Seeded { offset: head, tail_entries: 0 }` (the structured
/// observability signal) and folds an empty tail.
///
/// NB: the runtime captures checkpoints AFTER `scheduler.submit`, so the payload
/// carries the workflow's *declared* Motes — a richer state than a bare
/// `from_journal` (which folds only journal entries). We therefore do NOT compare
/// the seeded projection to a bare full fold here (they legitimately differ by the
/// declarations); the *runtime-level* bit-identical guarantee — where recovery
/// re-applies the same declarations via `submit`, and `declare∘fold == fold∘declare`
/// via the children re-index — is proven by `replay_bit_identical_with_and_without_sidecar`
/// below and the subprocess `kill_and_replay` checkpoint scenarios.
#[test]
fn head_checkpoint_reports_seeded_empty_tail() {
    let dir = tempfile::tempdir().unwrap();
    let c = cfg(dir.path(), Some(2));
    kx_runtime::run(&c).unwrap();

    let journal = SqliteJournal::open(&c.journal_path).unwrap();
    let head = journal.current_seq().unwrap();
    let cp = checkpoint_io::read_checkpoint(&checkpoint_io::sidecar_path(&c.journal_path)).unwrap();
    assert_eq!(cp.journal_offset(), head);

    let (seeded, outcome) =
        Projection::from_journal_with_checkpoint_reported(&journal, Some(&cp)).unwrap();
    assert_eq!(
        outcome,
        CheckpointOutcome::Seeded {
            offset: head,
            tail_entries: 0
        },
        "a head checkpoint seeds with an empty tail"
    );
    assert_eq!(seeded.current_seq(), head, "seeded frontier is the head");
}

/// **T1 / T3 (in-process)** — the runtime's product digest of a replay is identical
/// whether recovery seeds from the sidecar or full-folds (sidecar deleted). This is
/// the bit-identical guarantee at the level that includes the declaration re-apply.
#[test]
fn replay_bit_identical_with_and_without_sidecar() {
    let dir = tempfile::tempdir().unwrap();
    let c = cfg(dir.path(), Some(2));
    let first = kx_runtime::run(&c).unwrap();
    assert_eq!((first.committed, first.total), (8, 8));

    let replay_cfg = RuntimeConfig {
        mode: Mode::Replay,
        ..c.clone()
    };

    // Sidecar present -> seeded recovery.
    let sidecar = checkpoint_io::sidecar_path(&c.journal_path);
    assert!(sidecar.exists());
    let seeded = kx_runtime::run(&replay_cfg).unwrap();
    assert_eq!(seeded.digest, first.digest, "seeded replay == original run");

    // Sidecar deleted -> full-fold recovery. Identical product digest.
    std::fs::remove_file(&sidecar).unwrap();
    let full = kx_runtime::run(&replay_cfg).unwrap();
    assert_eq!(
        full.digest, first.digest,
        "full-fold replay (no sidecar) is bit-identical to the seeded replay"
    );
}

/// A truncated sidecar (shorter than the envelope header) is discarded by the
/// reader (returns `None`); a fresh recovery then reports `NoCheckpoint` and is
/// bit-identical (matrix **T2** at the IO+projection seam).
#[test]
fn truncated_sidecar_is_discarded_and_full_folds() {
    let dir = tempfile::tempdir().unwrap();
    let c = cfg(dir.path(), Some(2));
    kx_runtime::run(&c).unwrap();

    let sidecar = checkpoint_io::sidecar_path(&c.journal_path);
    std::fs::write(&sidecar, b"\x01\x00\x00").unwrap(); // < HEADER_LEN
    assert!(
        checkpoint_io::read_checkpoint(&sidecar).is_none(),
        "a truncated sidecar must be unreadable"
    );

    let journal = SqliteJournal::open(&c.journal_path).unwrap();
    let cp: Option<&FoldCheckpoint> = None; // what the runtime passes after a None read
    let (recovered, outcome) =
        Projection::from_journal_with_checkpoint_reported(&journal, cp).unwrap();
    assert!(matches!(outcome, CheckpointOutcome::FullFold { .. }));
    let full = Projection::from_journal(&journal).unwrap();
    assert_eq!(recovered.state_digest(), full.state_digest());
}
