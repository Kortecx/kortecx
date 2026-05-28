//! DAG integration scenarios driven through the coordinator (in-process, direct
//! trait calls). These exercise the scheduler/projection integration: submitting
//! a DAG registers edges, and committing parents advances `ready_set`. They also
//! cover nd_class variety and parent-bearing commits.
//!
//! Note: P2.2 is the *passive* control plane — it does not gate `ReportCommit` on
//! readiness (that is P2.3 dispatch). These tests commit in dependency order (the
//! order a dispatching coordinator would hand work out) and assert the projection
//! tracks readiness correctly.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use kx_coordinator::{CoordinatorService, MoteState};
use kx_journal::InMemoryJournal;
use kx_mote::NdClass;

fn coordinator() -> CoordinatorService {
    CoordinatorService::new(InMemoryJournal::new())
}

#[tokio::test]
async fn linear_chain_a_b_c_progresses_ready_set() {
    let svc = coordinator();
    let warrant = common::sample_warrant();
    let worker = common::register(&svc, "w").await;

    let a = common::mote(1, NdClass::Pure, &[]);
    let b = common::mote(2, NdClass::Pure, &[a.id]);
    let c = common::mote(3, NdClass::Pure, &[b.id]);

    for m in [&a, &b, &c] {
        let r = common::submit(&svc, m, &warrant).await;
        assert_eq!(
            r.status,
            kx_coordinator::proto::SubmitStatus::Accepted as i32
        );
    }

    // Only the root is ready; B and C are blocked on uncommitted parents.
    let ready = svc.ready_set().await.unwrap();
    assert_eq!(ready, vec![a.id], "only the root is ready initially");

    common::commit(&svc, &a, worker).await;
    assert_eq!(svc.ready_set().await.unwrap(), vec![b.id]);

    common::commit(&svc, &b, worker).await;
    assert_eq!(svc.ready_set().await.unwrap(), vec![c.id]);

    common::commit(&svc, &c, worker).await;
    assert!(svc.ready_set().await.unwrap().is_empty());

    assert_eq!(svc.committed_count().await.unwrap(), 3);
    for m in [&a, &b, &c] {
        assert_eq!(svc.state_of(m.id).await.unwrap(), MoteState::Committed);
    }
}

#[tokio::test]
async fn diamond_dag_only_ready_when_all_parents_commit() {
    let svc = coordinator();
    let warrant = common::sample_warrant();
    let worker = common::register(&svc, "w").await;

    // A -> {B, C} -> D
    let a = common::mote(10, NdClass::Pure, &[]);
    let b = common::mote(11, NdClass::Pure, &[a.id]);
    let c = common::mote(12, NdClass::Pure, &[a.id]);
    let d = common::mote(13, NdClass::Pure, &[b.id, c.id]);
    for m in [&a, &b, &c, &d] {
        common::submit(&svc, m, &warrant).await;
    }

    common::commit(&svc, &a, worker).await;
    // Both B and C become ready; D still blocked.
    let mut ready = svc.ready_set().await.unwrap();
    ready.sort();
    let mut expected = vec![b.id, c.id];
    expected.sort();
    assert_eq!(ready, expected);

    common::commit(&svc, &b, worker).await;
    // D needs BOTH parents — still blocked after only B.
    assert_eq!(svc.ready_set().await.unwrap(), vec![c.id]);

    common::commit(&svc, &c, worker).await;
    assert_eq!(svc.ready_set().await.unwrap(), vec![d.id]);

    common::commit(&svc, &d, worker).await;
    assert_eq!(svc.committed_count().await.unwrap(), 4);
}

#[tokio::test]
async fn multi_root_dag_both_roots_ready() {
    let svc = coordinator();
    let warrant = common::sample_warrant();
    let worker = common::register(&svc, "w").await;

    // Two independent roots R1, R2 -> J (join).
    let r1 = common::mote(20, NdClass::Pure, &[]);
    let r2 = common::mote(21, NdClass::Pure, &[]);
    let j = common::mote(22, NdClass::Pure, &[r1.id, r2.id]);
    for m in [&r1, &r2, &j] {
        common::submit(&svc, m, &warrant).await;
    }

    let mut ready = svc.ready_set().await.unwrap();
    ready.sort();
    let mut expected = vec![r1.id, r2.id];
    expected.sort();
    assert_eq!(ready, expected, "both roots ready");

    common::commit(&svc, &r1, worker).await;
    common::commit(&svc, &r2, worker).await;
    assert_eq!(svc.ready_set().await.unwrap(), vec![j.id]);
    common::commit(&svc, &j, worker).await;
    assert_eq!(svc.committed_count().await.unwrap(), 3);
}

#[tokio::test]
async fn nd_class_variety_round_trips_and_commits() {
    let svc = coordinator();
    let warrant = common::sample_warrant();
    let worker = common::register(&svc, "w").await;

    for (seed, nd) in [
        (30u8, NdClass::Pure),
        (31, NdClass::ReadOnlyNondet),
        (32, NdClass::WorldMutating),
    ] {
        let m = common::mote(seed, nd, &[]);
        common::submit(&svc, &m, &warrant).await;
        let commit = common::commit(&svc, &m, worker).await;
        assert_eq!(
            commit.outcome,
            kx_coordinator::proto::CommitOutcome::Committed as i32
        );
        assert_eq!(svc.state_of(m.id).await.unwrap(), MoteState::Committed);
    }
    assert_eq!(svc.committed_count().await.unwrap(), 3);
}

#[tokio::test]
async fn parent_bearing_commit_preserves_edges() {
    let svc = coordinator();
    let warrant = common::sample_warrant();
    let worker = common::register(&svc, "w").await;

    let parent = common::mote(40, NdClass::Pure, &[]);
    let child = common::mote(41, NdClass::Pure, &[parent.id]);
    common::submit(&svc, &parent, &warrant).await;
    common::submit(&svc, &child, &warrant).await;

    common::commit(&svc, &parent, worker).await;
    // The child carries a parent edge in its ReportCommit; committing it succeeds
    // and the child becomes committed (its declared parent is committed).
    let commit = common::commit(&svc, &child, worker).await;
    assert_eq!(
        commit.outcome,
        kx_coordinator::proto::CommitOutcome::Committed as i32
    );
    assert_eq!(svc.state_of(child.id).await.unwrap(), MoteState::Committed);
}
