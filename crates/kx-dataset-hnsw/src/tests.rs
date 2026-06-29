// SPDX-License-Identifier: Apache-2.0
//! Unit tests: query correctness, parity with the exact index, determinism,
//! idempotency, dimension handling, and the cache codec round-trip.

use kx_content::ContentRef;
use kx_dataset::{InMemoryRetrievalIndex, RetrievalIndex};

use crate::index::HnswRetrievalIndex;
use crate::persist::{decode_records, encode_records};

fn cref(tag: u8) -> ContentRef {
    ContentRef::from_bytes([tag; 32])
}

/// A pure one-hot vector — orthogonal across `hot`, so cosine top-1 is unambiguous.
fn onehot(dim: usize, hot: usize) -> Vec<f32> {
    let mut v = vec![0.0f32; dim];
    if let Some(slot) = v.get_mut(hot) {
        *slot = 1.0;
    }
    v
}

#[test]
fn nearest_is_matching_onehot() {
    let mut idx = HnswRetrievalIndex::new();
    for h in 0..8usize {
        idx.insert(cref(h as u8), onehot(8, h));
    }
    let hits = idx.query(&onehot(8, 3), 1);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, cref(3));
}

#[test]
fn top1_parity_with_inmemory() {
    let mut hnsw = HnswRetrievalIndex::new();
    let mut exact = InMemoryRetrievalIndex::new();
    for h in 0..16usize {
        hnsw.insert(cref(h as u8), onehot(16, h));
        exact.insert(cref(h as u8), onehot(16, h));
    }
    for h in 0..16usize {
        let q = onehot(16, h);
        let a = hnsw.query(&q, 1);
        let b = exact.query(&q, 1);
        assert_eq!(a[0].id, b[0].id, "top-1 mismatch at {h}");
    }
}

#[test]
fn deterministic_repeat_query_on_fixed_graph() {
    let mut idx = HnswRetrievalIndex::new();
    for h in 0..32usize {
        idx.insert(cref(h as u8), onehot(32, h));
    }
    let q = onehot(32, 7);
    // A fixed graph is deterministic across repeated queries (the committed
    // ordered-ref fact is stable for a given index state).
    assert_eq!(idx.query(&q, 5), idx.query(&q, 5));
}

#[test]
fn tiny_corpus_query_is_exact_and_deterministic() {
    // Regression for T-DATASETS-HNSW-DISCOVER-FLAKE: on a tiny corpus (n <= ef) the
    // query takes the EXACT brute-force path, so the true nearest neighbour is ALWAYS
    // the top hit — the approximate HNSW graph could occasionally MISS it (a random
    // top-hit on a small graph). The gateway `discover_returns_exact_out_refs_and_bp_scores`
    // flake lived here. Rebuild the index 50× (a fresh randomized graph each time) to
    // pin that the result no longer depends on the graph's random layer assignment.
    for _ in 0..50 {
        let mut idx = HnswRetrievalIndex::new();
        idx.insert(cref(0), onehot(3, 0)); // alpha   (axis 0)
        idx.insert(cref(1), onehot(3, 1)); // bravo   (axis 1)
        idx.insert(cref(2), onehot(3, 2)); // charlie (axis 2)
        let hits = idx.query(&onehot(3, 1), 3); // closest to axis 1 ⇒ bravo (cosine 1.0)
        assert_eq!(hits.len(), 3);
        assert_eq!(
            hits[0].id,
            cref(1),
            "the exact nearest (bravo) must ALWAYS rank first"
        );
        assert!(
            (hits[0].score - 1.0).abs() < 1e-6,
            "the exact-match cosine score is ~1.0, got {}",
            hits[0].score
        );
    }
}

#[test]
fn idempotent_duplicate_insert() {
    let mut idx = HnswRetrievalIndex::new();
    idx.insert(cref(1), onehot(4, 0));
    idx.insert(cref(1), onehot(4, 0));
    assert_eq!(idx.len(), 1);
}

#[test]
fn dim_mismatch_insert_skipped_and_query_empty() {
    let mut idx = HnswRetrievalIndex::new();
    idx.insert(cref(1), onehot(4, 0));
    idx.insert(cref(2), onehot(8, 0)); // wrong dim → skipped
    assert_eq!(idx.len(), 1);
    assert!(idx.query(&onehot(8, 0), 1).is_empty()); // wrong-dim query → empty
}

#[test]
fn empty_index_and_k_zero_are_empty() {
    let empty = HnswRetrievalIndex::new();
    assert!(empty.is_empty());
    assert!(empty.query(&onehot(4, 0), 3).is_empty());

    let mut one = HnswRetrievalIndex::new();
    one.insert(cref(1), onehot(4, 0));
    assert!(one.query(&onehot(4, 0), 0).is_empty());
}

#[test]
fn query_respects_k_and_len_bounds() {
    let mut idx = HnswRetrievalIndex::new();
    for h in 0..8usize {
        idx.insert(cref(h as u8), onehot(8, h));
    }
    let q = onehot(8, 2);
    // k larger than the corpus: never returns more than is indexed, and the
    // exact-present vector is the nearest (recall@1 — distance 0 is the global min).
    let many = idx.query(&q, 100);
    assert!(!many.is_empty());
    assert!(many.len() <= idx.len());
    assert_eq!(many[0].id, cref(2));
    // k smaller than the result set: the output is truncated to k.
    let few = idx.query(&q, 2);
    assert!(few.len() <= 2);
    assert_eq!(few[0].id, cref(2));
}

#[test]
fn record_codec_roundtrip() {
    let ids = vec![cref(1), cref(2)];
    let vectors = vec![vec![1.0f32, 2.0, 3.0], vec![4.0, 5.0, 6.0]];
    let bytes = encode_records(3, &ids, &vectors);
    let (dim, recs) = decode_records(&bytes).unwrap();
    assert_eq!(dim, 3);
    assert_eq!(
        recs,
        vec![
            (cref(1), vec![1.0, 2.0, 3.0]),
            (cref(2), vec![4.0, 5.0, 6.0]),
        ]
    );
}

#[test]
fn codec_rejects_garbage_and_truncation() {
    assert!(decode_records(b"nope").is_err());
    let bytes = encode_records(2, &[cref(1)], &[vec![1.0f32, 2.0]]);
    let mut truncated = bytes.clone();
    truncated.truncate(bytes.len() - 1);
    assert!(decode_records(&truncated).is_err());
    let mut trailing = bytes;
    trailing.push(0xFF);
    assert!(decode_records(&trailing).is_err());
}
