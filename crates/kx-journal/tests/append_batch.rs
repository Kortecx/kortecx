// Integration test (separate crate) — inherits the workspace deny on
// unwrap/expect; tests legitimately use them for fixtures.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]
//! `Journal::append_batch` — the group-commit primitive. Verified on BOTH shipped
//! backends (InMemoryJournal + SqliteJournal):
//!
//! - contiguous monotonic seqs in input order;
//! - parity with looping `append`;
//! - dedup-by-key within a single batch and across batches;
//! - `Proposed`/`Failed` are NOT deduped (each is its own attempt);
//! - empty batch is a no-op;
//! - readback through `read_entries_by_seq` / `read_committed`;
//! - **all-or-nothing atomicity**: a mid-batch failure (Sqlite) rolls the whole
//!   batch back, leaving the journal untouched.

use kx_content::ContentRef;
use kx_journal::{InMemoryJournal, Journal, JournalEntry, ParentEntry, SqliteJournal, MAX_PARENTS};
use kx_mote::{MoteDefHash, MoteId, NdClass};
use smallvec::SmallVec;

fn committed(id: u8, key: u8) -> JournalEntry {
    JournalEntry::Committed {
        mote_id: MoteId::from_bytes([id; 32]),
        idempotency_key: [key; 32],
        seq: 0,
        nondeterminism: NdClass::Pure,
        result_ref: ContentRef::from_bytes([id ^ 0x5a; 32]),
        parents: SmallVec::new(),
        warrant_ref: ContentRef::from_bytes([0xaa; 32]),
        mote_def_hash: MoteDefHash::from_bytes([id; 32]),
    }
}

fn proposed(id: u8, key: u8) -> JournalEntry {
    JournalEntry::Proposed {
        mote_id: MoteId::from_bytes([id; 32]),
        idempotency_key: [key; 32],
        seq: 0,
        nondeterminism: NdClass::Pure,
        placement_hint: 0,
        warrant_ref: ContentRef::from_bytes([0xaa; 32]),
    }
}

/// A `Committed` whose parent count exceeds `MAX_PARENTS` — `encode_entry` rejects
/// it (`TooManyParents`), giving a deterministic mid-batch failure (SqliteJournal).
fn poison() -> JournalEntry {
    let parents: SmallVec<[ParentEntry; 4]> = (0..=MAX_PARENTS as u32)
        .map(|i| ParentEntry {
            parent_id: MoteId::from_bytes([u8::try_from(i % 256).unwrap(); 32]),
            edge_kind: 0,
            non_cascade: 0,
        })
        .collect();
    JournalEntry::Committed {
        mote_id: MoteId::from_bytes([0xEE; 32]),
        idempotency_key: [0xEE; 32],
        seq: 0,
        nondeterminism: NdClass::Pure,
        result_ref: ContentRef::from_bytes([0xEE; 32]),
        parents,
        warrant_ref: ContentRef::from_bytes([0xaa; 32]),
        mote_def_hash: MoteDefHash::from_bytes([0xEE; 32]),
    }
}

fn seqs(entries: &[JournalEntry]) -> Vec<u64> {
    entries.iter().map(JournalEntry::seq).collect()
}

// --- contiguous monotonic seqs ---------------------------------------------

fn check_contiguous(j: &impl Journal) {
    let out = j
        .append_batch(vec![committed(1, 1), committed(2, 2), committed(3, 3)])
        .unwrap();
    assert_eq!(seqs(&out), vec![1, 2, 3], "seqs assigned in input order");
    assert_eq!(j.current_seq().unwrap(), 3);
    assert_eq!(j.count_entries().unwrap(), 3);
}

#[test]
fn contiguous_seqs_in_memory() {
    check_contiguous(&InMemoryJournal::new());
}

#[test]
fn contiguous_seqs_sqlite() {
    check_contiguous(&SqliteJournal::open_in_memory().unwrap());
}

// --- parity with looped append ---------------------------------------------

fn check_parity(batched: &impl Journal, looped: &impl Journal) {
    let entries = vec![proposed(1, 1), committed(2, 2), committed(3, 3)];
    let batch_out = batched.append_batch(entries.clone()).unwrap();
    let loop_out: Vec<JournalEntry> = entries
        .into_iter()
        .map(|e| looped.append(e).unwrap())
        .collect();
    assert_eq!(seqs(&batch_out), seqs(&loop_out));
    assert_eq!(
        batched.current_seq().unwrap(),
        looped.current_seq().unwrap()
    );
    assert_eq!(
        batched.count_entries().unwrap(),
        looped.count_entries().unwrap()
    );
    // The durable log is identical entry-for-entry.
    let b: Vec<JournalEntry> = batched.read_entries_by_seq(0..4).unwrap().collect();
    let l: Vec<JournalEntry> = looped.read_entries_by_seq(0..4).unwrap().collect();
    assert_eq!(b, l);
}

#[test]
fn parity_in_memory() {
    check_parity(&InMemoryJournal::new(), &InMemoryJournal::new());
}

#[test]
fn parity_sqlite() {
    check_parity(
        &SqliteJournal::open_in_memory().unwrap(),
        &SqliteJournal::open_in_memory().unwrap(),
    );
}

// --- dedup within a single batch -------------------------------------------

fn check_dedup_within(j: &impl Journal) {
    // Same idempotency_key twice in one batch: the second dedupes to the first.
    let out = j
        .append_batch(vec![committed(1, 7), committed(1, 7)])
        .unwrap();
    assert_eq!(out.len(), 2, "result length always equals input length");
    assert_eq!(out[0].seq(), 1);
    assert_eq!(out[1].seq(), 1, "duplicate returns the pre-existing seq");
    assert_eq!(j.current_seq().unwrap(), 1, "only one seq consumed");
    assert_eq!(j.count_entries().unwrap(), 1);
}

#[test]
fn dedup_within_batch_in_memory() {
    check_dedup_within(&InMemoryJournal::new());
}

#[test]
fn dedup_within_batch_sqlite() {
    check_dedup_within(&SqliteJournal::open_in_memory().unwrap());
}

// --- dedup across batches --------------------------------------------------

fn check_dedup_across(j: &impl Journal) {
    let first = j.append_batch(vec![committed(1, 1)]).unwrap();
    let second = j
        .append_batch(vec![committed(1, 1), committed(2, 2)])
        .unwrap();
    assert_eq!(second[0].seq(), first[0].seq(), "re-seen key dedupes");
    assert_eq!(second[1].seq(), 2, "new key gets the next seq");
    assert_eq!(j.current_seq().unwrap(), 2);
    assert_eq!(j.count_entries().unwrap(), 2);
}

#[test]
fn dedup_across_batches_in_memory() {
    check_dedup_across(&InMemoryJournal::new());
}

#[test]
fn dedup_across_batches_sqlite() {
    check_dedup_across(&SqliteJournal::open_in_memory().unwrap());
}

// --- Proposed is NOT deduped (each attempt is its own fact) -----------------

fn check_proposed_not_deduped(j: &impl Journal) {
    let out = j
        .append_batch(vec![proposed(1, 1), proposed(1, 1), committed(2, 2)])
        .unwrap();
    assert_eq!(
        seqs(&out),
        vec![1, 2, 3],
        "both Proposed land; nothing deduped"
    );
    assert_eq!(j.count_entries().unwrap(), 3);
}

#[test]
fn proposed_not_deduped_in_memory() {
    check_proposed_not_deduped(&InMemoryJournal::new());
}

#[test]
fn proposed_not_deduped_sqlite() {
    check_proposed_not_deduped(&SqliteJournal::open_in_memory().unwrap());
}

// --- empty batch is a no-op ------------------------------------------------

fn check_empty(j: &impl Journal) {
    let out = j.append_batch(Vec::new()).unwrap();
    assert!(out.is_empty());
    assert_eq!(j.current_seq().unwrap(), 0);
    assert_eq!(j.count_entries().unwrap(), 0);
}

#[test]
fn empty_batch_in_memory() {
    check_empty(&InMemoryJournal::new());
}

#[test]
fn empty_batch_sqlite() {
    check_empty(&SqliteJournal::open_in_memory().unwrap());
}

// --- readback through the query API ----------------------------------------

fn check_readback(j: &impl Journal) {
    j.append_batch(vec![committed(1, 1), proposed(2, 2), committed(3, 3)])
        .unwrap();
    let all: Vec<JournalEntry> = j.read_entries_by_seq(0..4).unwrap().collect();
    assert_eq!(seqs(&all), vec![1, 2, 3]);
    let c = j.read_committed(&MoteId::from_bytes([3; 32])).unwrap();
    assert!(matches!(c, Some(JournalEntry::Committed { .. })));
}

#[test]
fn readback_in_memory() {
    check_readback(&InMemoryJournal::new());
}

#[test]
fn readback_sqlite() {
    check_readback(&SqliteJournal::open_in_memory().unwrap());
}

// --- all-or-nothing atomicity (Sqlite mid-batch failure → full rollback) ----

#[test]
fn sqlite_batch_is_atomic_on_failure() {
    let j = SqliteJournal::open_in_memory().unwrap();
    // Prime with one good entry so we can prove the failed batch changes nothing.
    j.append(committed(5, 5)).unwrap();
    assert_eq!(j.current_seq().unwrap(), 1);

    // A batch whose middle entry fails to encode (too many parents).
    let result = j.append_batch(vec![committed(6, 6), poison(), committed(7, 7)]);
    assert!(result.is_err(), "the batch must fail as a unit");

    // Nothing from the failed batch landed: seq + count unchanged, neither good
    // sibling persisted (all-or-nothing rollback).
    assert_eq!(j.current_seq().unwrap(), 1, "seq did not advance");
    assert_eq!(j.count_entries().unwrap(), 1, "no batch entry persisted");
    assert!(j
        .read_committed(&MoteId::from_bytes([6; 32]))
        .unwrap()
        .is_none());
    assert!(j
        .read_committed(&MoteId::from_bytes([7; 32]))
        .unwrap()
        .is_none());
}
