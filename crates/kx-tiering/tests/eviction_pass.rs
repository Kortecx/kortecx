//! Integration: the budget-driven eviction pass — oldest-first eviction,
//! protected-tag invariant under maximal pressure, and idempotence.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::Fixture;
use kx_content::ContentStore;
use kx_mote::NdClass;
use kx_tiering::{run_pass, TieringBudget};

#[test]
fn max_objects_evicts_oldest_pure_first_until_under_budget() {
    let fx = Fixture::new();
    let (r0, _) = fx.commit_payload(b'a', b"p0", NdClass::Pure); // seq 0 (oldest)
    let (r1, _) = fx.commit_payload(b'b', b"p1", NdClass::Pure); // seq 1
    let (r2, _) = fx.commit_payload(b'c', b"p2", NdClass::Pure); // seq 2 (youngest)

    let report = run_pass(&fx.snapshot(), &fx.store, TieringBudget::MaxObjects(1)).unwrap();

    assert_eq!(report.candidates_considered, 3);
    assert_eq!(report.usage_before.objects, 3);
    assert_eq!(report.usage_after.objects, 1);
    // Oldest two evicted; youngest retained.
    assert_eq!(report.evicted, vec![r0, r1]);
    assert!(!fx.store.contains(&r0));
    assert!(!fx.store.contains(&r1));
    assert!(fx.store.contains(&r2));
}

#[test]
fn max_bytes_evicts_until_footprint_within_budget() {
    let fx = Fixture::new();
    // Three 4-byte PURE payloads = 12 bytes resident.
    let (r0, _) = fx.commit_payload(b'a', b"aaaa", NdClass::Pure);
    let (r1, _) = fx.commit_payload(b'b', b"bbbb", NdClass::Pure);
    let (_r2, _) = fx.commit_payload(b'c', b"cccc", NdClass::Pure);

    // Budget 4 bytes => must drop two oldest (12 -> 4).
    let report = run_pass(&fx.snapshot(), &fx.store, TieringBudget::MaxBytes(4)).unwrap();

    assert_eq!(report.usage_before.bytes, 12);
    assert_eq!(report.bytes_reclaimed, 8);
    assert_eq!(report.usage_after.bytes, 4);
    assert_eq!(report.evicted, vec![r0, r1]);
}

#[test]
fn protected_tags_never_evicted_under_maximal_pressure() {
    let fx = Fixture::new();
    let (r_pure, _) = fx.commit_payload(b'p', b"pure", NdClass::Pure);
    let (r_wm, _) = fx.commit_payload(b'w', b"world", NdClass::WorldMutating);
    let (r_rond, _) = fx.commit_payload(b'r', b"rng", NdClass::ReadOnlyNondet);

    // Zero-byte budget = maximal pressure: evict every PURE payload.
    let report = run_pass(&fx.snapshot(), &fx.store, TieringBudget::MaxBytes(0)).unwrap();

    assert_eq!(report.evicted, vec![r_pure]);
    assert!(!fx.store.contains(&r_pure), "PURE dropped");
    // The protected tags are untouched no matter how tight the budget.
    assert!(fx.store.contains(&r_wm), "WORLD-MUTATING never evicted");
    assert!(fx.store.contains(&r_rond), "READ-ONLY-NONDET never evicted");
}

#[test]
fn pass_is_idempotent() {
    let fx = Fixture::new();
    fx.commit_payload(b'a', b"p0", NdClass::Pure);
    fx.commit_payload(b'b', b"p1", NdClass::Pure);

    let snap = fx.snapshot();
    let first = run_pass(&snap, &fx.store, TieringBudget::MaxObjects(0)).unwrap();
    assert_eq!(first.evicted.len(), 2);
    assert_eq!(first.skipped_absent, 0);

    // Re-running over the same snapshot finds the candidates already absent.
    let second = run_pass(&snap, &fx.store, TieringBudget::MaxObjects(0)).unwrap();
    assert!(second.evicted.is_empty());
    assert_eq!(second.candidates_considered, 2);
    assert_eq!(second.skipped_absent, 2);
    assert_eq!(second.usage_before.objects, 0);
}
