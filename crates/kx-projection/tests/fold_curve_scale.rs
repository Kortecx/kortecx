// Integration-test file: compiled as a separate crate from the host lib; tests
// legitimately use `.unwrap()` for fixture construction.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]
//! IMP-4 (D116) — the **projection-fold curve** (number v): the read/recovery side of
//! the single-writer ceiling. Cold recovery folds the committed log into projection
//! state, so the per-entry fold cost is the resume-availability ceiling; a super-linear
//! fold turns a large-log resume into an outage. This publishes the µs/entry curve at
//! 10^3 → 10^5 (10^6 local) the dossier asks for.
//!
//! Unlike the throughput numbers (which are non-gating prints), this carries a **ratio
//! gate** — `per_entry[100k] / per_entry[1k] < 8.0` — identical in spirit to the existing
//! `incremental_children_index` / `run_metadata_scale` / `migrate_25k_is_linear` gates: a
//! *ratio* (never absolute time) so it cannot flake across CI-ubuntu vs local-Apple-Silicon,
//! while a quadratic fold (≈100× across this 100× span) is caught with wide headroom. This is
//! the ONE line this spike adds to the required `scale-smoke` gate.
//!
//! Run via `scale-smoke` / `just bench-ceiling`:
//! `cargo test -p kx-projection --release --test fold_curve_scale -- --ignored --nocapture --test-threads=1`
//! `--release` is required (the debug differential oracle makes the fold O(n²); ratio skipped in debug).
//! `KX_CEILING_HUGE=1` adds the 10^6 tier (hundreds of MB RAM — local only, kept OUT of the gate).

use std::time::Instant;

use kx_content::ContentRef;
use kx_journal::{JournalEntry, ParentEntry};
use kx_mote::{EdgeMeta, MoteDefHash, MoteId, NdClass, ParentRef};
use kx_projection::Projection;
use smallvec::SmallVec;

/// A distinct `MoteId` per `u32` (first 4 bytes carry `i`).
fn mid_n(i: u32) -> MoteId {
    let mut bytes = [0u8; 32];
    bytes[..4].copy_from_slice(&i.to_le_bytes());
    MoteId::from_bytes(bytes)
}

fn pref(id: MoteId, edge: EdgeMeta) -> ParentRef {
    ParentRef {
        parent_id: id,
        edge,
    }
}

fn committed_with(id: MoteId, seq: u64, parents: &[ParentRef]) -> JournalEntry {
    let pe: SmallVec<[ParentEntry; 4]> = parents.iter().map(ParentEntry::from_parent_ref).collect();
    JournalEntry::Committed {
        mote_id: id,
        idempotency_key: *id.as_bytes(),
        seq,
        nondeterminism: NdClass::Pure,
        result_ref: ContentRef::from_bytes([7u8; 32]),
        parents: pe,
        warrant_ref: ContentRef::from_bytes([0xaa; 32]),
        mote_def_hash: MoteDefHash::from_bytes([1u8; 32]),
    }
}

/// A wide-but-HUBLESS DAG (mote i ← i-1 data, i-2 control; fan-in/out ≤ 2), so a
/// correct incremental fold is O(n) — the common-case resume shape. (Built before
/// the timed region.)
fn hubless_chain(n: u32) -> Vec<JournalEntry> {
    let mut entries: Vec<JournalEntry> = Vec::with_capacity(n as usize);
    for i in 0..n {
        let mut ps: Vec<ParentRef> = Vec::new();
        if i >= 1 {
            ps.push(pref(mid_n(i - 1), EdgeMeta::data()));
        }
        if i >= 2 {
            ps.push(pref(mid_n(i - 2), EdgeMeta::control()));
        }
        entries.push(committed_with(mid_n(i), u64::from(i) + 1, &ps));
    }
    entries
}

#[test]
#[ignore = "scale: scale-smoke / just bench-ceiling (cargo test -p kx-projection --release --test fold_curve_scale -- --ignored --nocapture)"]
fn fold_curve_is_linear() {
    // The gated span is always [1k, 10k, 100k]; the 100k/1k ratio (indices 2/0) is the
    // gate and is stable regardless of the optional 10^6 tier appended below.
    let mut sizes: Vec<u32> = vec![1_000, 10_000, 100_000];
    if std::env::var_os("KX_CEILING_HUGE").is_some() {
        sizes.push(1_000_000);
    }

    let mut per_entry_us: Vec<f64> = Vec::with_capacity(sizes.len());
    for &n in &sizes {
        let entries = hubless_chain(n);
        let start = Instant::now();
        let mut p = Projection::new();
        for e in &entries {
            p.fold(e).unwrap();
        }
        let elapsed = start.elapsed();
        assert_eq!(p.committed_count(), n as usize, "all {n} motes folded");

        let us = elapsed.as_secs_f64() * 1e6;
        let per = us / f64::from(n);
        per_entry_us.push(per);
        eprintln!(
            "  n={n:>9}  fold={:>10.2}ms  per_entry={per:>7.3}us",
            us / 1e3
        );
    }

    // Gate on the fixed 100k/1k pair (indices 2/0), independent of the 10^6 tier.
    let ratio = per_entry_us[2] / per_entry_us[0];
    eprintln!("per-entry fold cost ratio (100k/1k) = {ratio:.2}  (quadratic would be ~100x)");

    if cfg!(debug_assertions) {
        eprintln!(
            "NOTE: debug build — the differential oracle makes the fold O(n^2); ratio \
             assertion skipped. Re-run with --release for the real gate."
        );
    } else {
        assert!(
            ratio < 8.0,
            "fold per-entry cost grew {ratio:.1}x (1k->100k) — super-linear; the IMP-4 \
             resume-availability ceiling (cold recovery folds the whole log) is violated"
        );
    }
}
