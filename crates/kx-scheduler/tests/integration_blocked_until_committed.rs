//! **DoD test 2** (per the P1.10 crate spec): the scheduler honors
//! "blocked until committed" rules. A child Mote whose parent has only
//! Failed (no Committed) is not dispatched; once the parent commits, the
//! child becomes ready on the next tick.
//!
//! Uses [`kx_journal::FailureReason::WorkerCrashed`] (a pre-commit-crash
//! per `is_pre_commit_crash`) so the projection treats the parent as still
//! pending (retry-allowed), then a real Committed to unblock the child.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use kx_projection::Projection;
use kx_scheduler::{LocalPlacement, Scheduler};
use smallvec::{smallvec, SmallVec};

use crate::common::{
    committed_entry, data_parent, failed_worker_crashed, fold_or_panic, permissive_warrant,
    pure_mote, MockExecutor,
};

#[test]
fn child_blocked_while_parent_only_failed_then_dispatches_on_parent_commit() {
    let mut projection = Projection::new();
    let mut scheduler = Scheduler::new(LocalPlacement);
    let executor = MockExecutor::default();
    let warrant = permissive_warrant();

    let parent = pure_mote(b"/parent", SmallVec::new());
    let child = pure_mote(b"/child", smallvec![data_parent(&parent)]);

    scheduler
        .submit(parent.clone(), warrant.clone(), &mut projection)
        .unwrap();
    scheduler
        .submit(child.clone(), warrant.clone(), &mut projection)
        .unwrap();

    // Tick 1: parent dispatches (no parents of its own). Child blocked.
    let s = scheduler.tick(&projection, &executor).unwrap();
    let ids: Vec<_> = s.dispatched.iter().map(|d| d.mote_id).collect();
    assert_eq!(
        ids,
        vec![parent.id],
        "parent must dispatch; child blocked on parent commit"
    );

    // Test harness records the parent's attempt as a non-terminal failure.
    // Per `is_pre_commit_crash`, WorkerCrashed leaves the Mote retry-eligible
    // and NOT terminal-failed; the projection's `state_of_id` returns Pending
    // (failed_pending_reattempt = true). The child remains blocked because
    // the parent is NOT Committed.
    fold_or_panic(&mut projection, &failed_worker_crashed(&parent, 1));

    // Tick 2: child still blocked — parent is not Committed. The parent is
    // also not re-dispatched because the scheduler removed it from the
    // pending map on Tick 1; only a fresh submit would put it back.
    let s = scheduler.tick(&projection, &executor).unwrap();
    assert!(
        s.dispatched.is_empty(),
        "child must remain blocked while parent is only Failed (not Committed)"
    );

    // Now record the parent's eventual commit.
    fold_or_panic(&mut projection, &committed_entry(&parent, 2));

    // Tick 3: child ready.
    let s = scheduler.tick(&projection, &executor).unwrap();
    let ids: Vec<_> = s.dispatched.iter().map(|d| d.mote_id).collect();
    assert_eq!(
        ids,
        vec![child.id],
        "child must dispatch once parent commits"
    );

    assert_eq!(executor.dispatched_ids(), vec![parent.id, child.id]);
}

#[test]
fn submitted_mote_with_unsubmitted_parent_never_dispatches() {
    // The parent is referenced by `data_parent` but never submitted to the
    // scheduler AND never registered with the projection. ready_set requires
    // the parent's state to be Committed; an unknown parent reads as not-
    // Committed; the child stays blocked forever.
    let mut projection = Projection::new();
    let mut scheduler = Scheduler::new(LocalPlacement);
    let executor = MockExecutor::default();
    let warrant = permissive_warrant();

    let phantom_parent = pure_mote(b"/phantom", SmallVec::new());
    let child = pure_mote(b"/child", smallvec![data_parent(&phantom_parent)]);

    scheduler
        .submit(child, warrant, &mut projection)
        .expect("submit child");

    // Tick: child blocked because phantom_parent isn't Committed (it isn't
    // anywhere — not in scheduler, not in projection, not in journal).
    let s = scheduler.tick(&projection, &executor).unwrap();
    assert!(
        s.dispatched.is_empty(),
        "child with an unknown parent must not dispatch"
    );
    assert_eq!(executor.call_count(), 0);
}

#[test]
fn duplicate_submit_returns_error() {
    let mut projection = Projection::new();
    let mut scheduler = Scheduler::new(LocalPlacement);
    let warrant = permissive_warrant();

    let m = pure_mote(b"/m", SmallVec::new());

    scheduler
        .submit(m.clone(), warrant.clone(), &mut projection)
        .expect("first submit ok");

    let err = scheduler
        .submit(m.clone(), warrant, &mut projection)
        .expect_err("second submit must error");

    match err {
        kx_scheduler::SchedulerError::DuplicateSubmission(id) => {
            assert_eq!(id, m.id);
        }
    }
}
