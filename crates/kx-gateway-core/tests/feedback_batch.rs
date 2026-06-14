//! PR-4.1 `SubmitFeedback` / `ListFeedback` over a real tonic transport — the
//! gateway-core boundary proofs:
//!
//! - the seam is `None` ⇒ both RPCs degrade to `Unimplemented` (old-host
//!   forward-compat, not an empty lie);
//! - the principal is SERVER-resolved (no stamped party ⇒ `Unauthenticated`);
//!   the `feedback_id` is SERVER-derived + DETERMINISTIC over `(message_id,
//!   principal)`, so a re-rating OVERWRITES (SN-8 — the client can neither name
//!   nor forge it);
//! - the rating must be UP/DOWN, the `message_id` is required, the comment is
//!   capped — all fail-closed BEFORE the write;
//! - malformed target ids are `InvalidArgument`; `ListFeedback` scopes by
//!   `instance_id`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::sync::Mutex;

use common::{spawn, spawn_with_party};
use kx_content::ContentRef;
use kx_gateway_core::{
    ContentReader, FeedbackEntry, FeedbackRecord, FeedbackStore, GatewayError, GatewayService,
    JournalReader, ReadOnly, RunSubmitter, MAX_FEEDBACK_COMMENT_BYTES,
};
use kx_proto::proto;
use tonic::Code;

/// An in-memory [`FeedbackStore`] fake (the host's `feedback.db` stand-in),
/// newest-last so `list` can reverse it.
#[derive(Default)]
struct MemFeedback {
    rows: Mutex<Vec<FeedbackRecord>>,
}

impl FeedbackStore for MemFeedback {
    fn record(&self, rec: FeedbackRecord) -> Result<(), GatewayError> {
        let mut rows = self.rows.lock().unwrap();
        // Overwrite-on-id (the host's INSERT OR REPLACE).
        if let Some(slot) = rows.iter_mut().find(|r| r.feedback_id == rec.feedback_id) {
            *slot = rec;
        } else {
            rows.push(rec);
        }
        Ok(())
    }

    fn list(
        &self,
        limit: usize,
        instance_id: Option<[u8; 16]>,
        _before_rowid: Option<u64>,
    ) -> Result<(Vec<FeedbackEntry>, bool), GatewayError> {
        let rows = self.rows.lock().unwrap();
        let mut out: Vec<FeedbackEntry> = rows
            .iter()
            .rev()
            .filter(|r| instance_id.is_none_or(|iid| r.instance_id == iid))
            .enumerate()
            .map(|(i, r)| FeedbackEntry {
                feedback_id: r.feedback_id,
                rating: r.rating,
                message_id: r.message_id.clone(),
                instance_id: r.instance_id,
                mote_id: r.mote_id,
                content_ref: r.content_ref,
                comment: r.comment.clone(),
                recipe_handle: r.recipe_handle.clone(),
                model_id: r.model_id.clone(),
                submitted_unix_ms: r.submitted_unix_ms,
                rowid: (i + 1) as u64,
            })
            .collect();
        let has_more = out.len() > limit;
        out.truncate(limit);
        Ok((out, has_more))
    }
}

fn build_service(feedback: Option<std::sync::Arc<MemFeedback>>) -> GatewayService {
    let run = common::build_run();
    let reader: std::sync::Arc<dyn JournalReader> = std::sync::Arc::new(ReadOnly::new(run.journal));
    let content: std::sync::Arc<dyn ContentReader> = std::sync::Arc::new(run.content);
    let submitter: std::sync::Arc<dyn RunSubmitter> =
        std::sync::Arc::new(common::MockSubmitter::default());
    let svc = GatewayService::new(reader, submitter, content);
    match feedback {
        Some(f) => svc.with_feedback_store(f),
        None => svc,
    }
}

fn submit(rating: proto::FeedbackRating, message_id: &str) -> proto::SubmitFeedbackRequest {
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
async fn submit_and_list_without_seam_are_unimplemented() {
    let mut client = spawn_with_party(build_service(None), "tester").await;
    let err = client
        .submit_feedback(submit(proto::FeedbackRating::Up, "m1"))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::Unimplemented);
    let err = client
        .list_feedback(proto::ListFeedbackRequest {
            limit: None,
            instance_id: None,
            before_rowid: None,
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::Unimplemented);
}

#[tokio::test]
async fn submit_without_party_is_unauthenticated() {
    let store = std::sync::Arc::new(MemFeedback::default());
    // `spawn` stamps NO CallerParty.
    let mut client = spawn(build_service(Some(store))).await;
    let err = client
        .submit_feedback(submit(proto::FeedbackRating::Up, "m1"))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::Unauthenticated);
}

#[tokio::test]
async fn submit_records_with_server_derived_deterministic_id() {
    let store = std::sync::Arc::new(MemFeedback::default());
    let mut client = spawn_with_party(build_service(Some(store)), "tester").await;

    let resp = client
        .submit_feedback(submit(proto::FeedbackRating::Up, "msg-7"))
        .await
        .unwrap()
        .into_inner();

    // SN-8: the id is server-derived + deterministic over (message_id, principal).
    let mut keyed = Vec::new();
    keyed.extend_from_slice(b"kx-feedback-id\0");
    keyed.extend_from_slice(b"msg-7");
    keyed.push(0);
    keyed.extend_from_slice(b"tester");
    let expect = &ContentRef::of(&keyed).0[..16];
    assert_eq!(resp.feedback_id, expect.to_vec());

    // It lists back, principal omitted from the read row but rating preserved.
    let page = client
        .list_feedback(proto::ListFeedbackRequest {
            limit: None,
            instance_id: None,
            before_rowid: None,
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(page.rows.len(), 1);
    assert_eq!(page.rows[0].message_id, "msg-7");
    assert_eq!(page.rows[0].rating, proto::FeedbackRating::Up as i32);
    assert_eq!(page.rows[0].model_id, "qwen3");
}

#[tokio::test]
async fn re_rating_the_same_answer_overwrites() {
    let store = std::sync::Arc::new(MemFeedback::default());
    let mut client = spawn_with_party(build_service(Some(store)), "tester").await;
    let up = client
        .submit_feedback(submit(proto::FeedbackRating::Up, "m"))
        .await
        .unwrap()
        .into_inner();
    let down = client
        .submit_feedback(submit(proto::FeedbackRating::Down, "m"))
        .await
        .unwrap()
        .into_inner();
    // Same answer + party ⇒ SAME id (overwrite, not a second row).
    assert_eq!(up.feedback_id, down.feedback_id);
    let page = client
        .list_feedback(proto::ListFeedbackRequest {
            limit: None,
            instance_id: None,
            before_rowid: None,
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(page.rows.len(), 1, "the re-rating overwrote");
    assert_eq!(page.rows[0].rating, proto::FeedbackRating::Down as i32);
}

#[tokio::test]
async fn invalid_rating_message_id_and_comment_fail_closed() {
    let store = std::sync::Arc::new(MemFeedback::default());
    let mut client = spawn_with_party(build_service(Some(store)), "tester").await;

    // UNSPECIFIED rating ⇒ invalid.
    let err = client
        .submit_feedback(submit(proto::FeedbackRating::Unspecified, "m"))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::InvalidArgument);

    // Empty message_id ⇒ invalid.
    let err = client
        .submit_feedback(submit(proto::FeedbackRating::Up, ""))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::InvalidArgument);

    // Over-cap comment ⇒ invalid (fail-closed BEFORE the write).
    let mut over = submit(proto::FeedbackRating::Up, "m");
    over.comment = "x".repeat(MAX_FEEDBACK_COMMENT_BYTES + 1);
    let err = client.submit_feedback(over).await.unwrap_err();
    assert_eq!(err.code(), Code::InvalidArgument);
}

#[tokio::test]
async fn malformed_target_ids_are_invalid() {
    let store = std::sync::Arc::new(MemFeedback::default());
    let mut client = spawn_with_party(build_service(Some(store)), "tester").await;
    let mut bad = submit(proto::FeedbackRating::Up, "m");
    bad.instance_id = Some(vec![0u8; 8]); // not 16
    let err = client.submit_feedback(bad).await.unwrap_err();
    assert_eq!(err.code(), Code::InvalidArgument);
}

#[tokio::test]
async fn list_scopes_by_instance() {
    let store = std::sync::Arc::new(MemFeedback::default());
    let mut client = spawn_with_party(build_service(Some(store)), "tester").await;
    let mut a = submit(proto::FeedbackRating::Up, "a");
    a.instance_id = Some(vec![0x01; 16]);
    let mut b = submit(proto::FeedbackRating::Down, "b");
    b.instance_id = Some(vec![0x02; 16]);
    client.submit_feedback(a).await.unwrap();
    client.submit_feedback(b).await.unwrap();
    let page = client
        .list_feedback(proto::ListFeedbackRequest {
            limit: None,
            instance_id: Some(vec![0x01; 16]),
            before_rowid: None,
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(page.rows.len(), 1);
    assert_eq!(page.rows[0].message_id, "a");
}
