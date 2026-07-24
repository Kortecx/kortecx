// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
use kx_content::ContentRef;
use kx_dataset::Hit;

use crate::graph::{InMemoryGraph, KnowledgeGraph};
use crate::triple::{normalize_entity, Triple};

/// A distinct source ref per document label.
fn src(label: &str) -> ContentRef {
    ContentRef::of(label.as_bytes())
}

fn ids(hits: &[Hit]) -> Vec<ContentRef> {
    hits.iter().map(|h| h.id).collect()
}

#[test]
fn normalize_is_case_and_whitespace_stable() {
    assert_eq!(normalize_entity("  Acme   Corp "), "acme corp");
    assert_eq!(normalize_entity("ZENITH"), "zenith");
    assert_eq!(
        normalize_entity("Acme Corp"),
        normalize_entity("acme  corp")
    );
    assert_eq!(normalize_entity("   "), "");
}

#[test]
fn empty_graph_and_empty_seeds_yield_nothing() {
    let g = InMemoryGraph::new();
    assert!(g.is_empty());
    assert!(g.neighbors(&["acme".into()], 2, 8).is_empty());

    let mut g = InMemoryGraph::new();
    g.insert_triple(Triple::new("Acme", "acquired", "Zenith", src("A")));
    assert!(g.neighbors(&[], 2, 8).is_empty()); // no seeds
    assert!(g.neighbors(&["nobody".into()], 2, 8).is_empty()); // unknown seed
    assert!(g.neighbors(&["acme".into()], 2, 0).is_empty()); // k == 0
}

#[test]
fn one_hop_surfaces_the_directly_mentioning_chunk() {
    let mut g = InMemoryGraph::new();
    g.insert_triple(Triple::new("Acme", "acquired", "Zenith", src("A")));
    g.insert_triple(Triple::new("Globex", "makes", "Widgets", src("other")));
    let hits = g.neighbors(&["Acme".into()], 1, 8);
    assert_eq!(ids(&hits), vec![src("A")]); // only the Acme chunk, not the unrelated one
}

#[test]
fn second_hop_reaches_a_chunk_the_first_hop_cannot() {
    // A 3-link chain Acme→Beta→Gamma→Orion. Chunk C mentions Gamma+Orion — NEITHER
    // the seed (Acme) nor its direct neighbour (Beta) — so it is reachable ONLY once
    // the walk has taken two hops (Acme→Beta→Gamma). This is the bridge pure
    // similarity cannot join.
    let mut g = InMemoryGraph::new();
    g.insert_triple(Triple::new("Acme", "partnered", "Beta", src("A"))); // Acme—Beta
    g.insert_triple(Triple::new("Beta", "owns", "Gamma", src("B"))); //     Beta—Gamma
    g.insert_triple(Triple::new("Gamma", "built", "Orion", src("C"))); //   Gamma—Orion

    // 1 hop reaches Beta, so chunks touching {acme, beta} — A and B, NOT C.
    let one = g.neighbors(&["Acme".into()], 1, 8);
    assert_eq!(ids(&one), vec![src("A"), src("B")]);

    // 2 hops reaches Gamma, surfacing chunk C (the answer) — A(0) < B(1) < C(2).
    let two = g.neighbors(&["Acme".into()], 2, 8);
    assert_eq!(ids(&two), vec![src("A"), src("B"), src("C")]);
}

#[test]
fn ranking_is_independent_of_insertion_order() {
    let triples = [
        Triple::new("Acme", "acquired", "Zenith", src("A")),
        Triple::new("Zenith", "built", "Orion", src("B")),
        Triple::new("Orion", "cooledby", "Water", src("C")),
    ];
    // Forward insertion.
    let mut g1 = InMemoryGraph::new();
    for t in &triples {
        g1.insert_triple(t.clone());
    }
    // Reversed insertion.
    let mut g2 = InMemoryGraph::new();
    for t in triples.iter().rev() {
        g2.insert_triple(t.clone());
    }
    let a = g1.neighbors(&["Acme".into()], 2, 8);
    let b = g2.neighbors(&["Acme".into()], 2, 8);
    // Byte-identical hits (ids + score bits) regardless of insertion order.
    assert_eq!(ids(&a), ids(&b));
    for (x, y) in a.iter().zip(b.iter()) {
        assert_eq!(x.score.to_bits(), y.score.to_bits());
    }
    // C is at hop 2 (Orion), still within budget; A(0) < B(1) < C(2).
    assert_eq!(ids(&a), vec![src("A"), src("B"), src("C")]);
}

#[test]
fn paths_break_ties_before_ref_order() {
    // Two sources both first-reached at hop 0, but D carries more connecting triples.
    let mut g = InMemoryGraph::new();
    g.insert_triple(Triple::new("Acme", "rel1", "X", src("D")));
    g.insert_triple(Triple::new("Acme", "rel2", "Y", src("D"))); // D: 2 paths at hop 0
    g.insert_triple(Triple::new("Acme", "rel3", "Z", src("E"))); // E: 1 path at hop 0
    let hits = g.neighbors(&["Acme".into()], 1, 8);
    assert_eq!(ids(&hits), vec![src("D"), src("E")]); // more paths ranks first
}

#[test]
fn k_truncates_to_the_nearest() {
    let mut g = InMemoryGraph::new();
    g.insert_triple(Triple::new("Acme", "acquired", "Zenith", src("A")));
    g.insert_triple(Triple::new("Zenith", "built", "Orion", src("B")));
    g.insert_triple(Triple::new("Orion", "cooledby", "Water", src("C")));
    let hits = g.neighbors(&["Acme".into()], 2, 2);
    assert_eq!(ids(&hits), vec![src("A"), src("B")]); // C (hop 2) dropped at k=2
}

#[test]
fn hops_is_clamped_to_the_ceiling() {
    // hops=99 behaves as MAX_HOPS (2): Water (hop 3) is never reached.
    let mut g = InMemoryGraph::new();
    g.insert_triple(Triple::new("Acme", "acquired", "Zenith", src("A")));
    g.insert_triple(Triple::new("Zenith", "built", "Orion", src("B")));
    g.insert_triple(Triple::new("Orion", "cooledby", "Water", src("C")));
    g.insert_triple(Triple::new("Water", "flowsto", "Sea", src("far")));
    let hits = g.neighbors(&["Acme".into()], 99, 8);
    // A(0), B(1), C(2 via Orion) reached; "far" (only Water@3 / Sea@4) excluded.
    assert_eq!(ids(&hits), vec![src("A"), src("B"), src("C")]);
}
