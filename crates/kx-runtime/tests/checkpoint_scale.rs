//! M2.2b — end-to-end recovery SCALE test (`#[ignore]`; wired into `scale-smoke`).
//!
//! Proves the live-runtime resume property over a **real disk-backed SQLite
//! journal** (not the in-memory projection double): under a crash-loopy,
//! high-churn workload, seeding recovery from the on-disk checkpoint sidecar
//! decouples resume cost from journal *length* — it is bounded by live state.
//! A full fold pays SQLite read + fold for every entry; a seeded recovery reads
//! one sidecar blob and folds an (empty) tail.
//!
//! Run:
//!   cargo test -p kx-runtime --release --test checkpoint_scale \
//!     -- --ignored --nocapture --test-threads=1

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::time::Instant;

use kx_content::ContentRef;
use kx_journal::{FailureReason, Journal, JournalEntry, ParentEntry, SqliteJournal};
use kx_mote::{EdgeMeta, MoteDefHash, MoteId, NdClass, ParentRef};
use kx_projection::Projection;
use kx_runtime::checkpoint_io;
use smallvec::SmallVec;

fn mid_n(i: u32) -> MoteId {
    let mut b = [0u8; 32];
    b[..4].copy_from_slice(&i.to_le_bytes());
    MoteId::from_bytes(b)
}

fn ukey(n: u64) -> [u8; 32] {
    let mut k = [0xEEu8; 32];
    k[..8].copy_from_slice(&n.to_le_bytes());
    k
}

fn war() -> ContentRef {
    ContentRef::from_bytes([0xaa; 32])
}

#[test]
#[ignore = "scale: run --release --test checkpoint_scale -- --ignored --nocapture"]
fn runtime_recovery_is_bounded_by_live_state_not_journal_length() {
    const M: u32 = 5_000; // distinct Motes (live state)
    const CHURN: u32 = 10; // Proposed+Failed(pre-commit) cycles before each commit
    const BATCH: usize = 4_000; // group-commit chunk so setup stays fast

    let dir = tempfile::tempdir().unwrap();
    let journal_path = dir.path().join("scale.sqlite");

    // Build the high-churn journal via group-committed batches. Total entries =
    // M*(2*CHURN+1); distinct Motes = M. Pre-commit (`TimedOut`) failures keep
    // every Mote terminally clean so all commit.
    {
        let journal = SqliteJournal::open(&journal_path).unwrap();
        let mut buf: Vec<JournalEntry> = Vec::with_capacity(BATCH);
        let mut ctr = 0u64;
        let flush = |buf: &mut Vec<JournalEntry>| {
            if !buf.is_empty() {
                journal.append_batch(std::mem::take(buf)).unwrap();
            }
        };
        for i in 0..M {
            for _ in 0..CHURN {
                ctr += 2;
                buf.push(JournalEntry::Proposed {
                    mote_id: mid_n(i),
                    idempotency_key: ukey(ctr),
                    seq: 0,
                    nondeterminism: NdClass::Pure,
                    placement_hint: 0,
                    warrant_ref: war(),
                });
                buf.push(JournalEntry::Failed {
                    mote_id: mid_n(i),
                    idempotency_key: ukey(ctr + 1),
                    seq: 0,
                    reason_class: FailureReason::TimedOut,
                    reporter_id: 0,
                });
                if buf.len() >= BATCH {
                    flush(&mut buf);
                }
            }
            let parents = if i >= 1 {
                vec![ParentRef {
                    parent_id: mid_n(i - 1),
                    edge: EdgeMeta::data(),
                }]
            } else {
                vec![]
            };
            let pe: SmallVec<[ParentEntry; 4]> =
                parents.iter().map(ParentEntry::from_parent_ref).collect();
            buf.push(JournalEntry::Committed {
                mote_id: mid_n(i),
                idempotency_key: *mid_n(i).as_bytes(),
                seq: 0,
                nondeterminism: NdClass::Pure,
                result_ref: ContentRef::from_bytes([7u8; 32]),
                parents: pe,
                warrant_ref: war(),
                mote_def_hash: MoteDefHash::from_bytes([1u8; 32]),
            });
            if buf.len() >= BATCH {
                flush(&mut buf);
            }
        }
        flush(&mut buf);
    }

    let total = SqliteJournal::open(&journal_path)
        .unwrap()
        .current_seq()
        .unwrap();

    // Capture the head checkpoint and persist it through the real atomic-write
    // path, exactly as the live runtime does.
    let full_for_cp =
        Projection::from_journal(&SqliteJournal::open(&journal_path).unwrap()).unwrap();
    let reference_digest = full_for_cp.state_digest();
    let sidecar = checkpoint_io::sidecar_path(&journal_path);
    checkpoint_io::write_atomic(&sidecar, &full_for_cp.fold_checkpoint().to_bytes()).unwrap();
    drop(full_for_cp);

    // Baseline: a full cold re-fold reading every row from a freshly-opened journal.
    let t0 = Instant::now();
    let full = Projection::from_journal(&SqliteJournal::open(&journal_path).unwrap()).unwrap();
    let full_us = t0.elapsed().as_secs_f64() * 1e6;
    assert_eq!(full.state_digest(), reference_digest);

    // Seeded recovery: read the sidecar from disk + decode live state + fold the
    // (empty) tail from a freshly-opened journal.
    let t1 = Instant::now();
    let cp = checkpoint_io::read_checkpoint(&sidecar).unwrap();
    let seeded = Projection::from_journal_with_checkpoint(
        &SqliteJournal::open(&journal_path).unwrap(),
        Some(&cp),
    )
    .unwrap();
    let seeded_us = t1.elapsed().as_secs_f64() * 1e6;

    assert_eq!(
        seeded.state_digest(),
        reference_digest,
        "seeded recovery must reproduce the full fold exactly"
    );

    let speedup = full_us / seeded_us;
    eprintln!(
        "sqlite total_entries={total} live_motes={M} churn={CHURN}x  \
         full_refold={:.2}ms  seeded_resume={:.2}ms  speedup={speedup:.1}x",
        full_us / 1000.0,
        seeded_us / 1000.0
    );
    // The disk-backed full fold pays SQLite read + fold for all M*(2C+1) rows;
    // the seeded resume reads one blob + folds nothing. Conservative gate to
    // catch a regression that re-couples resume to journal length.
    assert!(
        speedup > 2.0,
        "seeded recovery should be bounded by live state, not journal length; got \
         {speedup:.1}x (full={full_us:.0}us, seeded={seeded_us:.0}us, total_entries={total})"
    );
}
