//! Exhaustive boundary / refusal coverage. Every malformed or inadmissible
//! request is rejected with `INVALID_ARGUMENT` and leaves the journal untouched.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use kx_coordinator::proto;
use kx_coordinator::proto::coordinator_server::Coordinator;
use kx_coordinator::{CoordinatorService, WorkerId};
use kx_journal::InMemoryJournal;
use kx_mote::NdClass;
use kx_warrant::ExecutorClass;
use tonic::{Code, Request};

fn coordinator() -> CoordinatorService {
    CoordinatorService::new(InMemoryJournal::new())
}

/// Register a worker + submit a Mote so a `ReportCommit` reaches the validation
/// boundary (past the worker-admission + mote-admission guards). Returns the
/// service, the worker id, and the Mote.
async fn ready_to_report() -> (CoordinatorService, u64, kx_mote::Mote) {
    let svc = coordinator();
    let warrant = common::sample_warrant();
    let worker = common::register(&svc, "w").await;
    let mote = common::pure_root_mote();
    common::submit(&svc, &mote, &warrant).await;
    (svc, worker, mote)
}

async fn assert_commit_rejected(svc: &CoordinatorService, req: proto::ReportCommitRequest) {
    let err = svc.report_commit(Request::new(req)).await.unwrap_err();
    assert_eq!(
        err.code(),
        Code::InvalidArgument,
        "expected INVALID_ARGUMENT"
    );
    assert_eq!(
        svc.committed_count().await.unwrap(),
        0,
        "a rejected ReportCommit must not write the journal"
    );
}

#[tokio::test]
async fn every_32_byte_field_is_length_validated() {
    let (svc, worker, mote) = ready_to_report().await;

    // mote_id wrong length. (idempotency_key follows mote_id, so corrupt both to
    // isolate the mote_id check — otherwise the identity-mismatch check fires.)
    let mut bad = common::report_commit_request(&mote, worker);
    bad.mote_id = vec![1u8; 31];
    bad.idempotency_key = vec![1u8; 31];
    assert_commit_rejected(&svc, bad).await;

    let corruptors: [fn(&mut proto::ReportCommitRequest); 4] = [
        |r| r.idempotency_key = vec![2u8; 31],
        |r| r.result_ref = vec![3u8; 33],
        |r| r.warrant_ref = vec![4u8; 0],
        |r| r.mote_def_hash = vec![5u8; 16],
    ];
    for corrupt in corruptors {
        let mut req = common::report_commit_request(&mote, worker);
        corrupt(&mut req);
        assert_commit_rejected(&svc, req).await;
    }
}

#[tokio::test]
async fn idempotency_key_must_equal_mote_id() {
    let (svc, worker, mote) = ready_to_report().await;
    let mut bad = common::report_commit_request(&mote, worker);
    bad.idempotency_key = vec![0xAB; 32]; // valid length, wrong value
    assert_commit_rejected(&svc, bad).await;
}

#[tokio::test]
async fn out_of_range_nd_class_is_rejected() {
    let (svc, worker, mote) = ready_to_report().await;
    let mut bad = common::report_commit_request(&mote, worker);
    bad.nd_class = 12_345; // not a defined NdClass discriminant
    assert_commit_rejected(&svc, bad).await;
}

#[tokio::test]
async fn unspecified_nd_class_is_rejected() {
    let (svc, worker, mote) = ready_to_report().await;
    let mut bad = common::report_commit_request(&mote, worker);
    bad.nd_class = proto::NdClass::Unspecified as i32; // 0 — the rejected sentinel
    assert_commit_rejected(&svc, bad).await;
}

#[tokio::test]
async fn bad_parent_id_length_is_rejected() {
    let (svc, worker, mote) = ready_to_report().await;
    let mut bad = common::report_commit_request(&mote, worker);
    bad.parents = vec![proto::ParentRef {
        parent_id: vec![1u8; 31], // not 32 bytes
        edge_kind: proto::EdgeKind::Data as i32,
        non_cascade: false,
    }];
    assert_commit_rejected(&svc, bad).await;
}

#[tokio::test]
async fn submit_missing_mote_or_warrant_is_rejected() {
    let svc = coordinator();
    let warrant = common::sample_warrant();
    let mote = common::pure_root_mote();

    let err = svc
        .submit_mote(Request::new(proto::SubmitMoteRequest {
            mote: None,
            warrant: Some(warrant.clone().into()),
            accept_at_least_once: false,
        }))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::InvalidArgument);

    let err = svc
        .submit_mote(Request::new(proto::SubmitMoteRequest {
            mote: Some(mote.into()),
            warrant: None,
            accept_at_least_once: false,
        }))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::InvalidArgument);
    assert_eq!(svc.committed_count().await.unwrap(), 0);
}

#[tokio::test]
async fn register_accepts_every_executor_class() {
    let svc = coordinator();
    for (proto_class, domain_class) in [
        (proto::ExecutorClass::Bwrap, ExecutorClass::Bwrap),
        (proto::ExecutorClass::OciDaemon, ExecutorClass::OciDaemon),
        (
            proto::ExecutorClass::CloudMicroVm,
            ExecutorClass::CloudMicroVm,
        ),
        (
            proto::ExecutorClass::MacosSandbox,
            ExecutorClass::MacOsSandbox,
        ),
    ] {
        let id = svc
            .register_worker(Request::new(proto::RegisterWorkerRequest {
                executor_class: proto_class as i32,
                endpoint: "endpoint".into(),
            }))
            .await
            .unwrap()
            .into_inner()
            .worker_id;
        let record = svc.registry().get(WorkerId(id)).unwrap();
        assert_eq!(record.executor_class, domain_class);
    }
}

#[tokio::test]
async fn register_rejects_unspecified_and_unknown_executor_class() {
    let svc = coordinator();

    let err = svc
        .register_worker(Request::new(proto::RegisterWorkerRequest {
            executor_class: proto::ExecutorClass::Unspecified as i32, // 0
            endpoint: "e".into(),
        }))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::InvalidArgument);

    let err = svc
        .register_worker(Request::new(proto::RegisterWorkerRequest {
            executor_class: 9_999, // not a defined discriminant
            endpoint: "e".into(),
        }))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::InvalidArgument);
    assert_eq!(svc.registry().len(), 0);
}

#[tokio::test]
async fn report_commit_unknown_mote_with_nondefault_nd_class() {
    // A WORLD-MUTATING commit for a never-submitted Mote is still refused (the
    // admission guard fires before any write), regardless of nd_class.
    let svc = coordinator();
    let worker = common::register(&svc, "w").await;
    let mote = common::mote(7, NdClass::WorldMutating, &[]);
    let err = svc
        .report_commit(Request::new(common::report_commit_request(&mote, worker)))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::InvalidArgument);
    assert_eq!(svc.committed_count().await.unwrap(), 0);
}

#[tokio::test]
async fn too_many_parents_rejected_no_write() {
    // > 128 parents would fail the journal encoder — validated up front so it
    // can't poison a group-commit batch. Rejected individually, no write.
    let (svc, worker, mote) = ready_to_report().await;
    let mut bad = common::report_commit_request(&mote, worker);
    bad.parents = (0..129u32)
        .map(|i| proto::ParentRef {
            parent_id: vec![u8::try_from(i % 256).unwrap(); 32],
            edge_kind: proto::EdgeKind::Data as i32,
            non_cascade: false,
        })
        .collect();
    assert_commit_rejected(&svc, bad).await;
}

#[tokio::test]
async fn data_edge_non_cascade_rejected_no_write() {
    // A Data edge marked non_cascade is forbidden by the encoder; validated up
    // front so it can't poison a group-commit batch.
    let (svc, worker, mote) = ready_to_report().await;
    let mut bad = common::report_commit_request(&mote, worker);
    bad.parents = vec![proto::ParentRef {
        parent_id: vec![1u8; 32],
        edge_kind: proto::EdgeKind::Data as i32,
        non_cascade: true,
    }];
    assert_commit_rejected(&svc, bad).await;
}
