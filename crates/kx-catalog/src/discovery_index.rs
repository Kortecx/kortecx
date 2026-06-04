// SPDX-License-Identifier: Apache-2.0
//! The sub-linear catalog discovery index (M7.3, D87) — exact metadata lookup by
//! namespace / collection / path-prefix in `O(log n + result)`.
//!
//! The registry ([`crate::CatalogRegistry`]) and version ledger
//! ([`crate::VersionLedger`]) answer EXACT-hash / EXACT-handle queries only; a
//! "list everything in namespace `acme`" against them is an O(n) scan. This index
//! is the secondary structure that keeps namespace/collection/prefix discovery
//! sub-linear.
//!
//! # A rebuildable projection, never a source of truth
//!
//! The index is a **projection**: it can be rebuilt by replaying every
//! `index_path` from the registry/ledger, exactly as lineage is "computed, never
//! stored". A stale index degrades *discoverability* only — a miss can never gate
//! a committed selection (discovery is fuzzy-in / exact-out, D87), so it is
//! advisory in the SN-8 sense.
//!
//! # How the range scan works
//!
//! Every path is stored under one composite ordered key —
//! `"namespace/collection/name"` (the [`crate::AssetPath`] `Display`). Because
//! `/` is outside the legal segment class, the separator is unambiguous. A
//! namespace / collection query is then a half-open range scan over the keys that
//! begin with `"ns/"` / `"ns/col/"` (the trailing `/` makes it segment-exact, so
//! `"acme"` never matches `"acmecorp"`); a path-prefix query is a character-prefix
//! scan for type-ahead. Each is `O(log n)` to seek + `O(result)` to walk (the
//! `take_while` stops at the first non-matching key), bounded by
//! [`MAX_DISCOVERY_RESULT`].

use std::collections::BTreeMap;
use std::sync::RwLock;

use crate::path::{AssetPath, AssetRef};

/// A hard upper bound on a single discovery result. A discovery result is
/// advisory (it never gates), so truncating it cannot escalate anything — it only
/// bounds the response of a pathologically large namespace. Mirrors
/// [`crate::MAX_VERSION_DESCENDANTS`].
pub const MAX_DISCOVERY_RESULT: usize = 65_536;

/// A backend-agnostic secondary index over catalog asset paths that answers
/// namespace / collection / path-prefix queries in `O(log n + result)`.
///
/// A rebuildable PROJECTION (not a source of truth), advisory in the SN-8 sense:
/// it never gates a committed selection. A durable / cloud backend (D94) is a
/// later impl behind this same trait, exactly as [`crate::CatalogRegistry`] and
/// `kx_dataset::RetrievalIndex`.
pub trait DiscoveryIndex {
    /// Add a path-addressed asset to the index. Idempotent (re-indexing the same
    /// path is a no-op).
    fn index_path(&self, path: &AssetPath);

    /// Every asset whose namespace equals `ns` (segment-exact), in [`AssetRef`]
    /// order. `O(log n + result)`.
    fn by_namespace(&self, ns: &str) -> Vec<AssetRef>;

    /// Every asset whose `namespace/collection` equals `(ns, col)` (segment-exact),
    /// in [`AssetRef`] order. `O(log n + result)`.
    fn by_collection(&self, ns: &str, col: &str) -> Vec<AssetRef>;

    /// Every asset whose full `namespace/collection/name` begins with the literal
    /// `prefix` (character-prefix, for type-ahead), in [`AssetRef`] order.
    /// `O(log n + result)`.
    fn by_path_prefix(&self, prefix: &str) -> Vec<AssetRef>;

    /// Repopulate this index by replaying every published handle from a version
    /// ledger, so discovery survives a restart by rebuilding from the durable
    /// truth (G1 / D94). Idempotent ([`DiscoveryIndex::index_path`] is); ADVISORY —
    /// a partial rebuild only degrades discoverability, never gates (a discovery
    /// miss can't reach a committed selection: fuzzy-in / exact-out, SN-8 / D87).
    fn rebuild_from_versions(&self, ledger: &dyn crate::VersionLedger) {
        for version in ledger.list_versions() {
            self.index_path(version.handle());
        }
    }

    /// Number of indexed paths.
    fn len(&self) -> usize;

    /// `true` if nothing is indexed.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The in-memory reference [`DiscoveryIndex`]: a `BTreeMap` keyed by the composite
/// `"namespace/collection/name"` string, answering every query shape by a
/// half-open range scan.
#[derive(Debug, Default)]
pub struct InMemoryDiscoveryIndex {
    by_path: RwLock<BTreeMap<String, AssetRef>>,
}

impl InMemoryDiscoveryIndex {
    /// Create an empty index.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Range-scan every entry whose composite key begins with `prefix`. Seeks to
    /// the first key `>= prefix` (`O(log n)`), then walks while the prefix holds
    /// (stopping at the first non-match), bounded by [`MAX_DISCOVERY_RESULT`].
    ///
    /// The bounded result is re-sorted into [`AssetRef`] order before return:
    /// composite-String order is *not* identical to `AssetRef` (tuple) order
    /// (`-`/`.` byte-sort before the `/` separator), so the range scan finds the
    /// correct SET but in String order; the sort restores the documented
    /// `AssetRef`-ordered, deterministic contract. The sort is `O(result log
    /// result)` over a bounded result — sub-linear in the catalog size `n`.
    fn scan(&self, prefix: &str) -> Vec<AssetRef> {
        let guard = self.by_path.read().expect("poisoned lock");
        let mut out: Vec<AssetRef> = guard
            .range(prefix.to_owned()..)
            .take_while(|(k, _)| k.starts_with(prefix))
            .map(|(_, v)| v.clone())
            .take(MAX_DISCOVERY_RESULT)
            .collect();
        drop(guard);
        out.sort();
        out
    }
}

impl DiscoveryIndex for InMemoryDiscoveryIndex {
    fn index_path(&self, path: &AssetPath) {
        let key = path.to_string();
        self.by_path
            .write()
            .expect("poisoned lock")
            .insert(key, AssetRef::Path(path.clone()));
    }

    fn by_namespace(&self, ns: &str) -> Vec<AssetRef> {
        // Trailing `/` → segment-exact: "acme/" can never be a prefix of
        // "acmecorp/...".
        self.scan(&format!("{ns}/"))
    }

    fn by_collection(&self, ns: &str, col: &str) -> Vec<AssetRef> {
        self.scan(&format!("{ns}/{col}/"))
    }

    fn by_path_prefix(&self, prefix: &str) -> Vec<AssetRef> {
        self.scan(prefix)
    }

    fn len(&self) -> usize {
        self.by_path.read().expect("poisoned lock").len()
    }
}
