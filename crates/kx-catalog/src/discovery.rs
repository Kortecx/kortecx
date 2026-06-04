// SPDX-License-Identifier: Apache-2.0
//! Catalog discovery (M7.3, D87) — **fuzzy-in, exact-out**.
//!
//! Two strictly-separated tiers:
//!
//! - **(a) EXACT metadata lookup** — deterministic, registry/index-backed:
//!   [`CatalogDiscovery::by_namespace`] / `by_collection` / `by_path_prefix` (via
//!   the [`DiscoveryIndex`]), `by_tag` (via the [`AdvisoryMetadataStore`]), and
//!   `by_signature` (the registry's exact hash lookup).
//! - **(b) FUZZY vector discovery** — [`FuzzyDiscovery`], a thin wrapper over a
//!   `kx_dataset::RetrievalIndex` confined to `ReadOnlyNondet` semantics (the same
//!   SN-8 boundary `kx_workflow::retrieval` documents). Embeddings are OPAQUE,
//!   caller-supplied vectors; the catalog never computes them.
//!
//! # The hard SN-8 boundary
//!
//! The ONLY thing that becomes a committed selection is the chosen EXACT
//! [`AssetRef`] — a [`SelectionFact`] has **no score field**, and
//! [`commit_selection`] never reads a [`Hit`]'s score. Two index states returning
//! the same neighbours produce a byte-identical `SelectionFact` regardless of the
//! float scores, so similarity can never reach the identity / commit path. This is
//! the narrowed SN-8 (D70) applied to the catalog, mirroring
//! `kx_workflow::encode_retrieval_fact`.

use kx_content::ContentRef;
use kx_dataset::{Hit, RetrievalIndex};
use serde::{Deserialize, Serialize};

use crate::discovery_index::DiscoveryIndex;
use crate::entry::SignatureEntry;
use crate::metadata::{AdvisoryMetadataStore, Tag};
use crate::path::AssetRef;
use crate::registry::CatalogRegistry;
use crate::signature::{canonical_config, TaskSignatureHash};

/// The exact-lookup discovery facade: composes a [`CatalogRegistry`] (exact hash
/// lookup) with a [`DiscoveryIndex`] (namespace / collection / prefix), without
/// coupling either backend (the `GovernedCatalog` composition pattern). Tag lookup
/// borrows an [`AdvisoryMetadataStore`] explicitly — advisory metadata is mutable
/// and lives independently of the immutable registry.
pub struct CatalogDiscovery<R: CatalogRegistry, D: DiscoveryIndex> {
    registry: R,
    index: D,
}

impl<R: CatalogRegistry, D: DiscoveryIndex> CatalogDiscovery<R, D> {
    /// Compose a registry and a discovery index into the exact-lookup facade.
    pub fn new(registry: R, index: D) -> Self {
        Self { registry, index }
    }

    /// Borrow the underlying registry.
    pub fn registry(&self) -> &R {
        &self.registry
    }

    /// Borrow the underlying discovery index.
    pub fn index(&self) -> &D {
        &self.index
    }

    /// Every asset whose namespace equals `ns` (segment-exact), [`AssetRef`] order.
    pub fn by_namespace(&self, ns: &str) -> Vec<AssetRef> {
        self.index.by_namespace(ns)
    }

    /// Every asset whose `namespace/collection` equals `(ns, col)` (segment-exact).
    pub fn by_collection(&self, ns: &str, col: &str) -> Vec<AssetRef> {
        self.index.by_collection(ns, col)
    }

    /// Every asset whose path begins with the literal `prefix` (type-ahead).
    pub fn by_path_prefix(&self, prefix: &str) -> Vec<AssetRef> {
        self.index.by_path_prefix(prefix)
    }

    /// Every asset carrying `tag`, from the advisory metadata store (`O(log n + result)`).
    pub fn by_tag<'m>(&self, store: &'m AdvisoryMetadataStore, tag: &Tag) -> Vec<&'m AssetRef> {
        store.assets_with_tag(tag).collect()
    }

    /// The exact registry entry for a recipe signature hash, if registered.
    pub fn by_signature(&self, hash: &TaskSignatureHash) -> Option<SignatureEntry> {
        self.registry.lookup(hash)
    }
}

/// The fuzzy discovery surface — discovery-only, `ReadOnlyNondet` semantics. Holds
/// a `kx_dataset::RetrievalIndex` of OPAQUE caller-supplied embeddings keyed by
/// [`ContentRef`]. A [`Hit`]'s score lives entirely INSIDE this surface (for
/// ranking display only); it never crosses into a [`SelectionFact`].
pub struct FuzzyDiscovery<I: RetrievalIndex> {
    index: I,
}

impl<I: RetrievalIndex> FuzzyDiscovery<I> {
    /// Wrap a retrieval index.
    pub fn new(index: I) -> Self {
        Self { index }
    }

    /// Insert (or overwrite) an opaque embedding for a content ref. The catalog
    /// does NOT compute embeddings — the caller supplies the vector.
    pub fn embed(&mut self, id: ContentRef, vector: Vec<f32>) {
        self.index.insert(id, vector);
    }

    /// Top-`k` nearest candidates to `query`, highest score first (deterministic
    /// order — the retrieval index tiebreaks by content ref). Scores are included
    /// HERE, inside the boundary; promote to a committed selection via
    /// [`commit_selection`], which drops them.
    pub fn query(&self, query: &[f32], k: usize) -> Vec<Hit> {
        self.index.query(query, k)
    }

    /// Borrow the underlying retrieval index.
    pub fn index(&self) -> &I {
        &self.index
    }

    /// Number of indexed embeddings.
    pub fn len(&self) -> usize {
        self.index.len()
    }

    /// `true` if nothing is embedded.
    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }
}

/// A committed catalog selection: the EXACT chosen asset ref(s), in canonical
/// order. Scores are **structurally absent** — there is no score field, so
/// similarity can never reach a committed output (SN-8 / D87 "fuzzy-in,
/// exact-out"). The content-addressed identity is a pure function of the ref set
/// ONLY.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct SelectionFact {
    chosen: Vec<AssetRef>,
}

impl SelectionFact {
    /// The chosen exact asset refs, canonical (sorted, deduped) order.
    #[must_use]
    pub fn chosen(&self) -> &[AssetRef] {
        &self.chosen
    }

    /// `true` if no asset was selected (every candidate was unresolvable).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.chosen.is_empty()
    }

    /// Canonical bytes of the selection — the chosen ref set ONLY (scores absent),
    /// via the catalog's canonical bincode. The one place "fuzzy in, exact out" is
    /// enforced in code (mirrors `kx_workflow::encode_retrieval_fact`).
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        bincode::serde::encode_to_vec(&self.chosen, canonical_config())
            .expect("canonical bincode of a float-free Vec<AssetRef> is infallible")
    }

    /// The content-addressed identity of the selection — what a caller commits /
    /// consumes by exact hash. Pure over the chosen ref set (scores excluded).
    #[must_use]
    pub fn selection_ref(&self) -> ContentRef {
        ContentRef::of(&self.encode())
    }
}

/// Turn a fuzzy discovery result into a committed [`SelectionFact`]: resolve each
/// candidate [`Hit`]'s content ref → the EXACT [`AssetRef`] it stands for (via the
/// caller's `resolve` map), DROPPING scores at this boundary. Unresolvable hits
/// are filtered out (only an exact, resolvable ref can become a selection). Pure +
/// total; the result is canonical (sorted + deduped) so it is independent of the
/// score-driven hit order.
///
/// **This is the single chokepoint where fuzzy becomes exact** — `h.score` is
/// never read, and [`SelectionFact`] has no score field, so the SN-8 boundary is
/// structural.
pub fn commit_selection(
    hits: &[Hit],
    resolve: impl Fn(&ContentRef) -> Option<AssetRef>,
) -> SelectionFact {
    let mut chosen: Vec<AssetRef> = hits.iter().filter_map(|h| resolve(&h.id)).collect();
    chosen.sort();
    chosen.dedup();
    SelectionFact { chosen }
}
