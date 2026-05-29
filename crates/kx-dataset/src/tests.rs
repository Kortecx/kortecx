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
