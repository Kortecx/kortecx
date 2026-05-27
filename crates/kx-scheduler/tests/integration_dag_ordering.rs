//! **DoD test 1** (per the P1.10 crate spec): the scheduler resolves a
//! multi-Mote DAG in correct order from the projection alone.
//!
//! Linear chain M1 → M2 → M3 (Data edges). Submit all three; tick once
//! per Mote; assert each tick dispatches only the next-ready Mote and
//! that no out-of-order dispatch occurs.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use kx_mote::Mote;
use kx_projection::Projection;
use kx_scheduler::{LocalPlacement, Scheduler};
use smallvec::{smallvec, SmallVec};

use crate::common::{
    committed_entry, data_parent, fold_or_panic, permissive_warrant, pure_mote, MockExecutor,
};

#[test]
fn linear_chain_dispatches_in_parent_then_child_order() {
    let mut projection = Projection::new();
    let mut scheduler = Scheduler::new(LocalPlacement);
    let executor = MockExecutor::default();
    let warrant = permissive_warrant();

    let m1 = pure_mote(b"/m1", SmallVec::new());
    let m2 = pure_mote(b"/m2", smallvec![data_parent(&m1)]);
    let m3 = pure_mote(b"/m3", smallvec![data_parent(&m2)]);

    scheduler
        .submit(m1.clone(), warrant.clone(), &mut projection)
        .expect("submit m1");
    scheduler
        .submit(m2.clone(), warrant.clone(), &mut projection)
        .expect("submit m2");
    scheduler
        .submit(m3.clone(), warrant.clone(), &mut projection)
        .expect("submit m3");

    assert_eq!(scheduler.pending_count(), 3);

    // Tick 1: only M1 is ready (no parents). M2 and M3 wait for M1's commit.
    let summary = scheduler.tick(&projection, &executor).expect("tick 1");
    let dispatched: Vec<_> = summary.dispatched.iter().map(|d| d.mote_id).collect();
    assert_eq!(
        dispatched,
        vec![m1.id],
        "tick 1 must dispatch only M1; M2 and M3 are blocked on M1's commit"
    );
    assert_eq!(scheduler.pending_count(), 2);

    // Test harness folds M1's Committed (in production the executor's
    // lifecycle layer would do this).
    fold_or_panic(&mut projection, &committed_entry(&m1, 1));

    // Tick 2: M2 now ready.
    let summary = scheduler.tick(&projection, &executor).expect("tick 2");
    let dispatched: Vec<_> = summary.dispatched.iter().map(|d| d.mote_id).collect();
    assert_eq!(
        dispatched,
        vec![m2.id],
        "tick 2 must dispatch only M2; M3 still blocked on M2's commit"
    );
    assert_eq!(scheduler.pending_count(), 1);

    fold_or_panic(&mut projection, &committed_entry(&m2, 2));

    // Tick 3: M3 ready.
    let summary = scheduler.tick(&projection, &executor).expect("tick 3");
    let dispatched: Vec<_> = summary.dispatched.iter().map(|d| d.mote_id).collect();
    assert_eq!(dispatched, vec![m3.id], "tick 3 must dispatch M3");
    assert_eq!(scheduler.pending_count(), 0);

    fold_or_panic(&mut projection, &committed_entry(&m3, 3));

    // Tick 4: no work left.
    let summary = scheduler.tick(&projection, &executor).expect("tick 4");
    assert!(
        summary.dispatched.is_empty(),
        "tick 4 must be a no-op; pending map is empty"
    );

    // Total dispatch order observed by the executor:
    assert_eq!(executor.dispatched_ids(), vec![m1.id, m2.id, m3.id]);
}

#[test]
fn two_root_motes_both_dispatch_on_first_tick() {
    let mut projection = Projection::new();
    let mut scheduler = Scheduler::new(LocalPlacement);
    let executor = MockExecutor::default();
    let warrant = permissive_warrant();

    let m1 = pure_mote(b"/root-a", SmallVec::new());
    let m2 = pure_mote(b"/root-b", SmallVec::new());

    scheduler
        .submit(m1.clone(), warrant.clone(), &mut projection)
        .expect("submit m1");
    scheduler
        .submit(m2.clone(), warrant.clone(), &mut projection)
        .expect("submit m2");

    let summary = scheduler.tick(&projection, &executor).expect("tick");
    let dispatched: std::collections::BTreeSet<_> =
        summary.dispatched.iter().map(|d| d.mote_id).collect();
    let expected: std::collections::BTreeSet<_> = [m1.id, m2.id].into_iter().collect();
    assert_eq!(
        dispatched, expected,
        "both root Motes (no parents) must dispatch on the first tick"
    );
}

#[test]
fn diamond_dag_dispatches_in_correct_order() {
    //   M1  (root)
    //   / \
    // M2   M3
    //   \ /
    //   M4
    let mut projection = Projection::new();
    let mut scheduler = Scheduler::new(LocalPlacement);
    let executor = MockExecutor::default();
    let warrant = permissive_warrant();

    let m1 = pure_mote(b"/m1", SmallVec::new());
    let m2 = pure_mote(b"/m2", smallvec![data_parent(&m1)]);
    let m3 = pure_mote(b"/m3", smallvec![data_parent(&m1)]);
    let m4 = pure_mote(b"/m4", smallvec![data_parent(&m2), data_parent(&m3)]);

    for m in [&m1, &m2, &m3, &m4] {
        scheduler
            .submit(Mote::clone(m), warrant.clone(), &mut projection)
            .unwrap();
    }

    // Tick 1: only M1.
    let s1 = scheduler.tick(&projection, &executor).unwrap();
    assert_eq!(
        s1.dispatched.iter().map(|d| d.mote_id).collect::<Vec<_>>(),
        vec![m1.id]
    );

    fold_or_panic(&mut projection, &committed_entry(&m1, 1));

    // Tick 2: M2 and M3 both ready (siblings under M1).
    let s2 = scheduler.tick(&projection, &executor).unwrap();
    let s2_ids: std::collections::BTreeSet<_> = s2.dispatched.iter().map(|d| d.mote_id).collect();
    assert_eq!(
        s2_ids,
        [m2.id, m3.id].into_iter().collect(),
        "M2 and M3 must both dispatch — they share M1 as parent and have no other deps"
    );

    fold_or_panic(&mut projection, &committed_entry(&m2, 2));
    // Tick 3 with only M2 committed: M4 still blocked on M3.
    let s3 = scheduler.tick(&projection, &executor).unwrap();
    assert!(s3.dispatched.is_empty(), "M4 still blocked on M3");

    fold_or_panic(&mut projection, &committed_entry(&m3, 3));
    // Tick 4: M4 now ready.
    let s4 = scheduler.tick(&projection, &executor).unwrap();
    assert_eq!(
        s4.dispatched.iter().map(|d| d.mote_id).collect::<Vec<_>>(),
        vec![m4.id]
    );
}

#[test]
fn empty_tick_returns_empty_summary() {
    let projection = Projection::new();
    let mut scheduler = Scheduler::new(LocalPlacement);
    let executor = MockExecutor::default();

    let summary = scheduler.tick(&projection, &executor).expect("tick");
    assert!(summary.dispatched.is_empty());
    assert_eq!(executor.call_count(), 0);
}
