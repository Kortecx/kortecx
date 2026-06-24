//! POC-5 end-to-end over a REAL bound tonic port (MODEL-FREE — the deterministic
//! half of the seam, GR16 #5). Drives the new POC-5a/5b RPCs through the live
//! gateway + the `branches.db` / `locks.db` host stores:
//!
//! - **GetBranchContent** caller-scoped read: Alice reads her branch file body;
//!   Bob gets a UNIFORM not-found (no cross-party oracle); an absent path is the
//!   same uniform not-found.
//! - **per-App lock** at the AdvanceBranch chokepoint: a locked branch refuses an
//!   agentic in-CAS edit with `FAILED_PRECONDITION` + `kx-refusal-code: LOCKED_BRANCH`;
//!   unlock restores it. The lock is CALLER-SCOPED (Bob's lock never gates Alice).
//! - **ScaffoldApp** without a served model is `unimplemented` (fail-closed).

#![cfg(feature = "embedded-worker")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::HashMap;
use std::net::SocketAddr;

use kx_proto::proto;
use kx_proto::proto::kx_gateway_client::KxGatewayClient;
use tonic::transport::Channel;
use tonic::{Code, Request};

use kx_gateway::start;

async fn client(addr: SocketAddr) -> KxGatewayClient<Channel> {
    common::connect_client(addr).await
}

fn with_bearer<T>(payload: T, token: &str) -> Request<T> {
    let mut req = Request::new(payload);
    req.metadata_mut()
        .insert("authorization", format!("Bearer {token}").parse().unwrap());
    req
}

fn two_party_tokens() -> HashMap<String, String> {
    HashMap::from([
        ("tok-alice".to_string(), "alice@acme".to_string()),
        ("tok-bob".to_string(), "bob@acme".to_string()),
    ])
}

/// PutContent a body, returning its server-derived 32-byte ref (the branch advance
/// target — strictly in-CAS, the body must already be a committed blob).
async fn put(c: &mut KxGatewayClient<Channel>, token: &str, body: &[u8]) -> Vec<u8> {
    c.put_content(with_bearer(
        proto::PutContentRequest {
            payload: body.to_vec(),
            media_type: String::new(),
            filename: String::new(),
        },
        token,
    ))
    .await
    .unwrap()
    .into_inner()
    .content_ref
}

#[tokio::test]
async fn get_branch_content_is_caller_scoped_and_uniform_not_found() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, false, two_party_tokens()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    let handle = "team/apps/a".to_string();
    let body = b"# README\n\nthe scaffolded readme body";
    let cref = put(&mut c, "tok-alice", body).await;

    c.create_branch(with_bearer(
        proto::CreateBranchRequest {
            handle: handle.clone(),
            description: "app branch".into(),
            parent_handle: String::new(),
        },
        "tok-alice",
    ))
    .await
    .unwrap();
    c.advance_branch(with_bearer(
        proto::AdvanceBranchRequest {
            handle: handle.clone(),
            path: "README.md".into(),
            content_ref: cref.clone(),
        },
        "tok-alice",
    ))
    .await
    .unwrap();

    // Alice reads her own file body.
    let got = c
        .get_branch_content(with_bearer(
            proto::GetBranchContentRequest {
                handle: handle.clone(),
                path: "README.md".into(),
            },
            "tok-alice",
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(got.found);
    assert_eq!(got.payload, body);

    // Bob gets a UNIFORM not-found (no cross-party existence oracle).
    let bob = c
        .get_branch_content(with_bearer(
            proto::GetBranchContentRequest {
                handle: handle.clone(),
                path: "README.md".into(),
            },
            "tok-bob",
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(!bob.found);
    assert!(bob.payload.is_empty());

    // An absent path is the SAME uniform not-found.
    let absent = c
        .get_branch_content(with_bearer(
            proto::GetBranchContentRequest {
                handle: handle.clone(),
                path: "does-not-exist.md".into(),
            },
            "tok-alice",
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(!absent.found);
}

#[tokio::test]
async fn lock_refuses_locked_edit_then_unlock_restores() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, false, two_party_tokens()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    let handle = "team/apps/b".to_string();
    let cref = put(&mut c, "tok-alice", b"v1").await;
    let cref2 = put(&mut c, "tok-alice", b"v2").await;
    c.create_branch(with_bearer(
        proto::CreateBranchRequest {
            handle: handle.clone(),
            description: String::new(),
            parent_handle: String::new(),
        },
        "tok-alice",
    ))
    .await
    .unwrap();
    let advance_req = |r: Vec<u8>| proto::AdvanceBranchRequest {
        handle: handle.clone(),
        path: "README.md".into(),
        content_ref: r,
    };

    // Unlocked: advance succeeds.
    c.advance_branch(with_bearer(advance_req(cref.clone()), "tok-alice"))
        .await
        .unwrap();

    // Lock the App's branch.
    let locked = c
        .lock_app(with_bearer(
            proto::LockAppRequest {
                branch_handle: handle.clone(),
            },
            "tok-alice",
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(locked.locked);

    // Locked: the agentic edit is REFUSED with the structured refusal code.
    let err = c
        .advance_branch(with_bearer(advance_req(cref2.clone()), "tok-alice"))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::FailedPrecondition);
    assert_eq!(
        err.metadata()
            .get(kx_gateway_core::REFUSAL_CODE_METADATA_KEY)
            .and_then(|v| v.to_str().ok()),
        Some("LOCKED_BRANCH"),
    );

    // Unlock restores agentic edits.
    let unlocked = c
        .unlock_app(with_bearer(
            proto::UnlockAppRequest {
                branch_handle: handle.clone(),
            },
            "tok-alice",
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(unlocked.unlocked);
    c.advance_branch(with_bearer(advance_req(cref2), "tok-alice"))
        .await
        .unwrap();
}

#[tokio::test]
async fn lock_is_caller_scoped() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, false, two_party_tokens()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    let handle = "team/apps/c".to_string();
    let cref = put(&mut c, "tok-alice", b"x").await;
    c.create_branch(with_bearer(
        proto::CreateBranchRequest {
            handle: handle.clone(),
            description: String::new(),
            parent_handle: String::new(),
        },
        "tok-alice",
    ))
    .await
    .unwrap();

    // Bob locks the SAME handle string — but the lock is keyed by (principal,
    // branch_handle), so Bob's lock lives under Bob's scope and never gates Alice.
    c.lock_app(with_bearer(
        proto::LockAppRequest {
            branch_handle: handle.clone(),
        },
        "tok-bob",
    ))
    .await
    .unwrap();

    // Alice's advance still succeeds (Bob cannot lock Alice's branch).
    c.advance_branch(with_bearer(
        proto::AdvanceBranchRequest {
            handle: handle.clone(),
            path: "README.md".into(),
            content_ref: cref,
        },
        "tok-alice",
    ))
    .await
    .unwrap();
}

#[tokio::test]
async fn scaffold_app_without_a_served_model_is_unimplemented() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, false, two_party_tokens()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    let err = c
        .scaffold_app(with_bearer(
            proto::ScaffoldAppRequest {
                handle: "team/apps/x".into(),
                branch_handle: String::new(),
                instruction: "build a thing".into(),
            },
            "tok-alice",
        ))
        .await
        .unwrap_err();
    // Model-free serve ⇒ no scaffold orchestrator ⇒ fail-closed `unimplemented`.
    assert_eq!(err.code(), Code::Unimplemented);
}
