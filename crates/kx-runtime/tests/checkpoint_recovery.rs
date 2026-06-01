//! M2.2b — live checkpoint recovery wiring (in-process).
//!
//! Complements the subprocess `kill_and_replay` checkpoint scenarios: these run
//! the engine in-process so they can inspect the persisted sidecar and drive the
//! `kx_projection` recovery API directly, asserting the structured
//! [`CheckpointOutcome`] and the bit-identical (state-digest) guarantee.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::path::Path;

use kx_journal::{Journal, JournalEntry, SqliteJournal};
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

    // The completed run leaves a usable sidecar folded through the final frontier,
    // plus (M2.2c) a journaled `DigestSealed` anchoring it as the very last entry.
    // So the checkpoint frontier is `head - 1` and the seal occupies `head`.
    let sidecar = checkpoint_io::sidecar_path(&c.journal_path);
    let cp = checkpoint_io::read_checkpoint(&sidecar).expect("a sidecar after a completed run");
    let journal = SqliteJournal::open(&c.journal_path).unwrap();
    let head = journal.current_seq().unwrap();
    assert_eq!(
        cp.journal_offset(),
        head - 1,
        "graceful checkpoint at the frontier just before its seal"
    );
    let last = journal
        .read_entries_by_seq(head..(head + 1))
        .unwrap()
        .next()
        .expect("a head entry");
    assert!(
        matches!(last, JournalEntry::DigestSealed { through_seq, .. } if through_seq == cp.journal_offset()),
        "the last entry is the digest seal anchoring the graceful checkpoint frontier"
    );
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

/// The graceful-completion sidecar is at frontier `head - 1`, anchored by a
/// `DigestSealed` at `head` (M2.2c). A fresh recovery reports
/// `Seeded { offset: head - 1, tail_entries: 1 }` (the structured observability
/// signal) and folds the 1-entry tail (the seal, a `last_seq` no-op), ending at
/// `head`. This proves the seal anchors the seed end-to-end through the runtime.
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
fn head_checkpoint_seeds_anchored_by_seal() {
    let dir = tempfile::tempdir().unwrap();
    let c = cfg(dir.path(), Some(2));
    kx_runtime::run(&c).unwrap();

    let journal = SqliteJournal::open(&c.journal_path).unwrap();
    let head = journal.current_seq().unwrap();
    let cp = checkpoint_io::read_checkpoint(&checkpoint_io::sidecar_path(&c.journal_path)).unwrap();
    let offset = cp.journal_offset();
    assert_eq!(
        offset,
        head - 1,
        "checkpoint frontier sits just before its seal"
    );

    let (seeded, outcome) =
        Projection::from_journal_with_checkpoint_reported(&journal, Some(&cp)).unwrap();
    assert_eq!(
        outcome,
        CheckpointOutcome::Seeded {
            offset,
            tail_entries: 1
        },
        "a head checkpoint seeds, anchored by its 1-entry seal tail"
    );
    assert_eq!(
        seeded.current_seq(),
        head,
        "seeded frontier reaches the head"
    );
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

/// **M2.2c product-digest invariant (guards `a6b5c679…`).** A seal-writing run
/// (cadence on) and a seal-free run (cadence off) produce the SAME product digest:
/// `DigestSealed` is off-DAG metadata, invisible to `digest_projection` (which
/// folds only `Committed` Motes). Also proves the cadence does not runaway-seal —
/// each seal anchors a distinct, strictly-increasing frontier.
#[test]
fn seals_do_not_change_product_digest_and_do_not_runaway() {
    let d_sealed = tempfile::tempdir().unwrap();
    let c_sealed = cfg(d_sealed.path(), Some(2));
    let sealed = kx_runtime::run(&c_sealed).unwrap();

    let d_plain = tempfile::tempdir().unwrap();
    let plain = kx_runtime::run(&cfg(d_plain.path(), None)).unwrap();

    assert_eq!((sealed.committed, sealed.total), (8, 8));
    assert_eq!((plain.committed, plain.total), (8, 8));
    assert_eq!(
        sealed.digest, plain.digest,
        "journaled seals must NOT change the product digest"
    );

    // The cadence run wrote seals; their frontiers are distinct + strictly
    // increasing (a runaway would re-seal the same frontier), and each seal sits
    // AFTER the frontier it anchors (`seq == through_seq + 1` under single-writer).
    let journal = SqliteJournal::open(&c_sealed.journal_path).unwrap();
    let head = journal.current_seq().unwrap();
    let seal_frontiers: Vec<u64> = journal
        .read_entries_by_seq(1..(head + 1))
        .unwrap()
        .filter_map(|e| match e {
            JournalEntry::DigestSealed {
                through_seq, seq, ..
            } => {
                assert!(seq > through_seq, "a seal sits after its frontier");
                Some(through_seq)
            }
            _ => None,
        })
        .collect();
    assert!(!seal_frontiers.is_empty(), "the cadence run wrote ≥1 seal");
    assert!(
        seal_frontiers.windows(2).all(|w| w[0] < w[1]),
        "seal frontiers must be strictly increasing (no runaway re-sealing): {seal_frontiers:?}"
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
