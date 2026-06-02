//! Scale-smoke: the on-by-default capture store (D67) is flat per-record at scale.
//!
//! `#[ignore]`d — run in `--release` via the `scale-smoke` recipe. The runtime's
//! on-by-default capture (M3.1) records one [`StepRecord::action`] per committed
//! Mote into an [`InMemoryCaptureStore`]; this proves that record path is
//! O(N·log N) (a `BTreeMap` insert per action), so capture-on-by-default cannot
//! turn a large run super-linear. The other half of the engine's capture sweep —
//! `Projection::result_ref_of` — is a `BTreeMap` read held flat by
//! `kx-projection`'s `incremental_children_index` scale test; the kx-runtime
//! `stress_throughput::h3_capture_sweep_is_flat_per_mote` campaign case measures
//! the two together end-to-end.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Instant;

use kx_capture::{CaptureConsent, InMemoryCaptureStore, StepRecord};
use kx_content::ContentRef;
use kx_mote::MoteId;

const SIZES: &[usize] = &[1_000, 5_000, 10_000, 25_000];

/// A unique `MoteId` for index `i` (the index encoded into the id bytes).
fn mote_at(i: usize) -> MoteId {
    let mut b = [0u8; 32];
    b[..8].copy_from_slice(&(i as u64).to_le_bytes());
    MoteId::from_bytes(b)
}

#[test]
#[ignore = "scale-smoke: run with --release --ignored --nocapture --test-threads=1"]
fn capture_store_is_flat_per_record() {
    let cref = ContentRef::from_bytes([3u8; 32]);
    let mut per_record_ns: Vec<(usize, f64)> = Vec::new();
    for &n in SIZES {
        let mut store = InMemoryCaptureStore::new(CaptureConsent::actions_only());
        let start = Instant::now();
        for i in 0..n {
            store.record(StepRecord::action(mote_at(i), cref));
        }
        let elapsed = start.elapsed();
        // Exactly-once: every distinct Mote recorded, none dropped.
        assert_eq!(store.len(), n, "one record per Mote at n={n}");
        #[allow(clippy::cast_precision_loss)]
        let per = elapsed.as_nanos() as f64 / n as f64;
        println!(
            "capture-store: n={n} total_ms={} per_record_ns={per:.1}",
            elapsed.as_millis()
        );
        per_record_ns.push((n, per));
    }
    // Flat per-record: a `BTreeMap` insert is O(log N), so the 25k/1k ratio is
    // ~log(25k)/log(1k) ≈ 1.47×. Allow generous headroom for small-N timing noise
    // while still catching a genuine super-linear regression (quadratic ≈ 25×).
    let first = per_record_ns.first().unwrap().1;
    let last = per_record_ns.last().unwrap().1;
    assert!(
        last <= first * 4.0,
        "per-record insert must stay flat (n=1k {first:.1}ns vs n=25k {last:.1}ns)"
    );
}
