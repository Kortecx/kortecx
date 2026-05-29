//! `LeaseWork` serving (P2.3 dispatch surface): the coordinator hands a worker
//! the ready PURE Motes it can run, each with the warrant it was submitted under.
//!
//! Selection contract exercised here:
//! - **ready** — only Motes whose parents are all committed (the dispatch
//!   precondition; tracks the projection ready-set as parents commit);
//! - **PURE** — WORLD-MUTATING / READ-ONLY-NONDET are NOT leased (P2.3 runs the
//!   PURE path only; WM needs a durable staged-intent RPC, deferred);
//! - **executor_class** — only Motes whose warrant matches the worker's backend;
//! - **max_motes** — the worker bounds how many it pulls per call;
//! - **admission** — only a registered worker may lease.
//!
//! The returned `WorkItem` carries a full Mote + warrant; identity must survive
//! the wire mapping so the worker can re-derive a `ReportCommit` the coordinator
//! accepts.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use kx_coordinator::proto::coordinator_server::Coordinator;
use kx_coordinator::proto::ExecutorClass;
use kx_coordinator::CoordinatorService;
use kx_journal::InMemoryJournal;
use kx_mote::NdClass;
use kx_warrant::warrant_ref_of;
use tonic::{Code, Request};

fn coordinator() -> CoordinatorService {
    CoordinatorService::new(InMemoryJournal::new())
}

#[tokio::test]
async fn leases_the_ready_root_then_advances_as_parents_commit() {
    let svc = coordinator();
    let warrant = common::sample_warrant(); // executor_class = MacOsSandbox
    let worker = common::register(&svc, "w").await; // registers MacosSandbox

    let a = common::mote(1, NdClass::Pure, &[]);
    let b = common::mote(2, NdClass::Pure, &[a.id]);
    common::submit(&svc, &a, &warrant).await;
    common::submit(&svc, &b, &warrant).await;

    // Only the root is ready; the lease returns it with its warrant.
    let leased = common::lease_work(&svc, worker, ExecutorClass::MacosSandbox, 16).await;
    assert_eq!(leased.len(), 1, "only the root is leasable");
    let item = &leased[0];
    let leased_mote: kx_mote::Mote = item.mote.clone().unwrap().try_into().unwrap();
    let leased_warrant: kx_warrant::WarrantSpec = item.warrant.clone().unwrap().try_into().unwrap();
    assert_eq!(leased_mote.id, a.id, "leased the ready root");
    assert_eq!(
        warrant_ref_of(&leased_warrant),
        warrant_ref_of(&warrant),
        "warrant identity survives the lease"
    );

    // Commit the root; the child becomes leasable.
    common::commit(&svc, &a, worker).await;
    let leased = common::lease_work(&svc, worker, ExecutorClass::MacosSandbox, 16).await;
    assert_eq!(leased.len(), 1);
    let leased_mote: kx_mote::Mote = leased[0].mote.clone().unwrap().try_into().unwrap();
    assert_eq!(leased_mote.id, b.id, "child leasable once parent committed");

    // Commit the child; nothing left to lease.
    common::commit(&svc, &b, worker).await;
    assert!(
        common::lease_work(&svc, worker, ExecutorClass::MacosSandbox, 16)
            .await
            .is_empty(),
        "no work remains once the DAG is committed"
    );
}

#[tokio::test]
async fn leases_world_mutating_motes_since_p3_6() {
    let svc = coordinator();
    let warrant = common::sample_warrant();
    let worker = common::register(&svc, "w").await;

    // A ready (parentless) WORLD-MUTATING Mote. P2.3 kept it out of the lease (PURE-only);
    // D58 (P3.6) lifts that — WM is now leasable (the worker stages its intent via
    // ReportEffectStaged before firing). It still respects executor_class.
    let wm = common::mote(7, NdClass::WorldMutating, &[]);
    common::submit(&svc, &wm, &warrant).await;

    let leased = common::lease_work(&svc, worker, ExecutorClass::MacosSandbox, 16).await;
    assert_eq!(
        leased.len(),
        1,
        "WORLD-MUTATING Motes are leasable since P3.6 (D58)"
    );
    let leased_mote: kx_mote::Mote = leased[0].mote.clone().unwrap().try_into().unwrap();
    assert_eq!(leased_mote.id, wm.id);
}

#[tokio::test]
async fn does_not_lease_across_executor_class() {
    let svc = coordinator();
    let warrant = common::sample_warrant(); // executor_class = MacOsSandbox
    let worker = common::register(&svc, "w").await;

    let a = common::mote(1, NdClass::Pure, &[]);
    common::submit(&svc, &a, &warrant).await;

    // A Bwrap worker cannot run a MacOsSandbox-warranted Mote.
    assert!(
        common::lease_work(&svc, worker, ExecutorClass::Bwrap, 16)
            .await
            .is_empty(),
        "a mismatched executor_class leases nothing"
    );
    // The matching class does.
    assert_eq!(
        common::lease_work(&svc, worker, ExecutorClass::MacosSandbox, 16)
            .await
            .len(),
        1
    );
}

#[tokio::test]
async fn max_motes_bounds_the_lease() {
    let svc = coordinator();
    let warrant = common::sample_warrant();
    let worker = common::register(&svc, "w").await;

    // Two independent ready PURE roots.
    let a = common::mote(1, NdClass::Pure, &[]);
    let b = common::mote(2, NdClass::Pure, &[]);
    common::submit(&svc, &a, &warrant).await;
    common::submit(&svc, &b, &warrant).await;

    assert_eq!(
        common::lease_work(&svc, worker, ExecutorClass::MacosSandbox, 1)
            .await
            .len(),
        1,
        "max_motes caps the batch"
    );
    assert_eq!(
        common::lease_work(&svc, worker, ExecutorClass::MacosSandbox, 16)
            .await
            .len(),
        2,
        "both roots leasable without the cap"
    );
}

#[tokio::test]
async fn unregistered_worker_cannot_lease() {
    let svc = coordinator();
    let warrant = common::sample_warrant();
    let _registered = common::register(&svc, "w").await;

    let a = common::mote(1, NdClass::Pure, &[]);
    common::submit(&svc, &a, &warrant).await;

    // worker_id 999 was never registered.
    let status = svc
        .lease_work(Request::new(kx_coordinator::proto::LeaseWorkRequest {
            worker_id: 999,
            executor_class: ExecutorClass::MacosSandbox as i32,
            max_motes: 16,
        }))
        .await
        .map(|_| ())
        .unwrap_err();
    // UnknownWorker is an inadmissible request → INVALID_ARGUMENT (see error.rs).
    assert_eq!(
        status.code(),
        Code::InvalidArgument,
        "unregistered workers are refused"
    );
}
