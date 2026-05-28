//! Integration: candidate selection over a realistic mixed projection
//! (PURE / READ-ONLY-NONDET / WORLD-MUTATING / repudiated + a dedup collision),
//! built by folding a real in-memory journal.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::Fixture;
use kx_mote::NdClass;
use kx_tiering::select_candidates;

#[test]
fn selects_exactly_pure_only_refs_in_oldest_seq_order() {
    let fx = Fixture::new();

    // seq 0: PURE, unique payload  -> evictable
    let (r_pure_a, _) = fx.commit_payload(b'a', b"alpha", NdClass::Pure);
    // seq 1: READ-ONLY-NONDET      -> protected
    let (_r_rond, _) = fx.commit_payload(b'r', b"sampled-output", NdClass::ReadOnlyNondet);
    // seq 2: WORLD-MUTATING        -> protected
    let (_r_wm, _) = fx.commit_payload(b'w', b"world-effect", NdClass::WorldMutating);
    // seq 3: PURE, unique payload  -> evictable (younger than r_pure_a)
    let (r_pure_b, _) = fx.commit_payload(b'b', b"bravo", NdClass::Pure);

    // seq 4 + 5: dedup collision — two PURE Motes, identical bytes => one ref.
    let (r_shared_pure, _) = fx.commit_payload(b'c', b"shared", NdClass::Pure);
    let (r_shared_pure2, _) = fx.commit_payload(b'd', b"shared", NdClass::Pure);
    assert_eq!(r_shared_pure, r_shared_pure2, "content-addressed dedup");

    // seq 6 + 7: dedup collision — PURE + WM share a ref => protected.
    let (r_mixed, _) = fx.commit_payload(b'e', b"mixed", NdClass::Pure);
    let (r_mixed2, _) = fx.commit_payload(b'f', b"mixed", NdClass::WorldMutating);
    assert_eq!(r_mixed, r_mixed2);

    // seq 8: PURE then repudiated  -> no live contributor, offers nothing.
    let (_r_dead, dead_seq) = fx.commit_payload(b'g', b"dead", NdClass::Pure);
    fx.repudiate(b'g', dead_seq);

    let candidates = select_candidates(&fx.snapshot());
    let got: Vec<_> = candidates.iter().map(|c| c.result_ref).collect();

    // Exactly the three evictable PURE-only refs: a, b, and the all-PURE shared ref.
    assert_eq!(got, vec![r_pure_a, r_pure_b, r_shared_pure]);
    // Oldest-seq-first (journal seq is 1-based, append order): a (seq 1) before
    // b (seq 4) before the all-PURE shared ref (min of seq 5 and 6 = 5).
    let seqs: Vec<u64> = candidates.iter().map(|c| c.min_seq).collect();
    assert_eq!(seqs, vec![1, 4, 5]);
}
