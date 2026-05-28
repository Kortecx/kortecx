//! Thesis-test witness (compile-level): `kx-coordinator` builds against the real
//! `kx-scheduler` types, hosting them verbatim. The *authoritative* proof that
//! `kx-scheduler` / `kx-executor` / `kx-inference` source is unchanged is the PR
//! diff (`git diff <merge-base> -- crates/kx-scheduler crates/kx-executor
//! crates/kx-inference` is empty) — this test just pins the dependency direction.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use kx_scheduler::{LocalPlacement, Placement, Scheduler, WorkerId};

#[test]
fn coordinator_depends_on_real_scheduler_types() {
    // The exact constructor + placement the coordinator's orchestration core uses.
    let _scheduler = Scheduler::new(LocalPlacement);
    let worker = WorkerId(0);
    // Placement is consumed as a trait (the seam P2.5 swaps).
    let placed = LocalPlacement.place(&kx_mote::MoteId::from_bytes([0u8; 32]));
    assert_eq!(placed, worker);
}
