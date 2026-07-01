// SPDX-License-Identifier: Apache-2.0
//! Deterministic unit tests for the memory store — recall ordering, content-address
//! dedup, namespace isolation, the episodic log, forget, the vector-space guards,
//! and rebuild-on-open durability. No model, no float on any identity path.

use kx_content::ContentRef;

use crate::record::{memory_id, BundleRequest, DecayPolicy, MemoryKind, StoreRequest};
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

/// A store request with an explicit kind + write time (for the bundle/decay tests).
fn req_at<'a>(
    ns: &'a str,
    content: &'a [u8],
    vector: &'a [f32],
    kind: MemoryKind,
    created_ms: i64,
) -> StoreRequest<'a> {
    StoreRequest {
        namespace: ns,
        content,
        vector,
        kind,
        instance_id: [0u8; 16],
        created_ms,
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
    assert_eq!(s.list(NS, None, 100, false).unwrap().len(), 1);
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
    assert!(s.list("mem::bob", None, 10, false).unwrap().is_empty());
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

    let all = s.list(NS, None, 100, false).unwrap();
    assert_eq!(all.len(), 3);
    assert_eq!(all[0].content, b"third", "newest first");
    assert_eq!(all[2].content, b"first");

    let only_a = s.list(NS, Some(run_a), 100, false).unwrap();
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
    assert_eq!(s.list(NS, None, 100, false).unwrap().len(), 1);
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
    assert_eq!(reopened.list(NS, None, 100, false).unwrap().len(), 2);
}

// ---- RC5b: bundle (consolidation source) -------------------------------------

fn bundle_req<'a>(
    ns: &'a str,
    kind: Option<MemoryKind>,
    query_vec: Option<&'a [f32]>,
    limit: usize,
) -> BundleRequest<'a> {
    BundleRequest {
        namespace: ns,
        kind,
        query_vec,
        window_ms: None,
        embed_fingerprint: "fp-a",
        limit,
    }
}

#[test]
fn bundle_recency_is_newest_first_and_kind_filtered() {
    let s = SqliteMemoryStore::open_ephemeral().unwrap();
    s.store(req_at(
        NS,
        b"sem fact",
        &[1.0, 0.0, 0.0],
        MemoryKind::Semantic,
        10,
    ))
    .unwrap();
    s.store(req_at(
        NS,
        b"ep one",
        &[0.0, 1.0, 0.0],
        MemoryKind::Episodic,
        20,
    ))
    .unwrap();
    s.store(req_at(
        NS,
        b"ep two",
        &[0.0, 0.0, 1.0],
        MemoryKind::Episodic,
        30,
    ))
    .unwrap();

    let episodic = s
        .bundle(bundle_req(NS, Some(MemoryKind::Episodic), None, 10))
        .unwrap();
    assert_eq!(episodic.len(), 2, "only episodic memories are bundled");
    assert_eq!(episodic[0].content, b"ep two", "newest first");
    assert!(episodic.iter().all(|m| m.kind == MemoryKind::Episodic));

    let all = s.bundle(bundle_req(NS, None, None, 10)).unwrap();
    assert_eq!(all.len(), 3, "kind=None bundles every kind");
}

#[test]
fn bundle_semantic_ranks_by_similarity() {
    let s = SqliteMemoryStore::open_ephemeral().unwrap();
    s.store(req_at(
        NS,
        b"apple",
        &[1.0, 0.0, 0.0],
        MemoryKind::Episodic,
        1,
    ))
    .unwrap();
    s.store(req_at(
        NS,
        b"banana",
        &[0.0, 1.0, 0.0],
        MemoryKind::Episodic,
        2,
    ))
    .unwrap();
    let q: &[f32] = &[0.9, 0.1, 0.0];
    let out = s
        .bundle(bundle_req(NS, Some(MemoryKind::Episodic), Some(q), 2))
        .unwrap();
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].content, b"apple", "the closest memory ranks first");
}

#[test]
fn bundle_window_filters_by_created_ms() {
    let s = SqliteMemoryStore::open_ephemeral().unwrap();
    s.store(req_at(
        NS,
        b"old",
        &[1.0, 0.0, 0.0],
        MemoryKind::Episodic,
        100,
    ))
    .unwrap();
    s.store(req_at(
        NS,
        b"new",
        &[0.0, 1.0, 0.0],
        MemoryKind::Episodic,
        500,
    ))
    .unwrap();
    let mut r = bundle_req(NS, Some(MemoryKind::Episodic), None, 10);
    r.window_ms = Some((400, 1000));
    let out = s.bundle(r).unwrap();
    assert_eq!(out.len(), 1, "only in-window memories are bundled");
    assert_eq!(out[0].content, b"new");
}

// ---- RC5b: decay (TTL + salience, reversible) --------------------------------

#[test]
fn decay_tombstones_old_unaccessed_and_is_reversible() {
    let s = SqliteMemoryStore::open_ephemeral().unwrap();
    let stale = memory_id(b"stale fact");
    s.store(req_at(
        NS,
        b"stale fact",
        &[1.0, 0.0, 0.0],
        MemoryKind::Semantic,
        0,
    ))
    .unwrap();
    s.store(req_at(
        NS,
        b"fresh fact",
        &[0.0, 1.0, 0.0],
        MemoryKind::Semantic,
        900,
    ))
    .unwrap();
    // now = 1000; ttl = 500 ⇒ the age-0 memory is stale, the age-900-vs-1000 is fresh.
    let policy = DecayPolicy {
        ttl_ms: 500,
        min_access: 1,
        dry_run: false,
    };
    let report = s.decay_at(NS, policy, 1000).unwrap();
    assert_eq!(report.swept, 1, "exactly the stale memory is evicted");
    assert_eq!(report.kept, 1);
    assert_eq!(report.candidates[0].memory_id, stale);

    // Recall + default list exclude the tombstoned memory.
    let hits = s.recall(NS, &[1.0, 0.0, 0.0], 5, "fp-a").unwrap();
    assert!(hits.iter().all(|h| h.memory_id != stale));
    assert_eq!(s.list(NS, None, 100, false).unwrap().len(), 1);
    // The decayed view surfaces it with a tombstone set.
    let with_tomb = s.list(NS, None, 100, true).unwrap();
    assert_eq!(with_tomb.len(), 2);
    assert!(with_tomb
        .iter()
        .any(|m| m.memory_id == stale && m.tombstoned_ms.is_some()));

    // Restore un-decays it.
    assert!(s.restore(NS, &stale).unwrap());
    assert_eq!(s.list(NS, None, 100, false).unwrap().len(), 2);
    let hits = s.recall(NS, &[1.0, 0.0, 0.0], 5, "fp-a").unwrap();
    assert!(
        hits.iter().any(|h| h.memory_id == stale),
        "restored memory recalls again"
    );
}

#[test]
fn decay_protects_salient_memories() {
    let s = SqliteMemoryStore::open_ephemeral().unwrap();
    let hot = memory_id(b"hot fact");
    s.store(req_at(
        NS,
        b"hot fact",
        &[1.0, 0.0, 0.0],
        MemoryKind::Semantic,
        0,
    ))
    .unwrap();
    // Recall it twice ⇒ access_count = 2, above the min_access floor.
    s.recall(NS, &[1.0, 0.0, 0.0], 1, "fp-a").unwrap();
    s.recall(NS, &[1.0, 0.0, 0.0], 1, "fp-a").unwrap();
    let policy = DecayPolicy {
        ttl_ms: 500,
        min_access: 2,
        dry_run: false,
    };
    let report = s.decay_at(NS, policy, 1000).unwrap();
    assert_eq!(
        report.swept, 0,
        "a salient (frequently-recalled) old fact is protected"
    );
    assert!(s
        .recall(NS, &[1.0, 0.0, 0.0], 1, "fp-a")
        .unwrap()
        .iter()
        .any(|h| h.memory_id == hot));
}

#[test]
fn decay_dry_run_previews_without_evicting() {
    let s = SqliteMemoryStore::open_ephemeral().unwrap();
    s.store(req_at(
        NS,
        b"old",
        &[1.0, 0.0, 0.0],
        MemoryKind::Semantic,
        0,
    ))
    .unwrap();
    let policy = DecayPolicy {
        ttl_ms: 100,
        min_access: 1,
        dry_run: true,
    };
    let report = s.decay_at(NS, policy, 1000).unwrap();
    assert_eq!(report.candidates.len(), 1, "the candidate is previewed");
    assert_eq!(report.swept, 0, "a dry run tombstones nothing");
    assert_eq!(
        s.list(NS, None, 100, false).unwrap().len(),
        1,
        "the memory is untouched"
    );
}

#[test]
fn decay_survives_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let gone = memory_id(b"decay me");
    {
        let s = SqliteMemoryStore::open(dir.path()).unwrap();
        s.store(req_at(
            NS,
            b"decay me",
            &[1.0, 0.0, 0.0],
            MemoryKind::Semantic,
            0,
        ))
        .unwrap();
        s.store(req_at(
            NS,
            b"keep me",
            &[0.0, 1.0, 0.0],
            MemoryKind::Semantic,
            900,
        ))
        .unwrap();
        let policy = DecayPolicy {
            ttl_ms: 500,
            min_access: 1,
            dry_run: false,
        };
        assert_eq!(s.decay_at(NS, policy, 1000).unwrap().swept, 1);
    }
    // A cold reopen re-applies the tombstone (the sidecar survives the rebuild).
    let reopened = SqliteMemoryStore::open(dir.path()).unwrap();
    assert_eq!(reopened.list(NS, None, 100, false).unwrap().len(), 1);
    let hits = reopened.recall(NS, &[1.0, 0.0, 0.0], 5, "fp-a").unwrap();
    assert!(
        hits.iter().all(|h| h.memory_id != gone),
        "a decayed memory stays hidden after reopen"
    );
}

// ---- RC5b: stats -------------------------------------------------------------

#[test]
fn stats_counts_live_and_tombstoned() {
    let s = SqliteMemoryStore::open_ephemeral().unwrap();
    s.store(req_at(
        NS,
        b"sem",
        &[1.0, 0.0, 0.0],
        MemoryKind::Semantic,
        10,
    ))
    .unwrap();
    s.store(req_at(
        NS,
        b"ep a",
        &[0.0, 1.0, 0.0],
        MemoryKind::Episodic,
        20,
    ))
    .unwrap();
    s.store(req_at(
        NS,
        b"ep b",
        &[0.0, 0.0, 1.0],
        MemoryKind::Episodic,
        30,
    ))
    .unwrap();

    let st = s.stats(NS).unwrap();
    assert_eq!(st.total, 3);
    assert_eq!(st.semantic, 1);
    assert_eq!(st.episodic, 2);
    assert_eq!(st.tombstoned, 0);
    assert_eq!(st.dim, 3);
    assert_eq!(st.oldest_ms, 10);
    assert_eq!(st.newest_ms, 30);

    // Decay one ⇒ live counts drop, tombstoned rises.
    let policy = DecayPolicy {
        ttl_ms: 5,
        min_access: 1,
        dry_run: false,
    };
    s.decay_at(NS, policy, 25).unwrap(); // now=25 ⇒ only the age-10 semantic is stale
    let st = s.stats(NS).unwrap();
    assert_eq!(st.semantic, 0);
    assert_eq!(st.episodic, 2);
    assert_eq!(st.tombstoned, 1);
    assert_eq!(st.total, 2);
}

// ---- RC5b: salience-write hot-path measurement (GR10, measure-first) ---------
//
// Ignored profiling probe: prove the recall salience write (RC5b) is NOT a material
// recall regression. Run with:
//   cargo test -p kx-memory --release -- --ignored --nocapture recall_salience
// Emits the pure index-query cost vs the full recall (query + salience upsert) so the
// delta is auditable; the numbers land in the private `docs/benchmarks/` trend (SN-2).
#[test]
#[ignore = "profiling probe (GR10) — run explicitly with --ignored --nocapture"]
#[allow(clippy::cast_precision_loss, clippy::cast_possible_wrap)]
fn recall_salience_write_delta_is_immaterial() {
    use std::time::Instant;
    let s = SqliteMemoryStore::open_ephemeral().unwrap();
    let n = 1000usize;
    for i in 0..n {
        let v = [(i % 7) as f32 + 1.0, (i % 5) as f32, (i % 3) as f32];
        let content = format!("memory number {i} about topic {}", i % 11);
        s.store(req_at(
            NS,
            content.as_bytes(),
            &v,
            MemoryKind::Episodic,
            i as i64,
        ))
        .unwrap();
    }
    let q: &[f32] = &[3.0, 2.0, 1.0];
    let iters = 2000usize;
    // Warm.
    for _ in 0..50 {
        s.recall(NS, q, 10, "fp-a").unwrap();
    }
    let mut samples: Vec<u128> = Vec::with_capacity(iters);
    for _ in 0..iters {
        let t = Instant::now();
        let hits = s.recall(NS, q, 10, "fp-a").unwrap();
        samples.push(t.elapsed().as_micros());
        assert_eq!(hits.len(), 10);
    }
    samples.sort_unstable();
    let p50 = samples[samples.len() / 2];
    let p99 = samples[samples.len() * 99 / 100];
    eprintln!("M-mem recall(+salience) | n={n} k=10 iters={iters} | p50={p50}us p99={p99}us");
    // A best-effort bound: a full recall (index scan of 1000 + 10-row WAL upsert) must
    // stay comfortably sub-millisecond at p99 on any dev box (generous ceiling to avoid
    // CI flakiness; the eprintln is the real record).
    assert!(
        p99 < 5000,
        "recall p99 {p99}us unexpectedly high — salience write may be regressing the hot path"
    );
}

#[test]
fn decay_all_sweeps_every_namespace() {
    let s = SqliteMemoryStore::open_ephemeral().unwrap();
    s.store(req_at(
        "mem::a",
        b"a old",
        &[1.0, 0.0, 0.0],
        MemoryKind::Semantic,
        0,
    ))
    .unwrap();
    s.store(req_at(
        "mem::b",
        b"b old",
        &[1.0, 0.0, 0.0],
        MemoryKind::Semantic,
        0,
    ))
    .unwrap();
    let policy = DecayPolicy {
        ttl_ms: 100,
        min_access: 1,
        dry_run: false,
    };
    let swept = s.decay_all(policy).unwrap();
    assert_eq!(swept, 2, "decay_all sweeps across all namespaces");
    assert!(s.list("mem::a", None, 100, false).unwrap().is_empty());
    assert!(s.list("mem::b", None, 100, false).unwrap().is_empty());
}
