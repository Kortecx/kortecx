// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! Scale-smoke: building an `InMemoryRetrievalIndex` must be sub-quadratic — the
//! O(1) `HashMap` dedup, not the old O(n) `items` scan on every insert. Doubling N
//! scales an O(n) build ~2x and an O(n^2) build ~4x, so we assert the doubling
//! ratio stays comfortably below 3.0.
//! `#[ignore]`; run by `just scale-smoke` in `--release` (a wall-clock ratio in
//! debug/default mode is a flake vector — keep it gated).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation
)]

use std::time::{Duration, Instant};

use kx_content::ContentRef;
use kx_dataset::{InMemoryRetrievalIndex, RetrievalIndex};

/// Distinct `ContentRef` built by encoding `i` into the ref bytes.
fn ref_n(i: u64) -> ContentRef {
    let mut b = [0u8; 32];
    b[..8].copy_from_slice(&i.to_le_bytes());
    ContentRef::from_bytes(b)
}

/// Wall-clock cost of building an index of `n` unique inserts.
fn build_cost(n: u64) -> Duration {
    let start = Instant::now();
    let mut idx = InMemoryRetrievalIndex::new();
    for i in 0..n {
        idx.insert(ref_n(i), vec![1.0, 0.0, 0.0, 0.0]);
    }
    assert_eq!(idx.len(), n as usize);
    start.elapsed()
}

/// Median of `REPS` builds at `n`. A single wall-clock sample of a few milliseconds is
/// mostly scheduler noise; the median of an odd number of runs is robust to the one
/// sample that lands during a GC pause or a core migration.
fn median_build_cost(n: u64) -> f64 {
    const REPS: usize = 5;
    let mut samples: Vec<f64> = (0..REPS).map(|_| build_cost(n).as_secs_f64()).collect();
    samples.sort_by(|a, b| a.partial_cmp(b).expect("no NaN in a duration"));
    samples[REPS / 2]
}

// RED on the old O(n^2) insert, GREEN after the HashMap dedup.
// Doubling n scales an O(n) build ~2x and an O(n^2) build ~4x. We assert the doubling
// ratio stays sub-quadratic (< 3.0) — comfortably above the ~2x of the fixed code and
// below the ~4x of the quadratic scan.
//
// THE THRESHOLD IS NOT THE THING THAT NEEDED FIXING. It is an ALGORITHMIC boundary,
// halfway between linear and quadratic, and moving it to accommodate noise would stop it
// discriminating between the two — which is the only thing it is for. What was wrong was
// the measurement: at the old N the build took ~5 ms, so a single sample was dominated by
// scheduler jitter and the ratio read 3.07x on runs where nothing had changed. N is now
// large enough that the work dominates, and each point is a median of several runs.
#[test]
#[ignore = "scale-smoke; run via `just scale-smoke` in --release"]
fn insert_build_is_subquadratic() {
    const N: u64 = 200_000;
    // Warm up (allocator / caches) so the ratio reflects algorithmic cost.
    let _ = build_cost(N / 4);
    let t1 = median_build_cost(N);
    let t2 = median_build_cost(2 * N);
    let ratio = t2 / t1.max(1e-9);
    println!("kx-dataset insert scale-smoke: N={N} t1={t1:.4}s t2={t2:.4}s ratio={ratio:.2}x");
    assert!(
        t1 > 0.010,
        "the smaller build took only {t1:.4}s — too short to measure against scheduler \
         noise, so the ratio below would be jitter rather than algorithmic cost. Raise N."
    );
    assert!(
        ratio < 3.0,
        "doubling n scaled build cost {ratio:.2}x (t1={t1:.4}s, t2={t2:.4}s); \
         expected sub-quadratic (~2x). O(n^2) insert scales ~4x."
    );
}
