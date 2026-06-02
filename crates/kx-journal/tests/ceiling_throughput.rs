// Integration-test file: compiled as a separate crate from the host lib; tests
// legitimately use `.unwrap()` for fixture construction.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]
//! IMP-4 (D116) — single-writer journal throughput **measurement spike**.
//!
//! The single-writer journal is the runtime's exactly-once moat *and* its hard
//! scale wall. This file measures the raw `Journal`-trait write ceiling so a real
//! number can be published (HANDOFF §3.9 §A) instead of the standing "qualitatively
//! true, quantitatively unproven" placeholder. It is a **non-gating** characterization
//! (testing doctrine `04-testing-and-gates.md` §Load/throughput): every test is
//! `#[ignore]` (the green suite never runs it), prints commits/s, and asserts only a
//! loose catastrophic-regression floor + a correctness count — never an absolute-time
//! threshold (those flake across machines).
//!
//! Run via `just bench-ceiling` (or directly):
//! `cargo test -p kx-journal --release --test ceiling_throughput -- --ignored --nocapture --test-threads=1`
//! `--release` is essential — an unoptimized SQLite/encode path understates the ceiling.
//! Set `KX_CEILING_HUGE=1` to add the 10^6 tier (hundreds of MB RAM + on-disk WAL — local only).
//!
//! Four numbers, deliberately separated so the published curve attributes the cost:
//!
//! - (i) `SqliteJournal::append` sequential on-disk — one `BEGIN IMMEDIATE` + fsync per
//!   commit = the **pessimistic floor** (and the realistic per-commit floor: it already
//!   pays the per-entry dedup `SELECT` + `MAX(seq)+1` index probes).
//! - (ii) `SqliteJournal::append_batch` on-disk, swept over batch size — one fsync per
//!   batch = the **group-commit ceiling** (and how batch size moves it; 256 = the
//!   coordinator's `MAX_DRAIN`).
//! - (iv) `InMemoryJournal::append` — no fsync, `RwLock<Vec>` push = the **CPU-bound upper
//!   bound**; plus a `SqliteJournal::open_in_memory()` row that isolates the
//!   SQLite-transaction cost from the fsync cost.
//!
//! (Numbers i/ii/iv here; the realistic end-to-end coordinator number (iii) lives in
//! `kx-coordinator/tests/ceiling_e2e.rs` — it adds the channel + drain + group-commit.)
//!
//! Platform caveat for the published numbers: macOS `fsync` does not force a drive-cache
//! flush (no `F_FULLFSYNC`), so on-disk commits/s on Apple-Silicon is **optimistic**
//! relative to a Linux runner. Always label on-disk numbers with their environment; the
//! in-memory numbers are platform-comparable.

use std::time::Instant;

use kx_content::ContentRef;
use kx_journal::{InMemoryJournal, Journal, JournalEntry, SqliteJournal};
use kx_mote::{MoteDefHash, MoteId, NdClass};
use smallvec::SmallVec;
use tempfile::tempdir;

/// A distinct, dedup-storable `Committed` entry per index (the journal dedups by
/// `idempotency_key`, so distinct keys mean all N are stored). Mirrors the
/// `run_metadata_scale.rs` fixture shape.
fn committed_at(i: u64) -> JournalEntry {
    let mut id = [0u8; 32];
    id[..8].copy_from_slice(&i.to_le_bytes());
    JournalEntry::Committed {
        mote_id: MoteId::from_bytes(id),
        idempotency_key: id,
        seq: 0, // assigned by the journal at append time
        nondeterminism: NdClass::Pure,
        result_ref: ContentRef::from_bytes(id),
        parents: SmallVec::new(),
        warrant_ref: ContentRef::from_bytes([0u8; 32]),
        mote_def_hash: MoteDefHash::from_bytes([1u8; 32]),
    }
}

/// The size tiers. `KX_CEILING_HUGE=1` adds 10^6 (local only — RAM + disk heavy).
fn sizes() -> Vec<u64> {
    if std::env::var_os("KX_CEILING_HUGE").is_some() {
        vec![10_000, 100_000, 1_000_000]
    } else {
        vec![10_000, 100_000]
    }
}

/// commits/s from a count + elapsed wall time.
fn cps(n: u64, secs: f64) -> f64 {
    n as f64 / secs
}

// ---------------------------------------------------------------------------
// (i) raw SqliteJournal::append — sequential, on-disk, one fsync per commit.
// ---------------------------------------------------------------------------
#[test]
#[ignore = "scale: just bench-ceiling (cargo test -p kx-journal --release --test ceiling_throughput -- --ignored --nocapture)"]
fn append_sequential_ceiling() {
    eprintln!("=== (i) SqliteJournal::append — sequential on-disk (one fsync/commit) ===");
    for &n in &sizes() {
        let dir = tempdir().unwrap();
        let j = SqliteJournal::open(dir.path().join("ceiling.kxjournal")).unwrap();
        let entries: Vec<JournalEntry> = (0..n).map(committed_at).collect();

        let start = Instant::now();
        for e in entries {
            j.append(e).unwrap();
        }
        let elapsed = start.elapsed();

        assert_eq!(j.count_entries().unwrap(), n, "all {n} entries durable");
        let rate = cps(n, elapsed.as_secs_f64());
        eprintln!("  n={n:>9}  {elapsed:>12?}  {rate:>12.0} commits/s");
        assert!(
            rate > 100.0,
            "sequential append fell below 100 commits/s ({rate:.0}) — catastrophic regression"
        );
    }
}

// ---------------------------------------------------------------------------
// (ii) raw SqliteJournal::append_batch — on-disk, one fsync per batch.
//      Swept over batch size to show how group-commit moves the ceiling.
// ---------------------------------------------------------------------------
#[test]
#[ignore = "scale: just bench-ceiling (cargo test -p kx-journal --release --test ceiling_throughput -- --ignored --nocapture)"]
fn append_batch_ceiling() {
    // 256 == the coordinator's MAX_DRAIN (one drain → one append_batch → one fsync).
    const BATCH_SIZES: &[usize] = &[1, 64, 256];
    eprintln!("=== (ii) SqliteJournal::append_batch — on-disk (one fsync/batch) ===");
    for &n in &sizes() {
        for &batch in BATCH_SIZES {
            let dir = tempdir().unwrap();
            let j = SqliteJournal::open(dir.path().join("ceiling.kxjournal")).unwrap();
            let entries: Vec<JournalEntry> = (0..n).map(committed_at).collect();

            let start = Instant::now();
            for chunk in entries.chunks(batch) {
                j.append_batch(chunk.to_vec()).unwrap();
            }
            let elapsed = start.elapsed();

            assert_eq!(j.count_entries().unwrap(), n, "all {n} entries durable");
            let rate = cps(n, elapsed.as_secs_f64());
            eprintln!("  n={n:>9}  batch={batch:>4}  {elapsed:>12?}  {rate:>12.0} commits/s");
            assert!(
                rate > 100.0,
                "batch append fell below 100 commits/s ({rate:.0}) — catastrophic regression"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// (iv) in-memory ceilings — the CPU-bound upper bound (no fsync). Two rows:
//      InMemoryJournal (RwLock<Vec> push) and SqliteJournal in-memory (SQLite
//      txn machinery, no fsync) — the gap isolates SQLite cost from fsync cost.
// ---------------------------------------------------------------------------
#[test]
#[ignore = "scale: just bench-ceiling (cargo test -p kx-journal --release --test ceiling_throughput -- --ignored --nocapture)"]
fn in_memory_append_ceiling() {
    eprintln!("=== (iv) in-memory append — CPU-bound upper bound (no fsync) ===");
    for &n in &sizes() {
        // InMemoryJournal: pure RwLock<Vec> push + dedup probe.
        {
            let j = InMemoryJournal::new();
            let entries: Vec<JournalEntry> = (0..n).map(committed_at).collect();
            let start = Instant::now();
            for e in entries {
                j.append(e).unwrap();
            }
            let elapsed = start.elapsed();
            assert_eq!(j.count_entries().unwrap(), n);
            let rate = cps(n, elapsed.as_secs_f64());
            eprintln!("  InMemoryJournal   n={n:>9}  {elapsed:>12?}  {rate:>12.0} commits/s");
            assert!(
                rate > 100.0,
                "in-memory append below 100 commits/s ({rate:.0})"
            );
        }
        // SqliteJournal in-memory: SQLite txn machinery, no fsync.
        {
            let j = SqliteJournal::open_in_memory().unwrap();
            let entries: Vec<JournalEntry> = (0..n).map(committed_at).collect();
            let start = Instant::now();
            for e in entries {
                j.append(e).unwrap();
            }
            let elapsed = start.elapsed();
            assert_eq!(j.count_entries().unwrap(), n);
            let rate = cps(n, elapsed.as_secs_f64());
            eprintln!("  Sqlite in-memory  n={n:>9}  {elapsed:>12?}  {rate:>12.0} commits/s");
            assert!(
                rate > 100.0,
                "sqlite in-memory append below 100 commits/s ({rate:.0})"
            );
        }
    }
}
