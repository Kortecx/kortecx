//! **DoD test 3** (per the P1.10 crate spec): the placement policy is
//! behind a trait, and a second trivial impl substitutes for the first
//! without changing dispatch order or correctness.
//!
//! Same DAG; first run under [`LocalPlacement`], second run under
//! [`RoundRobinPlacement::new(3)`]. Assert worker IDs differ across the
//! two runs; assert dispatch ordering is identical.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use kx_projection::Projection;
use kx_scheduler::{LocalPlacement, Placement, RoundRobinPlacement, Scheduler, WorkerId};
use smallvec::{smallvec, SmallVec};

use crate::common::{
    committed_entry, data_parent, fold_or_panic, permissive_warrant, pure_mote, MockExecutor,
};

fn run_chain<P: Placement>(placement: P) -> (Vec<kx_mote::MoteId>, Vec<WorkerId>) {
    let mut projection = Projection::new();
    let mut scheduler = Scheduler::new(placement);
    let executor = MockExecutor::default();
    let warrant = permissive_warrant();

    let m1 = pure_mote(b"/m1", SmallVec::new());
    let m2 = pure_mote(b"/m2", smallvec![data_parent(&m1)]);
    let m3 = pure_mote(b"/m3", smallvec![data_parent(&m2)]);

    for m in [&m1, &m2, &m3] {
        scheduler
            .submit(kx_mote::Mote::clone(m), warrant.clone(), &mut projection)
            .unwrap();
    }

    let mut order = Vec::new();
    let mut workers = Vec::new();

    let s = scheduler.tick(&projection, &executor).unwrap();
    for d in s.dispatched {
        order.push(d.mote_id);
        workers.push(d.worker);
    }
    fold_or_panic(&mut projection, &committed_entry(&m1, 1));

    let s = scheduler.tick(&projection, &executor).unwrap();
    for d in s.dispatched {
        order.push(d.mote_id);
        workers.push(d.worker);
    }
    fold_or_panic(&mut projection, &committed_entry(&m2, 2));

    let s = scheduler.tick(&projection, &executor).unwrap();
    for d in s.dispatched {
        order.push(d.mote_id);
        workers.push(d.worker);
    }
    fold_or_panic(&mut projection, &committed_entry(&m3, 3));

    (order, workers)
}

#[test]
fn local_and_round_robin_produce_same_dispatch_order_with_different_workers() {
    let (local_order, local_workers) = run_chain(LocalPlacement);
    let (rr_order, rr_workers) = run_chain(RoundRobinPlacement::new(3));

    // Same MoteIds in the same order under both policies — placement
    // does not change dispatch sequence (ordering is the projection's job).
    assert_eq!(
        local_order, rr_order,
        "dispatch order must be identical across placements; placement does not change ordering"
    );

    // LocalPlacement → every dispatched Mote went to worker 0.
    assert!(
        local_workers.iter().all(|w| *w == WorkerId(0)),
        "LocalPlacement must route every Mote to WorkerId(0); got: {local_workers:?}"
    );

    // RoundRobinPlacement(3) → workers cycle 0, 1, 2 across the 3 dispatches.
    assert_eq!(
        rr_workers,
        vec![WorkerId(0), WorkerId(1), WorkerId(2)],
        "RoundRobinPlacement(3) must cycle workers in dispatch order"
    );

    // The two impls produced DIFFERENT worker sequences — proves the
    // trait is actually substitutable and not a hardcoded behavior.
    assert_ne!(local_workers, rr_workers);
}
