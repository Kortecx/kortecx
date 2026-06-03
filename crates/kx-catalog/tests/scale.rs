// SPDX-License-Identifier: Apache-2.0
//! Scale-smoke: the catalog registry stays sub-linear at scale.
//!
//! `#[ignore]`d — run in `--release` via the `scale-smoke` recipe. Registration
//! and lookup are `BTreeMap` insert/get keyed by [`TaskSignatureHash`], so both
//! are O(log n); this proves a large catalog cannot turn a registration or
//! discovery path super-linear. Mirrors `kx-capture/tests/scale.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Instant;

use kx_catalog::{
    CatalogRegistry, InMemoryCatalog, RecipeSnapshot, SignatureEntry, TaskSignature,
    TaskSignatureHash,
};
use kx_mote::MoteDefHash;
use kx_workflow::ManifestId;

const SIZES: &[usize] = &[1_000, 5_000, 10_000, 25_000];

/// A distinct entry for index `i` (the index encoded into the critic hash bytes,
/// so every signature — and thus every key — is unique).
fn entry_at(i: usize) -> SignatureEntry {
    let mut b = [0u8; 32];
    b[..8].copy_from_slice(&(i as u64).to_le_bytes());
    let sig = TaskSignature::model_invariant(MoteDefHash::from_bytes(b));
    SignatureEntry::new(sig, ManifestId([1u8; 32]), RecipeSnapshot::new([2u8; 32]))
}

#[test]
#[ignore = "scale-smoke: run with --release --ignored --nocapture --test-threads=1"]
fn registry_register_and_lookup_stay_sublinear() {
    let mut register_ns: Vec<(usize, f64)> = Vec::new();
    let mut lookup_ns: Vec<(usize, f64)> = Vec::new();

    for &n in SIZES {
        let catalog = InMemoryCatalog::new();

        // Build entries + their hashes OUTSIDE the timed regions.
        let mut hashes: Vec<TaskSignatureHash> = Vec::with_capacity(n);
        let entries: Vec<SignatureEntry> = (0..n)
            .map(|i| {
                let e = entry_at(i);
                hashes.push(e.hash());
                e
            })
            .collect();

        let start = Instant::now();
        for e in entries {
            catalog.register_signature(e).unwrap();
        }
        let register_elapsed = start.elapsed();

        // Exactly-once: every distinct signature stored, none dropped.
        assert_eq!(catalog.len(), n, "one entry per signature at n={n}");

        let start = Instant::now();
        for h in &hashes {
            assert!(catalog.lookup(h).is_some(), "registered hash must resolve");
        }
        let lookup_elapsed = start.elapsed();

        #[allow(clippy::cast_precision_loss)]
        let register_per = register_elapsed.as_nanos() as f64 / n as f64;
        #[allow(clippy::cast_precision_loss)]
        let lookup_per = lookup_elapsed.as_nanos() as f64 / n as f64;
        println!("catalog: n={n} register_per_ns={register_per:.1} lookup_per_ns={lookup_per:.1}");
        register_ns.push((n, register_per));
        lookup_ns.push((n, lookup_per));
    }

    // BTreeMap insert/get are O(log n): the 25k/1k ratio is ~log(25k)/log(1k)
    // ≈ 1.47×. Allow 4× headroom for small-N timing noise while still catching a
    // genuine super-linear regression (a quadratic path would be ≈ 25×).
    assert_sublinear("register", &register_ns);
    assert_sublinear("lookup", &lookup_ns);
}

fn assert_sublinear(label: &str, series: &[(usize, f64)]) {
    let first = series.first().unwrap().1;
    let last = series.last().unwrap().1;
    assert!(
        last <= first * 4.0,
        "{label} must stay sub-linear (n=1k {first:.1}ns vs n=25k {last:.1}ns)"
    );
}
