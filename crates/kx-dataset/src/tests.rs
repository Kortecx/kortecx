//! Unit tests: typed store roundtrip + schema typing, dataset identity purity +
//! journal-authoritative reconstruction, and deterministic retrieval ordering.

use kx_content::ContentRef;
use kx_mote::MoteId;
use smallvec::smallvec;

use crate::{
    ContentSchema, DataStore, Dataset, InMemoryDataStore, InMemoryRetrievalIndex, RetrievalIndex,
    TensorDType, TypedRef,
};

#[test]
fn datastore_roundtrip_and_schema_typing() {
    let store = InMemoryDataStore::new();
    let schema = ContentSchema::Tensor {
        dtype: TensorDType::F32,
        shape: smallvec![2, 3],
    };
    let bytes = b"\x00\x01\x02\x03payload";

    let typed = store.put_typed(bytes, schema.clone()).unwrap();
    assert_eq!(typed.content_ref, ContentRef::of(bytes));
    assert_eq!(typed.schema, schema);
    assert!(store.contains(&typed.content_ref));
    assert_eq!(store.schema_of(&typed.content_ref), Some(schema.clone()));

    let (got_bytes, got_schema) = store.get(&typed.content_ref).unwrap();
    assert_eq!(got_bytes, bytes);
    assert_eq!(got_schema, schema);

    // Idempotent on the bytes.
    let again = store.put_typed(bytes, schema).unwrap();
    assert_eq!(again.content_ref, typed.content_ref);
}

#[test]
fn missing_ref_is_not_found() {
    let store = InMemoryDataStore::new();
    let missing = ContentRef::from_bytes([9; 32]);
    assert!(!store.contains(&missing));
    assert!(store.get(&missing).is_err());
    assert_eq!(store.schema_of(&missing), None);
}

/// A dataset's identity is a PURE function of its rows + lineage — independent
/// of any store. Rebuilding the store from the same committed content
/// reconstructs the same DatasetId (journal-authoritative: the store is a
/// cache, the corpus identity is durable-by-reference).
#[test]
fn dataset_id_is_pure_and_reconstructible() {
    let schema = ContentSchema::Vector { dim: 3 };
    let mk = |store: &InMemoryDataStore| -> Dataset {
        let a = store.put_typed(b"row-a", schema.clone()).unwrap();
        let b = store.put_typed(b"row-b", schema.clone()).unwrap();
        Dataset::new(vec![a, b], vec![MoteId::from_bytes([7; 32])])
    };

    // Two independent stores fed the same committed content → same DatasetId.
    let id1 = mk(&InMemoryDataStore::new()).id();
    let id2 = mk(&InMemoryDataStore::new()).id();
    assert_eq!(
        id1, id2,
        "DatasetId must be reproducible across store instances"
    );

    // And id() does not depend on a store at all — pure over rows + lineage.
    let rows = vec![
        TypedRef {
            content_ref: ContentRef::of(b"row-a"),
            schema: schema.clone(),
        },
        TypedRef {
            content_ref: ContentRef::of(b"row-b"),
            schema,
        },
    ];
    let storeless = Dataset::new(rows, vec![MoteId::from_bytes([7; 32])]).id();
    assert_eq!(id1, storeless);
}

#[test]
fn dataset_id_is_sensitive_to_rows_and_lineage() {
    let s = ContentSchema::Blob;
    let base = Dataset::new(
        vec![TypedRef {
            content_ref: ContentRef::of(b"x"),
            schema: s.clone(),
        }],
        vec![MoteId::from_bytes([1; 32])],
    );
    let diff_row = Dataset::new(
        vec![TypedRef {
            content_ref: ContentRef::of(b"y"),
            schema: s.clone(),
        }],
        vec![MoteId::from_bytes([1; 32])],
    );
    let diff_lineage = Dataset::new(
        vec![TypedRef {
            content_ref: ContentRef::of(b"x"),
            schema: s,
        }],
        vec![MoteId::from_bytes([2; 32])],
    );
    assert_ne!(base.id(), diff_row.id());
    assert_ne!(base.id(), diff_lineage.id());
}

#[test]
fn retrieval_is_deterministic_and_orders_by_similarity() {
    let mut index = InMemoryRetrievalIndex::new();
    let near = ContentRef::of(b"near");
    let mid = ContentRef::of(b"mid");
    let far = ContentRef::of(b"far");
    index.insert(near, vec![1.0, 0.0, 0.0]);
    index.insert(mid, vec![0.7, 0.7, 0.0]);
    index.insert(far, vec![0.0, 0.0, 1.0]);

    let q = [1.0, 0.0, 0.0];
    let hits = index.query(&q, 3);
    assert_eq!(hits.len(), 3);
    assert_eq!(hits[0].id, near, "closest vector ranks first");
    assert_eq!(hits[1].id, mid);
    assert_eq!(hits[2].id, far);

    // Deterministic across calls.
    assert_eq!(index.query(&q, 3), hits);

    // top-k truncation.
    assert_eq!(index.query(&q, 1).len(), 1);
    assert_eq!(index.query(&q, 1)[0].id, near);
}

#[test]
fn retrieval_skips_dimension_mismatch() {
    let mut index = InMemoryRetrievalIndex::new();
    let ok = ContentRef::of(b"ok");
    let wrong = ContentRef::of(b"wrong");
    index.insert(ok, vec![1.0, 0.0]);
    index.insert(wrong, vec![1.0, 0.0, 0.0]); // different dim → cosine 0.0

    let hits = index.query(&[1.0, 0.0], 2);
    assert_eq!(hits[0].id, ok);
    // The mismatched entry scores 0.0 and ranks last.
    assert_eq!(hits[1].id, wrong);
    assert!(hits[0].score > hits[1].score);
}

#[test]
fn vector_of_returns_the_stored_vector() {
    let mut index = InMemoryRetrievalIndex::new();
    let a = ContentRef::of(b"a");
    index.insert(a, vec![1.0, 2.0, 3.0]);
    assert_eq!(index.vector_of(&a), Some(vec![1.0, 2.0, 3.0]));
    assert_eq!(index.vector_of(&ContentRef::of(b"absent")), None);
}

// Correctness guard for the O(1) HashMap dedup: re-inserting the same ref must
// overwrite in place (not grow the index) and preserve the exact query result /
// ordering the brute-force index promised. Timing-free, so it lives here in the
// default suite; the doubling-ratio scaling test is gated in `tests/scale.rs`.
#[test]
fn insert_dedup_overwrites_and_preserves_results() {
    fn ref_n(i: u64) -> ContentRef {
        let mut b = [0u8; 32];
        b[..8].copy_from_slice(&i.to_le_bytes());
        ContentRef::from_bytes(b)
    }

    let mut idx = InMemoryRetrievalIndex::new();
    let a = ref_n(1);
    let b = ref_n(2);
    idx.insert(a, vec![1.0, 0.0]);
    idx.insert(b, vec![0.0, 1.0]);
    // Re-insert `a` with a new vector: must overwrite, not append.
    idx.insert(a, vec![0.0, 1.0]);
    assert_eq!(idx.len(), 2, "re-insert must dedup, not grow the index");
    assert_eq!(idx.vector_of(&a), Some(vec![0.0, 1.0]));
    assert_eq!(idx.vector_of(&b), Some(vec![0.0, 1.0]));
    // Both now equal the query; deterministic tiebreak = ascending ref, so
    // `a` (smaller bytes) precedes `b`.
    let hits = idx.query(&[0.0, 1.0], 2);
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].id, a);
    assert_eq!(hits[1].id, b);
    assert!((hits[0].score - 1.0).abs() < 1e-6);
}

// ---- hybrid fusion (RRF) + diversity rerank (MMR) + index fingerprint ----

use crate::fusion::{index_fingerprint, mmr_rerank, rrf_fuse};
use crate::Hit;

fn hit(tag: u8, score: f32) -> Hit {
    Hit {
        id: ContentRef::from_bytes([tag; 32]),
        score,
    }
}

#[test]
fn rrf_rewards_agreement_across_both_rankers() {
    // Doc 1 is mid-rank in BOTH lists; docs 0 and 2 each top exactly one list.
    let dense = vec![hit(0, 0.9), hit(1, 0.8), hit(3, 0.1)];
    let sparse = vec![hit(2, 5.0), hit(1, 4.0), hit(4, 0.5)];
    let fused = rrf_fuse(&dense, &sparse, 60, 5);
    // Doc 1 (ranked in both) outranks docs that appear in only one list.
    assert_eq!(fused[0].id, hit(1, 0.0).id, "agreed-on doc wins");
}

#[test]
fn rrf_is_deterministic_and_truncates() {
    let dense = vec![hit(0, 0.9), hit(1, 0.8)];
    let sparse = vec![hit(1, 4.0), hit(2, 3.0)];
    assert_eq!(rrf_fuse(&dense, &sparse, 60, 2).len(), 2);
    assert_eq!(
        rrf_fuse(&dense, &sparse, 60, 9),
        rrf_fuse(&dense, &sparse, 60, 9)
    );
    assert!(rrf_fuse(&dense, &sparse, 60, 0).is_empty());
}

#[test]
fn mmr_demotes_a_near_duplicate() {
    // Relevance is the INCOMING score: `a` (1.0) > `b` (0.9) > `c` (0.5). `a` and `b`
    // are near-duplicate vectors; `c` is diverse. MMR keeps the top `a`, then must
    // rank the DIVERSE `c` ABOVE the redundant `b` — even though `b` is more
    // relevant than `c` — because `b` is near-identical to the already-picked `a`.
    let a = hit(0, 1.0);
    let b = hit(1, 0.9);
    let c = hit(2, 0.5);
    let vecs = |id: &ContentRef| -> Option<Vec<f32>> {
        if *id == a.id {
            Some(vec![1.0, 0.0])
        } else if *id == b.id {
            Some(vec![0.99, 0.14]) // ~identical to `a`
        } else {
            Some(vec![0.0, 1.0]) // diverse
        }
    };
    let ranked = mmr_rerank(&[a, b, c], vecs, 0.5, 3);
    assert_eq!(ranked[0].id, a.id, "the most relevant is kept first");
    assert_eq!(
        ranked[1].id, c.id,
        "diversity: the diverse doc beats the more-relevant near-duplicate"
    );
    assert_eq!(ranked[2].id, b.id, "the near-duplicate is demoted last");
}

#[test]
fn mmr_preserves_the_top_fused_hit_and_truncates() {
    // The first pick is always the highest incoming (fused) score, so MMR never
    // discards the best hit; out_k truncates.
    let cands = vec![hit(0, 0.2), hit(1, 0.9), hit(2, 0.5)];
    let vecs = |_: &ContentRef| Some(vec![1.0, 0.0]);
    let out = mmr_rerank(&cands, vecs, 0.7, 2);
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].id, hit(1, 0.0).id, "the top fused score leads");
}

#[test]
fn fingerprint_changes_with_config_and_is_stable() {
    let base = index_fingerprint("embeddinggemma", 0, 768, 1, 1000, 200, 1, false);
    // Stable for identical inputs.
    assert_eq!(
        base,
        index_fingerprint("embeddinggemma", 0, 768, 1, 1000, 200, 1, false)
    );
    // A different embed model ⇒ a different fingerprint (the silent-mismatch guard).
    assert_ne!(
        base,
        index_fingerprint("nomic-embed", 0, 768, 1, 1000, 200, 1, false)
    );
    // A different chunk size ⇒ a different fingerprint.
    assert_ne!(
        base,
        index_fingerprint("embeddinggemma", 0, 768, 1, 500, 200, 1, false)
    );
    // A different dimension ⇒ a different fingerprint.
    assert_ne!(
        base,
        index_fingerprint("embeddinggemma", 0, 384, 1, 1000, 200, 1, false)
    );
}
