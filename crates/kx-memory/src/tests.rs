// SPDX-License-Identifier: Apache-2.0
//! Deterministic unit tests for the memory store — recall ordering, content-address
//! dedup, namespace isolation, the episodic log, forget, the vector-space guards,
//! and rebuild-on-open durability. No model, no float on any identity path.

use kx_content::ContentRef;

use crate::record::{memory_id, MemoryKind, StoreRequest};
use crate::{MemoryError, MemoryStore, SqliteMemoryStore};

const NS: &str = "mem::local-dev";

fn req<'a>(ns: &'a str, content: &'a [u8], vector: &'a [f32]) -> StoreRequest<'a> {
    StoreRequest {
        namespace: ns,
        content,
        vector,
        kind: MemoryKind::Semantic,
        instance_id: [0u8; 16],
        created_ms: 0,
        embed_fingerprint: "fp-a",
    }
}

#[test]
fn store_then_recall_orders_by_similarity_and_drops_no_score_into_identity() {
    let s = SqliteMemoryStore::open_ephemeral().unwrap();
    s.store(req(NS, b"apple is a fruit", &[1.0, 0.0, 0.0]))
        .unwrap();
    s.store(req(NS, b"banana is yellow", &[0.0, 1.0, 0.0]))
        .unwrap();
    s.store(req(NS, b"cherry is red", &[0.0, 0.0, 1.0]))
        .unwrap();

    let hits = s.recall(NS, &[0.9, 0.1, 0.0], 3, "fp-a").unwrap();
    assert_eq!(hits.len(), 3);
    assert_eq!(hits[0].content, b"apple is a fruit");
    // The memory_id IS the content ref (SN-8: the citation key is content-addressed,
    // the score is display-only and never an identity input).
    assert_eq!(hits[0].memory_id, memory_id(b"apple is a fruit"));
    assert!(hits[0].score >= hits[1].score);
}

#[test]
fn store_is_content_addressed_idempotent() {
    let s = SqliteMemoryStore::open_ephemeral().unwrap();
    let first = s
        .store(req(NS, b"the deadline is march 3rd", &[1.0, 0.0, 0.0]))
        .unwrap();
    assert!(first.inserted, "first store writes a new row");
    let again = s
        .store(req(NS, b"the deadline is march 3rd", &[1.0, 0.0, 0.0]))
        .unwrap();
    assert!(!again.inserted, "the same payload dedups to one row");
    assert_eq!(first.memory_id, again.memory_id);
    // Exactly one row survives the dedup.
    assert_eq!(s.list(NS, None, 100).unwrap().len(), 1);
}

#[test]
fn namespace_isolation_is_structural() {
    let s = SqliteMemoryStore::open_ephemeral().unwrap();
    s.store(req("mem::alice", b"alice secret", &[1.0, 0.0, 0.0]))
        .unwrap();
    // Bob recalls his own (empty) namespace — Alice's memory is unreachable.
    let bob = s.recall("mem::bob", &[1.0, 0.0, 0.0], 5, "fp-a").unwrap();
    assert!(
        bob.is_empty(),
        "a namespace can never recall another's memories"
    );
    // Bob's list is empty too.
    assert!(s.list("mem::bob", None, 10).unwrap().is_empty());
}

#[test]
fn list_is_newest_first_and_filters_by_instance() {
    let s = SqliteMemoryStore::open_ephemeral().unwrap();
    let run_a = [0xAAu8; 16];
    let run_b = [0xBBu8; 16];
    let mut r = req(NS, b"first", &[1.0, 0.0, 0.0]);
    r.instance_id = run_a;
    s.store(r).unwrap();
    let mut r = req(NS, b"second", &[0.0, 1.0, 0.0]);
    r.instance_id = run_b;
    s.store(r).unwrap();
    let mut r = req(NS, b"third", &[0.0, 0.0, 1.0]);
    r.instance_id = run_a;
    s.store(r).unwrap();

    let all = s.list(NS, None, 100).unwrap();
    assert_eq!(all.len(), 3);
    assert_eq!(all[0].content, b"third", "newest first");
    assert_eq!(all[2].content, b"first");

    let only_a = s.list(NS, Some(run_a), 100).unwrap();
    assert_eq!(only_a.len(), 2, "instance filter scopes to one run");
    assert!(only_a.iter().all(|m| m.instance_id == run_a));
}

#[test]
fn forget_removes_from_recall_and_list() {
    let s = SqliteMemoryStore::open_ephemeral().unwrap();
    s.store(req(NS, b"forget me", &[1.0, 0.0, 0.0])).unwrap();
    s.store(req(NS, b"keep me", &[0.0, 1.0, 0.0])).unwrap();
    let target = memory_id(b"forget me");

    assert!(s.forget(NS, &target).unwrap(), "forget removes the row");
    assert!(
        !s.forget(NS, &target).unwrap(),
        "forget of an absent memory is false"
    );

    let recalled = s.recall(NS, &[1.0, 0.0, 0.0], 5, "fp-a").unwrap();
    assert!(
        recalled.iter().all(|h| h.memory_id != target),
        "a forgotten memory is never recalled (tombstoned)"
    );
    assert_eq!(s.list(NS, None, 100).unwrap().len(), 1);
}

#[test]
fn oversize_and_empty_content_are_rejected() {
    let s = SqliteMemoryStore::open_ephemeral().unwrap();
    let big = vec![b'x'; crate::MAX_CONTENT_LEN + 1];
    assert!(matches!(
        s.store(req(NS, &big, &[1.0, 0.0, 0.0])),
        Err(MemoryError::InvalidArgument(_))
    ));
    assert!(matches!(
        s.store(req(NS, b"", &[1.0, 0.0, 0.0])),
        Err(MemoryError::InvalidArgument(_))
    ));
}

#[test]
fn invalid_namespace_is_rejected() {
    let s = SqliteMemoryStore::open_ephemeral().unwrap();
    assert!(matches!(
        s.store(req("has space", b"x", &[1.0, 0.0, 0.0])),
        Err(MemoryError::InvalidArgument(_))
    ));
    assert!(matches!(
        s.store(req("", b"x", &[1.0, 0.0, 0.0])),
        Err(MemoryError::InvalidArgument(_))
    ));
}

#[test]
fn dim_mismatch_is_rejected() {
    let s = SqliteMemoryStore::open_ephemeral().unwrap();
    s.store(req(NS, b"three-dim", &[1.0, 0.0, 0.0])).unwrap();
    assert!(matches!(
        s.store(req(NS, b"two-dim", &[1.0, 0.0])),
        Err(MemoryError::DimMismatch(_))
    ));
    assert!(matches!(
        s.recall(NS, &[1.0, 0.0], 3, "fp-a"),
        Err(MemoryError::DimMismatch(_))
    ));
}

#[test]
fn stale_embed_fingerprint_is_rejected() {
    let s = SqliteMemoryStore::open_ephemeral().unwrap();
    s.store(req(NS, b"under model a", &[1.0, 0.0, 0.0]))
        .unwrap();
    // A store/recall with a DIFFERENT non-empty fingerprint mixes vector spaces.
    let mut r = req(NS, b"under model b", &[0.0, 1.0, 0.0]);
    r.embed_fingerprint = "fp-b";
    assert!(matches!(s.store(r), Err(MemoryError::StaleIndex(_))));
    assert!(matches!(
        s.recall(NS, &[1.0, 0.0, 0.0], 3, "fp-b"),
        Err(MemoryError::StaleIndex(_))
    ));
    // The SAME fingerprint is fine.
    assert!(s.recall(NS, &[1.0, 0.0, 0.0], 3, "fp-a").is_ok());
}

#[test]
fn non_finite_vectors_are_rejected() {
    let s = SqliteMemoryStore::open_ephemeral().unwrap();
    assert!(matches!(
        s.store(req(NS, b"x", &[f32::NAN, 0.0, 0.0])),
        Err(MemoryError::InvalidArgument(_))
    ));
    s.store(req(NS, b"ok", &[1.0, 0.0, 0.0])).unwrap();
    assert!(matches!(
        s.recall(NS, &[f32::INFINITY, 0.0, 0.0], 3, "fp-a"),
        Err(MemoryError::InvalidArgument(_))
    ));
}

#[test]
fn recall_on_empty_namespace_is_empty_not_error() {
    let s = SqliteMemoryStore::open_ephemeral().unwrap();
    assert!(s
        .recall("mem::nobody", &[1.0, 0.0, 0.0], 5, "fp-a")
        .unwrap()
        .is_empty());
}

#[test]
fn rebuild_on_open_recovers_memories_and_serves_recall() {
    let dir = tempfile::tempdir().unwrap();
    let cite: ContentRef;
    {
        let s = SqliteMemoryStore::open(dir.path()).unwrap();
        let out = s
            .store(req(NS, b"persisted fact", &[1.0, 0.0, 0.0]))
            .unwrap();
        cite = out.memory_id;
        s.store(req(NS, b"another fact", &[0.0, 1.0, 0.0])).unwrap();
    } // drop → the durable rows remain in memory.db
    let reopened = SqliteMemoryStore::open(dir.path()).unwrap();
    let hits = reopened.recall(NS, &[0.95, 0.05, 0.0], 2, "fp-a").unwrap();
    assert_eq!(hits[0].memory_id, cite, "recall works after a cold reopen");
    assert_eq!(reopened.list(NS, None, 100).unwrap().len(), 2);
}
