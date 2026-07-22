// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! Durability + fail-closed integration tests for the HNSW cache: dump → drop →
//! reopen preserves the corpus + its nearest neighbours; absent files open empty;
//! corrupt files and path-traversal are rejected.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::cast_precision_loss)]

use std::path::PathBuf;

use kx_content::ContentRef;
use kx_dataset::{InMemoryRetrievalIndex, RetrievalIndex};
use kx_dataset_hnsw::{dump, open, HnswRetrievalIndex};
use tempfile::tempdir;

fn cref(tag: u64) -> ContentRef {
    let mut b = [0u8; 32];
    b[..8].copy_from_slice(&tag.to_le_bytes());
    ContentRef::from_bytes(b)
}

/// Graded 2-D angular embeddings → an unambiguous cosine nearest neighbour
/// (graph-structure-independent), so durability can be asserted via recall.
fn angle_vec(h: u64) -> Vec<f32> {
    let t = h as f32 * 0.21;
    vec![t.cos(), t.sin()]
}

#[test]
fn dump_then_open_preserves_corpus_and_nearest() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("corpus.hnsw");

    let mut idx = HnswRetrievalIndex::new();
    let mut exact = InMemoryRetrievalIndex::new();
    for h in 0..24u64 {
        let v = angle_vec(h);
        idx.insert(cref(h), v.clone());
        exact.insert(cref(h), v);
    }
    dump(&idx, &path).unwrap();
    drop(idx);

    let reopened = open(&path).unwrap();
    assert_eq!(reopened.len(), 24);
    // The reloaded index returns the same exact nearest neighbour as the
    // brute-force baseline (data + queryability survived the round-trip).
    for h in [0u64, 5, 11, 23] {
        let q = angle_vec(h);
        assert_eq!(
            reopened.query(&q, 1)[0].id,
            exact.query(&q, 1)[0].id,
            "nearest mismatch after reload at {h}"
        );
    }
}

#[test]
fn open_absent_path_is_empty() {
    let dir = tempdir().unwrap();
    let idx = open(&dir.path().join("missing.hnsw")).unwrap();
    assert!(idx.is_empty());
}

#[test]
fn open_corrupt_file_is_err() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("bad.hnsw");
    std::fs::write(&path, b"not a valid kx hnsw cache file").unwrap();
    assert!(open(&path).is_err());
}

#[test]
fn path_traversal_is_rejected_on_open_and_dump() {
    let traversal = PathBuf::from("../escape.hnsw");
    assert!(open(&traversal).is_err());
    let idx = HnswRetrievalIndex::new();
    assert!(dump(&idx, &traversal).is_err());
}
