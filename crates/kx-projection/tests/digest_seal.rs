// Integration-test file: compiled as a separate crate from the host lib; tests
// legitimately `.unwrap()` for fixture construction.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]
//! M2.2c — journaled digest seals (D103.2): the unforgeability anchor for
//! checkpoint-seeded recovery.
//!
//! A checkpoint-seeded base at offset `S` is trusted ONLY if a journaled
//! `DigestSealed{through_seq == S}` (committed by the runtime at `S + 1`) matches
//! the seed's digest. This file proves the four outcomes at the projection level:
//!
//! - **match** → `Seeded` and bit-identical to a full fold;
//! - **mismatch** (a forged-but-self-consistent sidecar — the D103.1 attack) →
//!   `FullFold{SealMismatch}`, recovery falls back to the trust root;
//! - **missing** (the M2.2b world — a sidecar with no co-committed seal) →
//!   `FullFold{SealMissing}`;
//! - **stale** (a seal whose `through_seq` does not match the offset) →
//!   `FullFold{SealMissing}`.
//!
//! In every case recovery is **bit-identical** to a full fold (fail-closed).

use kx_content::ContentRef;
use kx_journal::{InMemoryJournal, Journal, JournalEntry};
use kx_mote::{MoteDefHash, MoteId, NdClass};
use kx_projection::{CheckpointOutcome, FoldCheckpoint, FullFoldReason, Projection};
use smallvec::SmallVec;

fn mid(b: u8) -> MoteId {
    MoteId::from_bytes([b; 32])
}

fn committed(m: u8) -> JournalEntry {
    JournalEntry::Committed {
        mote_id: mid(m),
        idempotency_key: *mid(m).as_bytes(),
        seq: 0,
        nondeterminism: NdClass::Pure,
        result_ref: ContentRef::from_bytes([7u8; 32]),
        parents: SmallVec::new(),
        warrant_ref: ContentRef::from_bytes([0xaa; 32]),
        mote_def_hash: MoteDefHash::from_bytes([1u8; 32]),
    }
}

/// Fold the journal prefix `[1, k]` into a fresh projection (the contiguous-prefix
/// precondition `fold_checkpoint` requires).
fn seed_through(journal: &InMemoryJournal, k: u64) -> Projection {
    let mut p = Projection::new();
    for e in journal.read_entries_by_seq(0..(k + 1)).unwrap() {
        p.fold(&e).unwrap();
    }
    p
}

/// A journal that commits motes 1..=2, co-commits the M2.2c seal for frontier 2
/// (optionally with a tampered digest / through_seq), then commits motes 3..=5.
/// Returns the journal and the checkpoint captured at frontier 2 (pre-seal).
fn journal_with_seal(
    seal_digest: Option<[u8; 32]>,
    seal_through: u64,
) -> (InMemoryJournal, FoldCheckpoint) {
    let journal = InMemoryJournal::new();
    journal.append(committed(1)).unwrap();
    journal.append(committed(2)).unwrap();
    let cp = seed_through(&journal, 2).fold_checkpoint();
    assert_eq!(cp.journal_offset(), 2);
    let digest = seal_digest.unwrap_or_else(|| seed_through(&journal, 2).state_digest());
    journal
        .append(JournalEntry::DigestSealed {
            through_seq: seal_through,
            state_digest: digest,
            seq: 0,
        })
        .unwrap();
    journal.append(committed(3)).unwrap();
    journal.append(committed(4)).unwrap();
    journal.append(committed(5)).unwrap();
    (journal, cp)
}

/// Assert the recovered projection is bit-identical (full-state digest + frontier)
/// to a from-scratch full fold of the same journal — the fail-safe invariant.
fn assert_bit_identical(journal: &InMemoryJournal, resumed: &Projection, ctx: &str) {
    let full = Projection::from_journal(journal).unwrap();
    assert_eq!(
        resumed.state_digest(),
        full.state_digest(),
        "{ctx}: state digest"
    );
    assert_eq!(resumed.current_seq(), full.current_seq(), "{ctx}: frontier");
}

/// A matching seal anchors the seed: `Seeded` + bit-identical.
#[test]
fn matching_seal_seeds_and_is_bit_identical() {
    let (journal, cp) = journal_with_seal(None, 2);
    let (resumed, outcome) =
        Projection::from_journal_with_checkpoint_reported(&journal, Some(&cp)).unwrap();
    // Tail (2, 6] = the seal + commits 3,4,5 = four entries.
    assert_eq!(
        outcome,
        CheckpointOutcome::Seeded {
            offset: 2,
            tail_entries: 4
        }
    );
    assert_bit_identical(&journal, &resumed, "matching seal");
}

/// **The D103.1 attack, now caught.** A forged-but-self-consistent checkpoint
/// (a valid envelope decoding to a *wrong* base state) whose digest does not match
/// the journaled seal is rejected → `SealMismatch` → full fold (correct state).
#[test]
fn forged_seed_with_mismatched_seal_full_folds() {
    // The journal's seal anchors the genuine state-at-2. Build a checkpoint that
    // encodes a DIFFERENT (wrong) base state at offset 2 (mote 99 committed instead
    // of motes 1,2) but is internally self-consistent (valid envelope digest).
    let (journal, _genuine_cp) = journal_with_seal(None, 2);

    let wrong = InMemoryJournal::new();
    wrong.append(committed(98)).unwrap();
    wrong.append(committed(99)).unwrap();
    let forged_cp =
        FoldCheckpoint::from_bytes(&seed_through(&wrong, 2).fold_checkpoint().to_bytes()).unwrap();
    assert_eq!(forged_cp.journal_offset(), 2); // same frontier — passes gates 1-5
    assert!(
        forged_cp.verify(),
        "the forged sidecar is internally self-consistent"
    );

    let (resumed, outcome) =
        Projection::from_journal_with_checkpoint_reported(&journal, Some(&forged_cp)).unwrap();
    assert_eq!(
        outcome,
        CheckpointOutcome::FullFold {
            reason: FullFoldReason::SealMismatch
        }
    );
    // The forged seed is discarded; recovery reproduces the genuine full fold.
    assert_bit_identical(&journal, &resumed, "forged seed rejected");
    // Sanity: the genuine state has motes 1..=5, NOT the forged 98/99.
    assert_eq!(resumed.iter_motes().count(), 5);
}

/// The M2.2b world: a valid checkpoint with NO co-committed seal → `SealMissing` →
/// full fold (bit-identical). An un-anchored seed is never trusted.
#[test]
fn seed_without_seal_full_folds() {
    let journal = InMemoryJournal::new();
    journal.append(committed(1)).unwrap();
    journal.append(committed(2)).unwrap();
    let cp = seed_through(&journal, 2).fold_checkpoint();
    journal.append(committed(3)).unwrap(); // seq 3 is a commit, NOT a seal

    let (resumed, outcome) =
        Projection::from_journal_with_checkpoint_reported(&journal, Some(&cp)).unwrap();
    assert_eq!(
        outcome,
        CheckpointOutcome::FullFold {
            reason: FullFoldReason::SealMissing
        }
    );
    assert_bit_identical(&journal, &resumed, "seed without seal");
}

/// A seal present at `S + 1` but carrying the WRONG `through_seq` does not anchor
/// offset `S` → `SealMissing` (the lookup requires `through_seq == S`).
#[test]
fn stale_seal_through_seq_is_treated_as_missing() {
    // Seal claims through_seq = 1 but sits at seq 3 (frontier 2's slot).
    let (journal, cp) = journal_with_seal(None, 1);
    let (resumed, outcome) =
        Projection::from_journal_with_checkpoint_reported(&journal, Some(&cp)).unwrap();
    assert_eq!(
        outcome,
        CheckpointOutcome::FullFold {
            reason: FullFoldReason::SealMissing
        }
    );
    assert_bit_identical(&journal, &resumed, "stale seal");
}

/// `DigestSealed` is a pure `last_seq`-only fold no-op: folding it changes neither
/// the Mote set nor the children index nor any per-Mote flag — only the frontier.
#[test]
fn seal_folds_as_pure_frontier_advance() {
    let journal = InMemoryJournal::new();
    journal.append(committed(1)).unwrap();
    let before = seed_through(&journal, 1);
    let motes_before: Vec<_> = before.iter_motes().collect();

    journal
        .append(JournalEntry::DigestSealed {
            through_seq: 1,
            state_digest: before.state_digest(),
            seq: 0,
        })
        .unwrap();
    let after = seed_through(&journal, 2);

    // The Mote set is unchanged; only the frontier advanced (1 -> 2).
    assert_eq!(after.iter_motes().collect::<Vec<_>>(), motes_before);
    assert_eq!(before.current_seq(), 1);
    assert_eq!(after.current_seq(), 2);
}
