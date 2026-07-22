// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! Scale-smoke: a 25k-vector HNSW must answer queries materially faster than the
//! exact O(n) brute-force scan — the whole point of the opt-in DP3 backend.
//! `#[ignore]`; run by `just scale-smoke` in `--release`.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation
)]

use std::time::Instant;

use kx_content::ContentRef;
use kx_dataset::{InMemoryRetrievalIndex, RetrievalIndex};
use kx_dataset_hnsw::HnswRetrievalIndex;

/// Cheap deterministic pseudo-vectors via xorshift — no `rand` dependency.
fn vec_for(seed: u64, dim: usize) -> Vec<f32> {
    let mut s = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1);
    (0..dim)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            // top 24 bits → [0, 1) → [-0.5, 0.5)
            (s >> 40) as f32 / 16_777_216.0 - 0.5
        })
        .collect()
}

fn cref(i: u64) -> ContentRef {
    let mut b = [0u8; 32];
    b[..8].copy_from_slice(&i.to_le_bytes());
    ContentRef::from_bytes(b)
}

#[test]
#[ignore = "scale-smoke; run via `just scale-smoke` in --release"]
fn hnsw_25k_query_beats_brute_force() {
    const N: u64 = 25_000;
    const DIM: usize = 64;

    let mut hnsw = HnswRetrievalIndex::new();
    let mut exact = InMemoryRetrievalIndex::new();
    for i in 0..N {
        let v = vec_for(i, DIM);
        hnsw.insert(cref(i), v.clone());
        exact.insert(cref(i), v);
    }
    let q = vec_for(7_777, DIM);

    let t = Instant::now();
    for _ in 0..50 {
        let _ = hnsw.query(&q, 10);
    }
    let ann = t.elapsed() / 50;

    let t = Instant::now();
    for _ in 0..5 {
        let _ = exact.query(&q, 10);
    }
    let brute = t.elapsed() / 5;

    println!("hnsw scale-smoke: N={N} dim={DIM} ann={ann:?} brute={brute:?}");
    assert!(
        ann < brute,
        "ANN query ({ann:?}) must beat exact brute-force ({brute:?}) at {N} vectors"
    );
}
