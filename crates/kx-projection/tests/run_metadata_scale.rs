//! M6.2 scale gate — the run-metadata fold (D78) must stay **O(entries)** (flat
//! per entry), so feeding the planner observability over a large journal never
//! becomes a super-linear tax. Mirrors `kx-journal`'s `migrate_25k_is_linear`
//! ratio-gate style.
//!
//! Run via `scale-smoke`:
//! `cargo test -p kx-projection --release --test run_metadata_scale -- --ignored --nocapture`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::time::Instant;

use kx_journal::{InMemoryJournal, Journal, JournalEntry, INSTANCE_ID_LEN};
use kx_mote::{MoteDefHash, MoteId, NdClass};
use kx_projection::fold_run_metadata;
use smallvec::SmallVec;

fn build_journal(n: u32) -> InMemoryJournal {
    let j = InMemoryJournal::new();
    j.append(JournalEntry::RunRegistered {
        instance_id: [0x01; INSTANCE_ID_LEN],
        recipe_fingerprint: [0x02; 32],
        ts: 0,
        seq: 0,
    })
    .unwrap();
    for i in 0..n {
        // Distinct id + idempotency_key per entry (the journal dedups by key), so
        // all n Committed entries are stored. The def hash varies over a smaller
        // domain so the recipe BTreeSet grows — the realistic worst case for the
        // fold's per-entry cost.
        let mut id = [0u8; 32];
        id[0..4].copy_from_slice(&i.to_le_bytes());
        let mut def = [0u8; 32];
        def[0..4].copy_from_slice(&(i % 4096).to_le_bytes());
        j.append(JournalEntry::Committed {
            mote_id: MoteId::from_bytes(id),
            idempotency_key: id,
            seq: 0,
            nondeterminism: NdClass::Pure,
            result_ref: kx_content::ContentRef::from_bytes(id),
            parents: SmallVec::new(),
            warrant_ref: kx_content::ContentRef::from_bytes([0; 32]),
            mote_def_hash: MoteDefHash::from_bytes(def),
        })
        .unwrap();
    }
    j
}

#[test]
#[ignore = "scale: run --release --test run_metadata_scale -- --ignored --nocapture"]
fn metadata_fold_scale_is_linear() {
    const SIZES: &[u32] = &[1_000, 5_000, 10_000, 25_000];
    let mut per_entry_us: Vec<f64> = Vec::with_capacity(SIZES.len());

    for &n in SIZES {
        let j = build_journal(n);
        let start = Instant::now();
        let md = fold_run_metadata(&j).unwrap();
        let elapsed = start.elapsed();
        assert_eq!(md.committed as u32, n, "every committed entry folded");
        assert_eq!(md.runs, 1);
        let us = elapsed.as_secs_f64() * 1e6;
        let per = us / f64::from(n);
        per_entry_us.push(per);
        eprintln!(
            "n={n:>6}  fold={:>9.2}ms  per_entry={per:>7.3}us  distinct_recipes={}",
            us / 1e3,
            md.recipe_fingerprints.len()
        );
    }

    let ratio = per_entry_us.last().unwrap() / per_entry_us.first().unwrap();
    eprintln!(
        "per-entry metadata-fold cost ratio (25k/1k) = {ratio:.2}  (quadratic would be ~25x)"
    );
    if !cfg!(debug_assertions) {
        assert!(
            ratio < 8.0,
            "fold_run_metadata per-entry cost grew {ratio:.1}x (1k->25k) — super-linear; \
             the planner observability fold must stay O(entries)"
        );
    }
}
