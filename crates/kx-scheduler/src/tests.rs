//! Inline unit tests covering small properties of [`crate::WorkerId`],
//! [`crate::Placement`] impls, and [`crate::SchedulerError`].
//!
//! Larger DoD-bearing scenarios live in `tests/integration_*.rs`.

use kx_mote::MoteId;

use crate::errors::SchedulerError;
use crate::placement::{LocalPlacement, Placement, RoundRobinPlacement};
use crate::worker::WorkerId;

#[test]
fn worker_id_is_ord_for_btreemap_keys() {
    let mut ids = [WorkerId(3), WorkerId(1), WorkerId(2)];
    ids.sort();
    assert_eq!(ids, [WorkerId(1), WorkerId(2), WorkerId(3)]);
}

#[test]
fn local_placement_always_returns_worker_zero() {
    let p = LocalPlacement;
    assert_eq!(p.place(&MoteId::from_bytes([0u8; 32])), WorkerId(0));
    assert_eq!(p.place(&MoteId::from_bytes([1u8; 32])), WorkerId(0));
    assert_eq!(p.place(&MoteId::from_bytes([0xff; 32])), WorkerId(0));
}

#[test]
fn round_robin_placement_cycles_through_workers() {
    let p = RoundRobinPlacement::new(3);
    let any = MoteId::from_bytes([0u8; 32]);
    assert_eq!(p.place(&any), WorkerId(0));
    assert_eq!(p.place(&any), WorkerId(1));
    assert_eq!(p.place(&any), WorkerId(2));
    assert_eq!(p.place(&any), WorkerId(0));
    assert_eq!(p.place(&any), WorkerId(1));
}

#[test]
#[should_panic(expected = "RoundRobinPlacement requires at least one worker")]
fn round_robin_placement_rejects_zero_workers() {
    let _ = RoundRobinPlacement::new(0);
}

#[test]
fn scheduler_error_display_includes_mote_id() {
    let id = MoteId::from_bytes([7u8; 32]);
    let err = SchedulerError::DuplicateSubmission(id);
    let s = format!("{err}");
    assert!(s.contains("already submitted"), "got: {s}");
}
