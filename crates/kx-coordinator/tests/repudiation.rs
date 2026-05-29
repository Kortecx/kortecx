//! Poison-invalidation cascade (P3.5 / P0.7, D22): repudiating a committed Mote cascades
//! to its committed downstream consumers (one `Repudiated{UpstreamCascade}` each), the
//! coordinator writes the batch through its sole-writer thread, and the next `LeaseWork`
//! offers none of them. Idempotent (re-repudiation dedupes); only committed Motes are
//! repudiable.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use kx_coordinator::proto::ExecutorClass;
use kx_coordinator::{CoordinatorService, MoteState, RepudiationError, RepudiationReason};
use kx_journal::InMemoryJournal;
use kx_mote::NdClass;

const MAC: ExecutorClass = ExecutorClass::MacosSandbox;

fn coordinator() -> CoordinatorService {
    CoordinatorService::new(InMemoryJournal::new())
}

/// a → b → c (data edges). Repudiating `a` cascades to `b` and `c`.
#[tokio::test]
async fn repudiating_a_root_cascades_to_committed_downstream() {
    let svc = coordinator();
    let warrant = common::sample_warrant();
    let worker = common::register(&svc, "w").await;

    let a = common::mote(1, NdClass::Pure, &[]);
    let b = common::mote(2, NdClass::Pure, &[a.id]);
    let c = common::mote(3, NdClass::Pure, &[b.id]);
    for m in [&a, &b, &c] {
        common::submit(&svc, m, &warrant).await;
    }
    // Commit the whole chain (in dependency order).
    common::commit(&svc, &a, worker).await;
    common::commit(&svc, &b, worker).await;
    common::commit(&svc, &c, worker).await;
    for m in [&a, &b, &c] {
        assert_eq!(svc.state_of(m.id).await.unwrap(), MoteState::Committed);
    }

    // Repudiate the root → cascade marks b and c.
    let outcome = svc
        .repudiate(a.id, RepudiationReason::OperatorAction, 42)
        .await
        .unwrap();
    assert_eq!(outcome.target, a.id);
    assert_eq!(outcome.cascade_size, 2, "b and c are downstream of a");

    for m in [&a, &b, &c] {
        assert_eq!(
            svc.state_of(m.id).await.unwrap(),
            MoteState::Repudiated,
            "the whole poisoned lineage is repudiated"
        );
    }
    assert_eq!(
        svc.committed_count().await.unwrap(),
        0,
        "no Mote remains committed after the cascade"
    );

    // Re-repudiating is idempotent: the journal dedupes by key, state is unchanged.
    let again = svc
        .repudiate(a.id, RepudiationReason::OperatorAction, 42)
        .await
        .unwrap();
    assert_eq!(
        again.cascade_size, 2,
        "the cascade set is recomputed identically"
    );
    for m in [&a, &b, &c] {
        assert_eq!(svc.state_of(m.id).await.unwrap(), MoteState::Repudiated);
    }
}

/// Repudiating a leaf invalidates only it — the cascade is downstream-only.
#[tokio::test]
async fn repudiating_a_leaf_does_not_touch_its_ancestors() {
    let svc = coordinator();
    let warrant = common::sample_warrant();
    let worker = common::register(&svc, "w").await;

    let a = common::mote(1, NdClass::Pure, &[]);
    let b = common::mote(2, NdClass::Pure, &[a.id]);
    common::submit(&svc, &a, &warrant).await;
    common::submit(&svc, &b, &warrant).await;
    common::commit(&svc, &a, worker).await;
    common::commit(&svc, &b, worker).await;

    let outcome = svc
        .repudiate(b.id, RepudiationReason::OperatorAction, 1)
        .await
        .unwrap();
    assert_eq!(outcome.cascade_size, 0, "b is a leaf — no downstream");
    assert_eq!(svc.state_of(b.id).await.unwrap(), MoteState::Repudiated);
    assert_eq!(
        svc.state_of(a.id).await.unwrap(),
        MoteState::Committed,
        "the ancestor is untouched — cascade flows downstream only"
    );
}

/// After a cascade, the coordinator's `LeaseWork` offers none of the repudiated Motes —
/// the distributed cascade is observed by workers through the lease gate.
#[tokio::test]
async fn repudiated_motes_are_not_leasable() {
    let svc = coordinator();
    let warrant = common::sample_warrant();
    let worker = common::register(&svc, "w").await;

    // a committed; b (child of a) submitted-but-not-committed → b is ready to lease.
    let a = common::mote(1, NdClass::Pure, &[]);
    let b = common::mote(2, NdClass::Pure, &[a.id]);
    common::submit(&svc, &a, &warrant).await;
    common::submit(&svc, &b, &warrant).await;
    common::commit(&svc, &a, worker).await;
    assert_eq!(
        common::lease_work(&svc, worker, MAC, 16).await.len(),
        1,
        "b is leasable while its parent a is committed"
    );

    // Repudiate a → b's parent is now repudiated → b is no longer leasable.
    svc.repudiate(a.id, RepudiationReason::OperatorAction, 1)
        .await
        .unwrap();
    assert!(
        common::lease_work(&svc, worker, MAC, 16).await.is_empty(),
        "no Mote with a repudiated parent is leased"
    );
}

#[tokio::test]
async fn repudiating_an_uncommitted_mote_is_refused() {
    let svc = coordinator();
    let warrant = common::sample_warrant();
    let _ = common::register(&svc, "w").await;
    let a = common::mote(1, NdClass::Pure, &[]);
    common::submit(&svc, &a, &warrant).await; // submitted, NOT committed

    let err = svc
        .repudiate(a.id, RepudiationReason::OperatorAction, 1)
        .await
        .unwrap_err();
    assert_eq!(err, RepudiationError::TargetNotCommitted(a.id));
}
