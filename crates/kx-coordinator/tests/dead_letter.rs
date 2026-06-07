//! F4 worker dead-letter (the distributed `WorkerCrashed`-budget analog). A LIVE worker
//! that cannot complete a Mote (a deterministic execution failure, or a transient one that
//! exhausts the retry budget) reports a TERMINAL failure; the coordinator (sole writer,
//! D40) appends a `Failed{DeadLettered}`, the Mote leaves `ready_set`, and it is never
//! re-leased — closing the live-worker spin (PR-9b F2 / the deferred F4 analog).
//!
//! Asserted *via the service* (the projection is a pure fold of the log, so a `Failed`
//! state proves a `Failed` journal entry exists — no off-journal facts). Distinct from
//! `reschedule.rs` (coordinator-OBSERVED worker death → `WorkerCrashed`, re-leasable);
//! here the live worker reports its OWN terminal verdict → terminal `Failed`, gone for good.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use kx_coordinator::proto::{CommitOutcome, ExecutorClass, FailureReason};
use kx_coordinator::{CoordinatorService, MoteState};
use kx_journal::InMemoryJournal;
use kx_mote::NdClass;

const MAC: ExecutorClass = ExecutorClass::MacosSandbox;

fn coordinator() -> CoordinatorService {
    CoordinatorService::new(InMemoryJournal::new())
}

/// The flagship: a leased Mote a worker reports as terminally failed becomes terminal
/// `Failed`, leaves `ready_set`, and is NOT handed back on the next lease (no spin).
#[tokio::test]
async fn reported_terminal_failure_dead_letters_and_is_not_re_leased() {
    let svc = coordinator();
    let warrant = common::sample_warrant();
    let worker = common::register(&svc, "w").await;
    let m = common::mote(7, NdClass::Pure, &[]);
    common::submit(&svc, &m, &warrant).await;

    // The worker leases the Mote (the coordinator records the lease) and then reports it
    // as terminally failed (e.g. its body always non-zero-exits / a malformed proposal).
    let leased = common::lease_work(&svc, worker, MAC, 16).await;
    assert_eq!(leased.len(), 1, "the ready Mote was leased");

    let resp = common::report_failure(&svc, &m, worker, FailureReason::DeadLettered)
        .await
        .expect("a leased Mote may be dead-lettered");
    assert!(resp.ack, "the dead-letter is acked once durable");

    // The death is a journal fact: the projection reports the Mote terminal `Failed`.
    assert_eq!(
        svc.state_of(m.id).await.unwrap(),
        MoteState::Failed,
        "the worker-reported terminal failure is a durable Failed fact"
    );

    // The spin-prevention witness: a subsequent lease returns NOTHING for this Mote — a
    // terminal `Failed` leaves `ready_set`, so the worker never re-leases it forever.
    let again = common::lease_work(&svc, worker, MAC, 16).await;
    assert!(
        again.is_empty(),
        "a dead-lettered Mote is never re-leased (the F4 spin is closed)"
    );
}

/// Idempotent: a duplicate dead-letter report is a no-op (acked, no second `Failed`).
#[tokio::test]
async fn report_failure_is_idempotent() {
    let svc = coordinator();
    let warrant = common::sample_warrant();
    let worker = common::register(&svc, "w").await;
    let m = common::mote(9, NdClass::Pure, &[]);
    common::submit(&svc, &m, &warrant).await;
    common::lease_work(&svc, worker, MAC, 16).await;

    let first = common::report_failure(&svc, &m, worker, FailureReason::DeadLettered)
        .await
        .unwrap();
    assert!(first.ack);
    assert!(first.failed_seq > 0, "the first report appends a Failed");

    // A second report (e.g. a retry after a dropped ack) is a no-op: the Mote is already
    // terminal, so nothing new is appended and the call still acks.
    let second = common::report_failure(&svc, &m, worker, FailureReason::DeadLettered)
        .await
        .unwrap();
    assert!(second.ack, "a duplicate report still acks");
    assert_eq!(
        svc.state_of(m.id).await.unwrap(),
        MoteState::Failed,
        "the Mote stays terminal Failed (no double-write of state)"
    );
}

/// Admission: only the worker that HOLDS the lease may dead-letter the Mote — a worker
/// cannot terminate work it does not hold (or a phantom Mote).
#[tokio::test]
async fn report_failure_requires_the_lease() {
    let svc = coordinator();
    let warrant = common::sample_warrant();
    let holder = common::register(&svc, "holder").await;
    let other = common::register(&svc, "other").await;
    let m = common::mote(11, NdClass::Pure, &[]);
    common::submit(&svc, &m, &warrant).await;
    common::lease_work(&svc, holder, MAC, 16).await;

    // A different (registered) worker that does not hold the lease is refused.
    let err = common::report_failure(&svc, &m, other, FailureReason::DeadLettered)
        .await
        .expect_err("a non-holder cannot dead-letter the Mote");
    assert_eq!(
        err.code(),
        tonic::Code::FailedPrecondition,
        "the admission gate refuses a non-holder"
    );
    // The Mote is untouched — still pending, still leasable by the real holder.
    assert_ne!(
        svc.state_of(m.id).await.unwrap(),
        MoteState::Failed,
        "a refused report writes no Failed fact"
    );
}

/// Committed wins: a Mote that raced to `Committed` is never dead-lettered — a late
/// failure report is a no-op and the commit stands.
#[tokio::test]
async fn committed_mote_is_never_dead_lettered() {
    let svc = coordinator();
    let warrant = common::sample_warrant();
    let worker = common::register(&svc, "w").await;
    let m = common::mote(13, NdClass::Pure, &[]);
    common::submit(&svc, &m, &warrant).await;
    common::lease_work(&svc, worker, MAC, 16).await;

    // The worker commits first.
    let out = common::commit(&svc, &m, worker).await;
    assert_eq!(out.outcome, CommitOutcome::Committed as i32);

    // A late failure report (e.g. a duplicate attempt that lost the race) is a no-op.
    let resp = common::report_failure(&svc, &m, worker, FailureReason::DeadLettered)
        .await
        .expect("a late report on a committed Mote is a benign no-op");
    assert!(resp.ack);
    assert_eq!(
        svc.state_of(m.id).await.unwrap(),
        MoteState::Committed,
        "Committed wins — the late dead-letter never un-commits the Mote"
    );
}

/// Fail-closed reason boundary: a worker may self-report ONLY a terminal-logic verdict. A
/// pre-commit-crash reason (`WorkerCrashed`/`TimedOut`) or the `UNSPECIFIED` proto default
/// is rejected — accepting one would (under an `EffectStaged`) leave the Mote
/// re-dispatchable forever (the F4 hang).
#[tokio::test]
async fn pre_commit_crash_or_unspecified_reason_is_rejected() {
    let svc = coordinator();
    let warrant = common::sample_warrant();
    let worker = common::register(&svc, "w").await;
    let m = common::mote(15, NdClass::Pure, &[]);
    common::submit(&svc, &m, &warrant).await;
    common::lease_work(&svc, worker, MAC, 16).await;

    for bad in [
        FailureReason::Unspecified,
        FailureReason::TimedOut,
        FailureReason::WorkerCrashed,
    ] {
        let err = common::report_failure(&svc, &m, worker, bad)
            .await
            .expect_err("a non-worker-reportable reason is rejected");
        assert_eq!(
            err.code(),
            tonic::Code::InvalidArgument,
            "reason {bad:?} must be refused at the fail-closed boundary"
        );
    }
    assert_ne!(
        svc.state_of(m.id).await.unwrap(),
        MoteState::Failed,
        "no Failed fact was written for any rejected reason"
    );
}
