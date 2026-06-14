//! PR-4.1 `SubmitFeedback` / `ListFeedback` end-to-end over a REAL bound tonic
//! port — the GR8 proofs for the client-origin product-signal write:
//!
//! - **no-journal-write**: a feedback burst moves the journal head by ZERO (the
//!   write lands in the `feedback.db` sidecar, never the journal — the digest
//!   cannot move by construction);
//! - **auth**: deny-all (no `--dev-allow-local`) refuses `SubmitFeedback`;
//! - **overwrite idempotency over the wire**: re-rating the same answer keeps a
//!   single row (the server-derived deterministic id);
//! - **durability**: the sidecar survives a restart.

#![cfg(feature = "embedded-worker")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Duration;

use kx_gateway::{start, GatewayConfig};
use kx_journal::{Journal, SqliteJournal};
use kx_proto::proto;
use kx_proto::proto::kx_gateway_client::KxGatewayClient;
use tempfile::TempDir;
use tonic::transport::Channel;
use tonic::Code;

fn config(dir: &TempDir, dev_allow_local: bool) -> GatewayConfig {
    common::gateway_config(dir, dev_allow_local, HashMap::new())
}

async fn client(addr: SocketAddr) -> KxGatewayClient<Channel> {
    let endpoint = format!("http://{addr}");
    for _ in 0..100 {
        if let Ok(c) = KxGatewayClient::connect(endpoint.clone()).await {
            return c;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("client connects to the gateway at {endpoint}");
}

fn fb(rating: proto::FeedbackRating, message_id: &str) -> proto::SubmitFeedbackRequest {
    proto::SubmitFeedbackRequest {
        rating: rating as i32,
        message_id: message_id.into(),
        instance_id: None,
        mote_id: None,
        content_ref: None,
        comment: String::new(),
        recipe_handle: "kx/recipes/chat".into(),
        model_id: "qwen3".into(),
    }
}

#[tokio::test]
async fn submit_feedback_round_trips_with_zero_journal_writes() {
    let dir = TempDir::new().unwrap();
    let journal_path = dir.path().join("kx.db");
    let running = start(config(&dir, true)).await.unwrap();
    let mut c = client(running.local_addr()).await;

    let head_before = SqliteJournal::open(&journal_path)
        .unwrap()
        .current_seq()
        .unwrap();

    let resp = c
        .submit_feedback(fb(proto::FeedbackRating::Up, "answer-1"))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(resp.feedback_id.len(), 16, "16B server-derived id");

    let page = c
        .list_feedback(proto::ListFeedbackRequest {
            limit: None,
            instance_id: None,
            before_rowid: None,
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(page.rows.len(), 1);
    assert_eq!(page.rows[0].message_id, "answer-1");
    assert_eq!(page.rows[0].rating, proto::FeedbackRating::Up as i32);

    // THE no-journal-write proof: the burst moved the head by ZERO entries.
    let head_after = SqliteJournal::open(&journal_path)
        .unwrap()
        .current_seq()
        .unwrap();
    assert_eq!(
        head_before, head_after,
        "SubmitFeedback must never write the journal (digest-invariant by construction)"
    );

    running.shutdown().await.unwrap();
}

#[tokio::test]
async fn deny_all_refuses_submit_feedback() {
    let dir = TempDir::new().unwrap();
    let running = start(config(&dir, false)).await.unwrap(); // NO dev flag ⇒ deny-all
    let mut c = client(running.local_addr()).await;
    let err = c
        .submit_feedback(fb(proto::FeedbackRating::Up, "x"))
        .await
        .unwrap_err();
    assert!(
        matches!(err.code(), Code::Unauthenticated | Code::PermissionDenied),
        "an unauthenticated feedback must be refused, got {:?}",
        err.code()
    );
    running.shutdown().await.unwrap();
}

#[tokio::test]
async fn re_rating_overwrites_over_the_wire() {
    let dir = TempDir::new().unwrap();
    let running = start(config(&dir, true)).await.unwrap();
    let mut c = client(running.local_addr()).await;

    let up = c
        .submit_feedback(fb(proto::FeedbackRating::Up, "same"))
        .await
        .unwrap()
        .into_inner();
    let down = c
        .submit_feedback(fb(proto::FeedbackRating::Down, "same"))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(up.feedback_id, down.feedback_id, "same answer ⇒ same id");

    let page = c
        .list_feedback(proto::ListFeedbackRequest {
            limit: None,
            instance_id: None,
            before_rowid: None,
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(page.rows.len(), 1, "the re-rating overwrote, not appended");
    assert_eq!(page.rows[0].rating, proto::FeedbackRating::Down as i32);

    running.shutdown().await.unwrap();
}

#[tokio::test]
async fn feedback_survives_a_restart() {
    let dir = TempDir::new().unwrap();
    {
        let running = start(config(&dir, true)).await.unwrap();
        let mut c = client(running.local_addr()).await;
        c.submit_feedback(fb(proto::FeedbackRating::Down, "durable"))
            .await
            .unwrap();
        running.shutdown().await.unwrap();
    }
    let running = start(config(&dir, true)).await.unwrap();
    let mut c = client(running.local_addr()).await;
    let page = c
        .list_feedback(proto::ListFeedbackRequest {
            limit: None,
            instance_id: None,
            before_rowid: None,
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        page.rows.len(),
        1,
        "the feedback sidecar survives a restart"
    );
    assert_eq!(page.rows[0].message_id, "durable");
    running.shutdown().await.unwrap();
}
