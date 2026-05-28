//! The P1.12 headline exit-gate test: under memory pressure a PURE payload is
//! dropped and recomputed on demand, a sibling WORLD-MUTATING payload is never
//! dropped, and correctness is unaffected by dropping the PURE payload.
//!
//! Recompute is modelled faithfully to the content-store contract: a PURE Mote's
//! payload is a deterministic function of its inputs, so re-running that logic
//! yields bit-identical bytes which content-addressing maps back to the *same*
//! `ContentRef`. (The executor's production put-then-commit recompute wiring
//! lands with the kx-runtime binary at P1.13; the property proven here — that
//! recompute restores the identical ref — is what makes that wiring sound.)

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::Fixture;
use kx_content::{ContentStore, NotFound};
use kx_mote::NdClass;
use kx_tiering::{run_pass, TieringBudget};

/// The deterministic logic of a PURE Mote. Re-running it on the same input is
/// the "recompute" the tiering contract relies on.
fn recompute_pure(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    let mut counter: u8 = 0;
    for b in input {
        out.push(b.wrapping_add(counter));
        counter = counter.wrapping_add(1);
    }
    out
}

#[test]
fn pure_payload_evicted_then_recomputed_bit_identical() {
    let fx = Fixture::new();

    let input = b"deterministic-input";
    let payload = recompute_pure(input);

    // A PURE Mote backed by the computed payload, plus a sibling WM Mote.
    let (r_pure, _) = fx.commit_payload(b'p', &payload, NdClass::Pure);
    let (r_wm, _) = fx.commit_payload(b'w', b"irreversible-effect", NdClass::WorldMutating);

    // Memory pressure drops the PURE payload.
    let report = run_pass(&fx.snapshot(), &fx.store, TieringBudget::MaxObjects(0)).unwrap();
    assert_eq!(report.evicted, vec![r_pure]);

    // The evicted ref reads as NotFound — the normal "recompute me" signal.
    assert_eq!(fx.store.get(&r_pure), Err(NotFound));
    // The WORLD-MUTATING payload was never touched.
    assert!(fx.store.contains(&r_wm), "WM payload never evicted");

    // Recompute on demand: re-run the deterministic logic and re-put.
    let recomputed = recompute_pure(input);
    let restored_ref = fx.store.put(&recomputed).unwrap();

    // Content-addressing: recompute restores the SAME ref...
    assert_eq!(
        restored_ref, r_pure,
        "recompute yields the same content ref"
    );
    // ...and a read now returns bit-identical bytes. Correctness unaffected.
    let got = fx.store.get(&r_pure).unwrap();
    assert_eq!(&got[..], &payload[..]);
}
