// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! [`KnowledgeGraph`] — the graph-RAG traversal seam — and [`InMemoryGraph`], the
//! OSS-default in-memory backend.

use std::collections::{BTreeMap, BTreeSet};

use kx_content::ContentRef;
use kx_dataset::Hit;

use crate::triple::{normalize_entity, Triple};

/// The maximum neighbour-walk radius. Two hops reaches a bridge fact
/// (`seed —edge— X —edge— result`) — the case pure similarity cannot join — while
/// keeping the neighbourhood bounded. PR-1 fixes the ceiling here; weighted /
/// deeper traversal is a follow-up.
const MAX_HOPS: u32 = 2;

/// A knowledge graph of extracted triples with a multi-hop neighbour walk. Used
/// ONLY inside a ReadOnlyNondet retrieval Mote: the walk returns source
/// [`ContentRef`]s (a similarity-like read), and only the ordered ref SET is
/// committed downstream (SN-8) — traversal never touches a `MoteId`.
pub trait KnowledgeGraph {
    /// Add a triple, indexing its subject + object (normalized) as incident nodes.
    fn insert_triple(&mut self, triple: Triple);

    /// The `k` source refs nearest `seeds` within `hops` (clamped to `1..=MAX_HOPS`),
    /// highest proximity first. A source is "reached" when a triple bearing it has an
    /// endpoint within the hop budget of some seed. Deterministic: the returned order
    /// is a pure function of the triple SET + seeds (independent of insertion order).
    fn neighbors(&self, seeds: &[String], hops: usize, k: usize) -> Vec<Hit>;

    /// The number of triples held.
    fn len(&self) -> usize;

    /// `true` if no triples are held.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The OSS-default in-memory [`KnowledgeGraph`]. Triples in insertion order (never
/// iterated for ranking) plus a normalized-entity → incident-triple-index adjacency
/// map. `BTreeMap`/`BTreeSet` give sorted, insertion-order-independent iteration, so
/// two graphs built from the same triple SET in any order walk identically.
#[derive(Default)]
pub struct InMemoryGraph {
    triples: Vec<Triple>,
    /// normalized entity → the indices of triples it is a subject or object of.
    adjacency: BTreeMap<String, BTreeSet<usize>>,
}

impl InMemoryGraph {
    /// Create an empty graph.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// BFS the entity graph outward from `seeds`, returning `entity → min hop
    /// distance` for every entity within `hops` of some seed (seeds themselves at 0).
    fn distances(&self, seeds: &[String], hops: u32) -> BTreeMap<String, u32> {
        let mut dist: BTreeMap<String, u32> = BTreeMap::new();
        let mut frontier: Vec<String> = Vec::new();
        for seed in seeds {
            let e = normalize_entity(seed);
            if !e.is_empty() && dist.insert(e.clone(), 0).is_none() {
                frontier.push(e);
            }
        }
        for depth in 0..hops {
            let mut next: Vec<String> = Vec::new();
            let reach = depth + 1;
            for ent in &frontier {
                let Some(incident) = self.adjacency.get(ent) else {
                    continue;
                };
                for &ti in incident {
                    let t = &self.triples[ti];
                    for endpoint in [normalize_entity(&t.subject), normalize_entity(&t.object)] {
                        if !endpoint.is_empty() && !dist.contains_key(&endpoint) {
                            dist.insert(endpoint.clone(), reach);
                            next.push(endpoint);
                        }
                    }
                }
            }
            if next.is_empty() {
                break;
            }
            frontier = next;
        }
        dist
    }
}

impl KnowledgeGraph for InMemoryGraph {
    fn insert_triple(&mut self, triple: Triple) {
        let idx = self.triples.len();
        for entity in [
            normalize_entity(&triple.subject),
            normalize_entity(&triple.object),
        ] {
            if !entity.is_empty() {
                self.adjacency.entry(entity).or_default().insert(idx);
            }
        }
        self.triples.push(triple);
    }

    #[allow(clippy::cast_precision_loss)] // display-only score; RRF fuses by rank, not magnitude
    fn neighbors(&self, seeds: &[String], hops: usize, k: usize) -> Vec<Hit> {
        if k == 0 || self.triples.is_empty() {
            return Vec::new();
        }
        let hops = u32::try_from(hops).unwrap_or(MAX_HOPS).clamp(1, MAX_HOPS);
        let dist = self.distances(seeds, hops);
        if dist.is_empty() {
            return Vec::new();
        }

        // Accumulate reached SOURCE refs: `min_hop` = the smallest reachable-endpoint
        // distance over triples bearing that source; `paths` = how many reached
        // triples bear it. Keyed by ContentRef in a BTreeMap so accumulation is
        // sorted + order-independent (min/count are both commutative).
        let mut reached: BTreeMap<ContentRef, (u32, u32)> = BTreeMap::new();
        for t in &self.triples {
            let s = dist.get(&normalize_entity(&t.subject)).copied();
            let o = dist.get(&normalize_entity(&t.object)).copied();
            let Some(hop) = s.into_iter().chain(o).min() else {
                continue; // neither endpoint within reach ⇒ not reached
            };
            let entry = reached.entry(t.source).or_insert((u32::MAX, 0));
            entry.0 = entry.0.min(hop);
            entry.1 += 1;
        }

        let mut ranked: Vec<(ContentRef, u32, u32)> = reached
            .into_iter()
            .map(|(src, (min_hop, paths))| (src, min_hop, paths))
            .collect();
        // Total, deterministic order: nearest hop first, then more connecting paths,
        // then ascending content ref (the fusion.rs / index.rs tiebreak convention).
        ranked.sort_by(|a, b| {
            a.1.cmp(&b.1)
                .then_with(|| b.2.cmp(&a.2))
                .then_with(|| a.0.as_bytes().cmp(b.0.as_bytes()))
        });
        ranked.truncate(k);

        ranked
            .into_iter()
            .map(|(src, min_hop, paths)| Hit {
                id: src,
                // Display-only proximity. RRF fuses by list POSITION, not this score,
                // so its magnitude never affects fusion or the committed fact.
                score: 1.0 / (min_hop as f32 + 1.0) + paths as f32 * 1e-3,
            })
            .collect()
    }

    fn len(&self) -> usize {
        self.triples.len()
    }
}
