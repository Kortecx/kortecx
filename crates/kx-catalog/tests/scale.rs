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
    AssetBinding, AssetPath, AssetRef, AssetVersion, CatalogAction, CatalogActionSet,
    CatalogRegistry, Grant, GrantLedger, InMemoryCatalog, InMemoryGrantLedger,
    InMemoryVersionLedger, PartyId, Provenance, RecipeSnapshot, SignatureEntry, TaskSignature,
    TaskSignatureHash, VersionLedger, VersionedContent, MAX_VERSION_CHAIN_DEPTH,
};
use kx_mote::{ModelId, MoteDefHash};
use kx_warrant::{ModelRoute, ResourceCeiling, Role, WarrantSpec};
use kx_workflow::ManifestId;

const SIZES: &[usize] = &[1_000, 5_000, 10_000, 25_000];

/// A non-default-quantitative warrant (qualitative axes are empty defaults).
fn warrant_calls(max_calls: u32) -> WarrantSpec {
    WarrantSpec {
        model_route: ModelRoute {
            model_id: ModelId("m".into()),
            max_input_tokens: 1_000,
            max_output_tokens: 1_000,
            max_calls,
        },
        resource_ceiling: ResourceCeiling {
            cpu_milli: 1_000,
            mem_bytes: 1 << 20,
            wall_clock_ms: 1_000,
            fd_count: 16,
            disk_bytes: 1 << 20,
        },
        ..Default::default()
    }
}

fn role_calls(max_calls: u32) -> Role {
    Role {
        name: "r".into(),
        version: 1,
        spec: warrant_calls(max_calls),
        description: String::new(),
    }
}

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

/// M7.2: grant append + authorization query + warrant resolution stay O(log n)
/// in the number of distinct (party, asset) grants — the index keeps governance
/// sub-linear at catalog scale (a full-log scan would be ~25× at 25k vs 1k).
#[test]
#[ignore = "scale-smoke: run with --release --ignored --nocapture --test-threads=1"]
fn grant_ledger_fold_stays_sublinear() {
    let mut append_ns: Vec<(usize, f64)> = Vec::new();
    let mut auth_ns: Vec<(usize, f64)> = Vec::new();
    let mut resolve_ns: Vec<(usize, f64)> = Vec::new();
    let owner = PartyId::new("owner");
    let owner_root = warrant_calls(100);

    for &n in SIZES {
        let ledger = InMemoryGrantLedger::new();

        // n distinct (party, asset) pairs, each a single root grant of {Use}.
        let assets: Vec<AssetRef> = (0..n)
            .map(|i| AssetRef::Path(AssetPath::new("ns", "c", format!("a{i}")).unwrap()))
            .collect();
        let parties: Vec<PartyId> = (0..n).map(|i| PartyId::new(format!("p{i}"))).collect();
        for a in &assets {
            ledger
                .append_binding(AssetBinding::new(a.clone(), owner.clone()))
                .unwrap();
        }

        let start = Instant::now();
        for (a, p) in assets.iter().zip(&parties) {
            ledger
                .append_grant(Grant::root(
                    a.clone(),
                    owner.clone(),
                    p.clone(),
                    CatalogActionSet::allow([CatalogAction::Use]),
                    role_calls(10),
                ))
                .unwrap();
        }
        let append_elapsed = start.elapsed();

        let start = Instant::now();
        for (a, p) in assets.iter().zip(&parties) {
            assert!(ledger.is_authorized(p, a, CatalogAction::Use));
        }
        let auth_elapsed = start.elapsed();

        let start = Instant::now();
        for (a, p) in assets.iter().zip(&parties) {
            assert!(ledger
                .resolve_effective_warrant_for(p, a, CatalogAction::Use, &owner_root)
                .unwrap()
                .is_some());
        }
        let resolve_elapsed = start.elapsed();

        #[allow(clippy::cast_precision_loss)]
        let per = |d: std::time::Duration| d.as_nanos() as f64 / n as f64;
        println!(
            "grant-ledger: n={n} append_per_ns={:.1} auth_per_ns={:.1} resolve_per_ns={:.1}",
            per(append_elapsed),
            per(auth_elapsed),
            per(resolve_elapsed)
        );
        append_ns.push((n, per(append_elapsed)));
        auth_ns.push((n, per(auth_elapsed)));
        resolve_ns.push((n, per(resolve_elapsed)));
    }

    assert_sublinear("grant-append", &append_ns);
    assert_sublinear("grant-authorize", &auth_ns);
    assert_sublinear("grant-resolve-warrant", &resolve_ns);
}

/// M7.2: an authorization query is BOUNDED by `MAX_DELEGATION_DEPTH` (64),
/// independent of how deep the delegation chain actually is — the `DoS` / stack
/// guard. A 50k-deep chain queries no slower than a 1k-deep one (both walk at
/// most 64 hops, then fail closed), so the cost stays flat as depth grows.
#[test]
#[ignore = "scale-smoke: run with --release --ignored --nocapture --test-threads=1"]
fn deep_chain_query_is_bounded() {
    const DEPTHS: &[usize] = &[1_000, 10_000, 50_000];
    const ITERS: usize = 2_000;
    let mut query_ns: Vec<(usize, f64)> = Vec::new();
    let owner = PartyId::new("owner");
    let acts = CatalogActionSet::allow([CatalogAction::Use, CatalogAction::Delegate]);

    for &depth in DEPTHS {
        let ledger = InMemoryGrantLedger::new();
        let asset = AssetRef::Path(AssetPath::new("ns", "c", "deep").unwrap());
        ledger
            .append_binding(AssetBinding::new(asset.clone(), owner.clone()))
            .unwrap();

        // owner → p0 → p1 → … → p(depth-1), each conveying {Use, Delegate}.
        let leaf = PartyId::new(format!("p{}", depth - 1));
        let root = Grant::root(
            asset.clone(),
            owner.clone(),
            PartyId::new("p0"),
            acts.clone(),
            role_calls(50),
        );
        let mut prev_id = root.grant_id();
        let mut prev_party = PartyId::new("p0");
        ledger.append_grant(root).unwrap();
        for i in 1..depth {
            let p = PartyId::new(format!("p{i}"));
            let g = Grant::delegated(
                prev_id,
                asset.clone(),
                prev_party.clone(),
                p.clone(),
                acts.clone(),
                role_calls(50),
            );
            prev_id = g.grant_id();
            prev_party = p;
            ledger.append_grant(g).unwrap();
        }

        // Query the leaf many times; the fold walks at most 64 hops (then fails
        // closed for depth > 64), so cost is independent of `depth`.
        let start = Instant::now();
        for _ in 0..ITERS {
            let _ = ledger.is_authorized(&leaf, &asset, CatalogAction::Use);
        }
        let elapsed = start.elapsed();
        #[allow(clippy::cast_precision_loss)]
        let per = elapsed.as_nanos() as f64 / ITERS as f64;
        println!("deep-chain: depth={depth} query_per_ns={per:.1}");
        query_ns.push((depth, per));
    }

    // Flat within the 4× headband across a 50× depth increase.
    let first = query_ns.first().unwrap().1;
    let last = query_ns.last().unwrap().1;
    assert!(
        last <= first * 4.0,
        "deep-chain query must be depth-bounded (depth 1k {first:.1}ns vs 50k {last:.1}ns)"
    );
}

/// M7.2 versioning: publish + handle-resolve + history stay O(log n) in the
/// number of distinct handles — the `BTreeMap` indices keep the
/// mutable-handle → immutable-content mapping sub-linear at catalog scale.
#[test]
#[ignore = "scale-smoke: run with --release --ignored --nocapture --test-threads=1"]
fn version_ledger_publish_resolve_history_stay_sublinear() {
    let mut publish_ns: Vec<(usize, f64)> = Vec::new();
    let mut resolve_ns: Vec<(usize, f64)> = Vec::new();
    let mut history_ns: Vec<(usize, f64)> = Vec::new();

    for &n in SIZES {
        let ledger = InMemoryVersionLedger::new();
        let handles: Vec<AssetPath> = (0..n)
            .map(|i| AssetPath::new("ns", "c", format!("a{i}")).unwrap())
            .collect();
        // Build the (distinct-handle) root versions OUTSIDE the timed region.
        let versions: Vec<AssetVersion> = handles
            .iter()
            .map(|h| {
                AssetVersion::root(
                    h.clone(),
                    VersionedContent::Recipe(TaskSignatureHash::from_bytes([1u8; 32])),
                    PartyId::new("p"),
                    Provenance::from_recipe([2u8; 32]),
                )
            })
            .collect();

        let start = Instant::now();
        for v in versions {
            ledger.publish(v).unwrap();
        }
        let publish_elapsed = start.elapsed();
        assert_eq!(ledger.len(), n, "one version per handle at n={n}");

        let start = Instant::now();
        for h in &handles {
            assert!(ledger.resolve(h).is_some(), "published handle must resolve");
        }
        let resolve_elapsed = start.elapsed();

        let start = Instant::now();
        for h in &handles {
            assert_eq!(ledger.history(h).len(), 1, "single-version history");
        }
        let history_elapsed = start.elapsed();

        #[allow(clippy::cast_precision_loss)]
        let per = |d: std::time::Duration| d.as_nanos() as f64 / n as f64;
        println!(
            "version-ledger: n={n} publish_per_ns={:.1} resolve_per_ns={:.1} history_per_ns={:.1}",
            per(publish_elapsed),
            per(resolve_elapsed),
            per(history_elapsed)
        );
        publish_ns.push((n, per(publish_elapsed)));
        resolve_ns.push((n, per(resolve_elapsed)));
        history_ns.push((n, per(history_elapsed)));
    }

    assert_sublinear("version-publish", &publish_ns);
    assert_sublinear("version-resolve", &resolve_ns);
    assert_sublinear("version-history", &history_ns);
}

/// M7.2 versioning: a lineage query is BOUNDED by `MAX_VERSION_CHAIN_DEPTH`
/// (1024), independent of how deep the version chain actually is. A 50k-deep
/// chain's lineage walks at most 1024 hops — without the cap it would walk 50k
/// (≈50× slower), so flatness across a 50× depth increase proves the bound.
#[test]
#[ignore = "scale-smoke: run with --release --ignored --nocapture --test-threads=1"]
fn deep_version_chain_lineage_is_bounded() {
    const DEPTHS: &[usize] = &[1_000, 10_000, 50_000];
    const ITERS: usize = 500;
    let mut query_ns: Vec<(usize, f64)> = Vec::new();

    for &depth in DEPTHS {
        let ledger = InMemoryVersionLedger::new();
        let handle = AssetPath::new("ns", "c", "deep").unwrap();
        let v0 = AssetVersion::root(
            handle.clone(),
            VersionedContent::Recipe(TaskSignatureHash::from_bytes([0u8; 32])),
            PartyId::new("p"),
            Provenance::from_recipe([0u8; 32]),
        );
        let mut prev_id = v0.version_id();
        let mut prev_rev = v0.revision();
        ledger.publish(v0).unwrap();
        for i in 1..depth {
            let v = AssetVersion::successor(
                prev_id,
                prev_rev,
                handle.clone(),
                VersionedContent::Recipe(TaskSignatureHash::from_bytes(
                    [u8::try_from(i % 256).unwrap(); 32],
                )),
                PartyId::new("p"),
                Provenance::from_recipe([0u8; 32]),
            );
            prev_id = v.version_id();
            prev_rev = v.revision();
            ledger.publish(v).unwrap();
        }
        let leaf = prev_id;

        // Query the leaf's lineage many times; the walk caps at MAX_VERSION_CHAIN_DEPTH.
        // Tight length check: EXACTLY min(depth, cap) — catches an early-truncation
        // regression that a loose `<= cap` would pass trivially.
        let start = Instant::now();
        for _ in 0..ITERS {
            let lin = ledger.lineage(&leaf);
            assert_eq!(lin.len(), depth.min(MAX_VERSION_CHAIN_DEPTH));
        }
        let elapsed = start.elapsed();
        #[allow(clippy::cast_precision_loss)]
        let per = elapsed.as_nanos() as f64 / ITERS as f64;
        println!("deep-version-chain: depth={depth} lineage_per_ns={per:.1}");
        query_ns.push((depth, per));
    }

    let first = query_ns.first().unwrap().1;
    let last = query_ns.last().unwrap().1;
    assert!(
        last <= first * 4.0,
        "deep version-chain lineage must be depth-bounded (depth 1k {first:.1}ns vs 50k {last:.1}ns)"
    );
}

/// M7.2 versioning: the realistic enterprise shape — MANY handles each with a
/// fixed-depth version history — keeps publish (the rank-comparison handle-move
/// `Some` branch), resolve, and multi-version history sub-linear as the catalog
/// (handle count) grows. Chain depth is held CONSTANT (`DEPTH`) and the handle
/// count grows with `n`, so each op stays `O(log n)` in the `BTreeMap` indices.
/// Complements `version_ledger_publish_resolve_history_stay_sublinear` (which
/// only times distinct single-version roots and never exercises the handle move).
#[test]
#[ignore = "scale-smoke: run with --release --ignored --nocapture --test-threads=1"]
fn version_chains_publish_resolve_history_stay_sublinear() {
    const DEPTH: usize = 4; // fixed per-handle chain depth (exercises the Some-branch move)
    let mut publish_ns: Vec<(usize, f64)> = Vec::new();
    let mut resolve_ns: Vec<(usize, f64)> = Vec::new();
    let mut history_ns: Vec<(usize, f64)> = Vec::new();

    for &n in SIZES {
        let ledger = InMemoryVersionLedger::new();
        let handle_count = n / DEPTH;
        let handles: Vec<AssetPath> = (0..handle_count)
            .map(|i| AssetPath::new("ns", "c", format!("h{i}")).unwrap())
            .collect();

        // Build all DEPTH-deep chains (causal order) OUTSIDE the timed region.
        let mut versions: Vec<AssetVersion> = Vec::with_capacity(handle_count * DEPTH);
        for h in &handles {
            let v0 = AssetVersion::root(
                h.clone(),
                VersionedContent::Recipe(TaskSignatureHash::from_bytes([0u8; 32])),
                PartyId::new("p"),
                Provenance::from_recipe([0u8; 32]),
            );
            let mut prev_id = v0.version_id();
            let mut prev_rev = v0.revision();
            versions.push(v0);
            for k in 1..DEPTH {
                let v = AssetVersion::successor(
                    prev_id,
                    prev_rev,
                    h.clone(),
                    VersionedContent::Recipe(TaskSignatureHash::from_bytes(
                        [u8::try_from(k).unwrap(); 32],
                    )),
                    PartyId::new("p"),
                    Provenance::from_recipe([0u8; 32]),
                );
                prev_id = v.version_id();
                prev_rev = v.revision();
                versions.push(v);
            }
        }

        let total = versions.len();
        let start = Instant::now();
        for v in versions {
            ledger.publish(v).unwrap(); // exercises the rank-comparison handle move
        }
        let publish_elapsed = start.elapsed();

        let start = Instant::now();
        for h in &handles {
            assert!(ledger.resolve(h).is_some());
        }
        let resolve_elapsed = start.elapsed();

        let start = Instant::now();
        for h in &handles {
            assert_eq!(ledger.history(h).len(), DEPTH, "full multi-version history");
        }
        let history_elapsed = start.elapsed();

        #[allow(clippy::cast_precision_loss)]
        let per = |d: std::time::Duration, by: usize| d.as_nanos() as f64 / by as f64;
        println!(
            "version-chains: n={n} handles={handle_count} depth={DEPTH} publish_per_ns={:.1} resolve_per_ns={:.1} history_per_ns={:.1}",
            per(publish_elapsed, total),
            per(resolve_elapsed, handle_count),
            per(history_elapsed, handle_count)
        );
        publish_ns.push((n, per(publish_elapsed, total)));
        resolve_ns.push((n, per(resolve_elapsed, handle_count)));
        history_ns.push((n, per(history_elapsed, handle_count)));
    }

    assert_sublinear("version-chains-publish", &publish_ns);
    assert_sublinear("version-chains-resolve", &resolve_ns);
    assert_sublinear("version-chains-history", &history_ns);
}

/// M7.3 (D87): exact discovery by namespace / collection stays `O(log n + result)`
/// — NOT an O(n) `list` scan. Each namespace holds a CONSTANT number of entries
/// (`PER_NS`) and the namespace COUNT grows with `n`, so every query returns a
/// fixed-size result and the measured cost isolates the `O(log n)` range-scan seek.
/// A full-table scan would be ~25× at 25k vs 1k; the index keeps it flat.
#[test]
#[ignore = "scale-smoke: run with --release --ignored --nocapture --test-threads=1"]
fn discovery_exact_lookup_stays_sublinear() {
    use kx_catalog::{DiscoveryIndex, InMemoryDiscoveryIndex};

    const PER_NS: usize = 4; // constant result size per namespace
    let mut by_ns: Vec<(usize, f64)> = Vec::new();
    let mut by_col: Vec<(usize, f64)> = Vec::new();

    for &n in SIZES {
        let index = InMemoryDiscoveryIndex::new();
        let ns_count = n / PER_NS;
        for i in 0..n {
            let p = AssetPath::new(format!("ns{}", i / PER_NS), "c", format!("n{i}")).unwrap();
            index.index_path(&p);
        }
        assert_eq!(index.len(), n);

        // Precompute query keys OUTSIDE the timed region (mirrors the registry
        // test convention) so the measurement isolates the lookup, not `format!`.
        let q = 500;
        let ns_queries: Vec<String> = (0..q)
            .map(|j| format!("ns{}", (j * 7) % ns_count))
            .collect();

        let start = Instant::now();
        for ns in &ns_queries {
            assert_eq!(index.by_namespace(ns).len(), PER_NS);
        }
        let ns_elapsed = start.elapsed();

        let start = Instant::now();
        for ns in &ns_queries {
            assert_eq!(index.by_collection(ns, "c").len(), PER_NS);
        }
        let col_elapsed = start.elapsed();

        #[allow(clippy::cast_precision_loss)]
        let per = |d: std::time::Duration| d.as_nanos() as f64 / q as f64;
        println!(
            "discovery-exact: n={n} ns_count={ns_count} by_ns_per_ns={:.1} by_col_per_ns={:.1}",
            per(ns_elapsed),
            per(col_elapsed)
        );
        by_ns.push((n, per(ns_elapsed)));
        by_col.push((n, per(col_elapsed)));
    }

    assert_sublinear("discovery-by-namespace", &by_ns);
    assert_sublinear("discovery-by-collection", &by_col);
}

/// M7.3 (D84/D87): tag-based discovery stays `O(log n + result)` via the inverted
/// tag index — never an O(n) scan over all advisory records. Each tag holds a
/// CONSTANT number of assets (`PER_TAG`); the tag COUNT grows with `n`.
#[test]
#[ignore = "scale-smoke: run with --release --ignored --nocapture --test-threads=1"]
fn discovery_tag_lookup_stays_sublinear() {
    use std::collections::BTreeSet;

    use kx_catalog::{AdvisoryMetadata, AdvisoryMetadataStore, Tag};

    const PER_TAG: usize = 4;
    let mut series: Vec<(usize, f64)> = Vec::new();

    for &n in SIZES {
        let mut store = AdvisoryMetadataStore::new();
        let tag_count = n / PER_TAG;
        for i in 0..n {
            let tag = Tag::new(format!("t{}", i / PER_TAG)).unwrap();
            let asset = AssetRef::Path(AssetPath::new("ns", "c", format!("n{i}")).unwrap());
            let meta = AdvisoryMetadata {
                tags: BTreeSet::from([tag]),
                ..Default::default()
            };
            store.set(asset, meta).unwrap();
        }
        assert_eq!(store.len(), n);

        // Precompute query tags OUTSIDE the timed region; materialize each result
        // into a Vec (what a real discovery caller does) so the measured cost is
        // the realistic get + collect, not a bare iterator `count`.
        let q = 500;
        let query_tags: Vec<Tag> = (0..q)
            .map(|j| Tag::new(format!("t{}", (j * 7) % tag_count)).unwrap())
            .collect();

        let start = Instant::now();
        for tag in &query_tags {
            let hits: Vec<AssetRef> = store.assets_with_tag(tag).cloned().collect();
            assert_eq!(hits.len(), PER_TAG);
        }
        let elapsed = start.elapsed();

        #[allow(clippy::cast_precision_loss)]
        let per = elapsed.as_nanos() as f64 / q as f64;
        println!("discovery-tag: n={n} tag_count={tag_count} per_query_ns={per:.1}");
        series.push((n, per));
    }

    assert_sublinear("discovery-by-tag", &series);
}
