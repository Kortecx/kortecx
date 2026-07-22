// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! Property + scenario tests for M7.3 exact discovery (D87): the range-scan index
//! matches a brute-force scan for namespace / collection / path-prefix, results
//! are deterministic [`AssetRef`]-ordered, namespace/collection matching is
//! segment-exact, and `by_signature` is the exact registry lookup.

#![allow(clippy::unwrap_used)]

use kx_catalog::{
    AssetPath, AssetRef, CatalogDiscovery, CatalogRegistry, DiscoveryIndex, InMemoryCatalog,
    InMemoryDiscoveryIndex, RecipeSnapshot, SignatureEntry, TaskSignature, TaskSignatureHash,
};
use kx_mote::MoteDefHash;
use kx_workflow::ManifestId;
use proptest::prelude::*;

// Small alphabets so collisions + prefix overlaps actually occur.
fn path(i: usize) -> AssetPath {
    AssetPath::new(
        format!("ns{}", i % 3),
        format!("col{}", i % 4),
        format!("n{i}"),
    )
    .unwrap()
}

fn pref(p: &AssetPath) -> AssetRef {
    AssetRef::Path(p.clone())
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

    #[test]
    fn by_namespace_matches_linear_scan(ids in prop::collection::vec(0usize..60, 0..40)) {
        let index = InMemoryDiscoveryIndex::new();
        let mut all: Vec<AssetPath> = Vec::new();
        for i in &ids {
            let p = path(*i);
            index.index_path(&p);
            all.push(p);
        }
        for nsk in 0..3 {
            let ns = format!("ns{nsk}");
            let via_index = index.by_namespace(&ns);
            let mut via_scan: Vec<AssetRef> = all
                .iter()
                .filter(|p| p.namespace() == ns)
                .map(pref)
                .collect();
            via_scan.sort();
            via_scan.dedup();
            prop_assert_eq!(via_index, via_scan);
        }
    }

    #[test]
    fn by_collection_matches_linear_scan(ids in prop::collection::vec(0usize..60, 0..40)) {
        let index = InMemoryDiscoveryIndex::new();
        let mut all: Vec<AssetPath> = Vec::new();
        for i in &ids {
            let p = path(*i);
            index.index_path(&p);
            all.push(p);
        }
        for nsk in 0..3 {
            for colk in 0..4 {
                let (ns, col) = (format!("ns{nsk}"), format!("col{colk}"));
                let via_index = index.by_collection(&ns, &col);
                let mut via_scan: Vec<AssetRef> = all
                    .iter()
                    .filter(|p| p.namespace() == ns && p.collection() == col)
                    .map(pref)
                    .collect();
                via_scan.sort();
                via_scan.dedup();
                prop_assert_eq!(via_index, via_scan);
            }
        }
    }

    #[test]
    fn by_path_prefix_matches_linear_scan(
        ids in prop::collection::vec(0usize..60, 0..40),
        plen in 1usize..6,
    ) {
        let index = InMemoryDiscoveryIndex::new();
        let mut all: Vec<AssetPath> = Vec::new();
        for i in &ids {
            let p = path(*i);
            index.index_path(&p);
            all.push(p);
        }
        // Derive a prefix from a stable key (all chars ASCII → any byte split ok).
        let prefix: String = path(0).to_string().chars().take(plen).collect();
        let via_index = index.by_path_prefix(&prefix);
        let mut via_scan: Vec<AssetRef> = all
            .iter()
            .filter(|p| p.to_string().starts_with(&prefix))
            .map(pref)
            .collect();
        via_scan.sort();
        via_scan.dedup();
        prop_assert_eq!(via_index, via_scan);
    }

    /// Re-indexing the same path is idempotent (no duplicate result).
    #[test]
    fn index_path_is_idempotent(i in 0usize..50, reps in 1usize..5) {
        let index = InMemoryDiscoveryIndex::new();
        let p = path(i);
        for _ in 0..reps {
            index.index_path(&p);
        }
        prop_assert_eq!(index.len(), 1);
        prop_assert_eq!(index.by_namespace(p.namespace()), vec![pref(&p)]);
    }
}

#[test]
fn namespace_match_is_segment_exact() {
    let index = InMemoryDiscoveryIndex::new();
    let inside = AssetPath::new("ns", "c", "x").unwrap();
    let sibling = AssetPath::new("nsx", "c", "y").unwrap(); // "nsx" must NOT match "ns"
    index.index_path(&inside);
    index.index_path(&sibling);
    assert_eq!(index.by_namespace("ns"), vec![pref(&inside)]);
    assert_eq!(index.by_namespace("nsx"), vec![pref(&sibling)]);
}

#[test]
fn collection_match_is_segment_exact() {
    let index = InMemoryDiscoveryIndex::new();
    let inside = AssetPath::new("ns", "col", "x").unwrap();
    let sibling = AssetPath::new("ns", "colx", "y").unwrap(); // "colx" must NOT match "col"
    index.index_path(&inside);
    index.index_path(&sibling);
    assert_eq!(index.by_collection("ns", "col"), vec![pref(&inside)]);
}

/// The String-vs-tuple ordering edge: `-` and `.` byte-sort before the `/`
/// separator, so the composite-key scan order differs from `AssetRef` order. The
/// scan re-sorts to `AssetRef` order — assert it directly.
#[test]
fn results_are_assetref_ordered_across_separator_edge() {
    let index = InMemoryDiscoveryIndex::new();
    let plain = AssetPath::new("ns", "b", "z").unwrap(); // "ns/b/z"
    let dashed = AssetPath::new("ns", "b-c", "a").unwrap(); // "ns/b-c/a" (String-sorts FIRST)
    index.index_path(&plain);
    index.index_path(&dashed);
    let got = index.by_namespace("ns");
    let mut expect = vec![pref(&plain), pref(&dashed)];
    expect.sort(); // AssetRef (tuple) order: "b" < "b-c" → plain first
    assert_eq!(got, expect);
    assert_eq!(
        got[0],
        pref(&plain),
        "AssetRef order, not composite-String order"
    );
}

#[test]
fn empty_prefix_returns_all() {
    let index = InMemoryDiscoveryIndex::new();
    for i in 0..5 {
        index.index_path(&path(i));
    }
    assert_eq!(index.by_path_prefix("").len(), 5);
}

#[test]
fn by_signature_is_exact_registry_lookup() {
    let registry = InMemoryCatalog::new();
    let index = InMemoryDiscoveryIndex::new();
    let sig = TaskSignature::model_invariant(MoteDefHash::from_bytes([7u8; 32]));
    let entry = SignatureEntry::new(sig, ManifestId([1u8; 32]), RecipeSnapshot::new([2u8; 32]));
    let h = entry.hash();
    registry.register_signature(entry.clone()).unwrap();
    let disc = CatalogDiscovery::new(registry, index);
    assert_eq!(disc.by_signature(&h), Some(entry));
    assert!(disc
        .by_signature(&TaskSignatureHash::from_bytes([9u8; 32]))
        .is_none());
}
