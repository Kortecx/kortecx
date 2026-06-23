//! POC-2 context-edit end-to-end over a REAL bound tonic port. The edit family is
//! a pure CLIENT compose over EXISTING RPCs — `GetContextBundle` → `PutContent`
//! (new bytes ⇒ a new server-derived ref, since CAS is immutable) → re-upsert via
//! `PutContextBundle`. There is no edit RPC and no journal write, so these prove
//! the server-side primitives every surface (CLI / SDK / UI) composes:
//!
//! - **edit round trip**: re-upsert re-points the item at the new ref; the OLD ref
//!   still resolves (CAS is immutable), the description survives, the `bundle_ref`
//!   (a content hash of the manifest) moves.
//! - **dedup**: editing back to identical bytes re-reports `deduplicated`.
//! - **empty-manifest refusal**: removing the last item (an empty items list) is
//!   refused — the UI/CLI/SDK "use delete to unbind" guard's server backing.
//! - **security negatives** (two auth-token parties): cross-party bundle isolation
//!   (uniform not-found; a same-handle put makes the OTHER party's own row, never
//!   mutates the author's); fail-closed unknown-ref read; server-derived identity;
//!   and a PINNED witness of the pre-existing uploads-scope cross-party read gap
//!   (ticket T-UPLOADS-PRINCIPAL-SCOPE → PR-8) so a future fix flips one assertion.

#![cfg(feature = "embedded-worker")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::HashMap;
use std::net::SocketAddr;

use kx_gateway::start;
use kx_proto::proto;
use kx_proto::proto::kx_gateway_client::KxGatewayClient;
use tonic::transport::Channel;
use tonic::{Code, Request};

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

/// `PutContent` a payload in the dev-local (no-bearer) scope; return the ref.
async fn put(c: &mut KxGatewayClient<Channel>, payload: &[u8]) -> Vec<u8> {
    c.put_content(proto::PutContentRequest {
        payload: payload.to_vec(),
        media_type: "text/plain".into(),
        filename: "item.txt".into(),
    })
    .await
    .unwrap()
    .into_inner()
    .content_ref
}

fn item(name: &str, content_ref: Vec<u8>) -> proto::ContextItem {
    proto::ContextItem {
        name: name.into(),
        content_ref,
        media_type: "text/plain".into(),
    }
}

async fn upsert(
    c: &mut KxGatewayClient<Channel>,
    handle: &str,
    description: &str,
    items: Vec<proto::ContextItem>,
) -> proto::PutContextBundleResponse {
    c.put_context_bundle(proto::PutContextBundleRequest {
        handle: handle.into(),
        description: description.into(),
        items,
    })
    .await
    .unwrap()
    .into_inner()
}

/// `GetContent` a ref in the dev-local uploads scope; return the full payload.
async fn read_uploads(c: &mut KxGatewayClient<Channel>, r: Vec<u8>) -> Vec<u8> {
    c.get_content(proto::GetContentRequest {
        content_ref: r,
        instance_id: Vec::new(),
    })
    .await
    .unwrap()
    .into_inner()
    .payload
}

async fn get_bundle(c: &mut KxGatewayClient<Channel>, handle: &str) -> proto::ContextBundle {
    c.get_context_bundle(proto::GetContextBundleRequest {
        handle: handle.into(),
    })
    .await
    .unwrap()
    .into_inner()
    .bundle
    .expect("bundle present")
}

#[tokio::test]
async fn edit_round_trip_repoints_the_item_and_keeps_the_old_ref() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    // Author a two-item bundle.
    let a = put(&mut c, b"alpha").await;
    let b = put(&mut c, b"beta").await;
    let put0 = upsert(
        &mut c,
        "team/ctx/docs",
        "the docs",
        vec![item("a", a.clone()), item("b", b.clone())],
    )
    .await;

    // EDIT item "a": upload new bytes (a NEW ref) and re-upsert with it re-pointed.
    let a2 = put(&mut c, b"ALPHA-v2").await;
    assert_ne!(a2, a, "immutable CAS: new bytes ⇒ a new server-derived ref");
    let bundle = get_bundle(&mut c, "team/ctx/docs").await;
    let mut items = bundle.items.clone();
    items[0].content_ref = a2.clone();
    let put1 = upsert(&mut c, "team/ctx/docs", &bundle.description, items).await;

    // The manifest changed (its content hash moved) but the description survived.
    assert_ne!(put1.bundle_ref, put0.bundle_ref);
    let after = get_bundle(&mut c, "team/ctx/docs").await;
    assert_eq!(after.description, "the docs");
    assert_eq!(after.items[0].name, "a", "the advisory name is preserved");
    assert_eq!(after.items[0].content_ref, a2, "item re-pointed to new ref");
    assert_eq!(after.items[1].content_ref, b, "the other item is untouched");

    // Both the new AND the old ref still resolve (CAS is immutable — no in-place
    // mutation, the old bytes are not lost).
    assert_eq!(read_uploads(&mut c, a2).await, b"ALPHA-v2");
    assert_eq!(
        read_uploads(&mut c, a).await,
        b"alpha",
        "the old ref is still intact"
    );

    running.shutdown().await.unwrap();
}

#[tokio::test]
async fn dedup_on_edit_back_to_identical_bytes() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    let a = put(&mut c, b"alpha").await;
    upsert(&mut c, "team/ctx/d", "", vec![item("a", a.clone())]).await;
    // Re-uploading identical bytes is a content-store dedup hit (same ref).
    let again = c
        .put_content(proto::PutContentRequest {
            payload: b"alpha".to_vec(),
            media_type: "text/plain".into(),
            filename: "item.txt".into(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(again.content_ref, a);
    assert!(
        again.deduplicated,
        "identical bytes ⇒ dedup at the content layer"
    );
    // Re-upserting the identical manifest is also a dedup (same bundle_ref).
    let re = upsert(&mut c, "team/ctx/d", "", vec![item("a", a)]).await;
    assert!(
        re.deduplicated,
        "identical manifest ⇒ dedup at the bundle layer"
    );

    running.shutdown().await.unwrap();
}

#[tokio::test]
async fn removing_the_last_item_empties_the_manifest_and_is_refused() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    let a = put(&mut c, b"alpha").await;
    upsert(&mut c, "team/ctx/d", "", vec![item("a", a)]).await;
    // An empty items list (the remove-last-item case) is refused server-side —
    // this is what the CLI/SDK/UI "use delete to unbind the handle" guard backs.
    let err = c
        .put_context_bundle(proto::PutContextBundleRequest {
            handle: "team/ctx/d".into(),
            description: String::new(),
            items: Vec::new(),
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::InvalidArgument);

    running.shutdown().await.unwrap();
}

#[tokio::test]
async fn cross_party_bundle_isolation_and_server_derived_identity() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, false, two_party_tokens()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    // Alice uploads + authors a bundle.
    let a = c
        .put_content(with_bearer(
            proto::PutContentRequest {
                payload: b"alice secret".to_vec(),
                media_type: "text/plain".into(),
                filename: "a.txt".into(),
            },
            "tok-alice",
        ))
        .await
        .unwrap()
        .into_inner()
        .content_ref;
    let alice_put = c
        .put_context_bundle(with_bearer(
            proto::PutContextBundleRequest {
                handle: "team/ctx/secret".into(),
                description: "alice".into(),
                items: vec![item("a", a.clone())],
            },
            "tok-alice",
        ))
        .await
        .unwrap()
        .into_inner();

    // Bob cannot see Alice's bundle (uniform not-found — no cross-party oracle).
    let bob_get = c
        .get_context_bundle(with_bearer(
            proto::GetContextBundleRequest {
                handle: "team/ctx/secret".into(),
            },
            "tok-bob",
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(!bob_get.found, "Bob cannot read Alice's bundle");
    let bob_list = c
        .list_context_bundles(with_bearer(
            proto::ListContextBundlesRequest {
                limit: 0,
                after_handle: String::new(),
            },
            "tok-bob",
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(bob_list.bundles.is_empty(), "Bob lists none of Alice's");

    // Bob putting the SAME (handle, items) makes BOB's OWN row — server-derived
    // identity means the same manifest yields the same bundle_ref, but it is a
    // DIFFERENT principal-scoped row; Alice's bundle is never mutated.
    let bob_put = c
        .put_context_bundle(with_bearer(
            proto::PutContextBundleRequest {
                handle: "team/ctx/secret".into(),
                description: "alice".into(),
                items: vec![item("a", a)],
            },
            "tok-bob",
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        bob_put.bundle_ref, alice_put.bundle_ref,
        "same manifest ⇒ same server-derived bundle_ref (SN-8; client cannot forge it)"
    );
    // Alice's bundle is unchanged + still hers.
    let alice_after = c
        .get_context_bundle(with_bearer(
            proto::GetContextBundleRequest {
                handle: "team/ctx/secret".into(),
            },
            "tok-alice",
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(alice_after.found);
    assert_eq!(alice_after.bundle.unwrap().bundle_ref, alice_put.bundle_ref);

    running.shutdown().await.unwrap();
}

#[tokio::test]
async fn unknown_uploads_ref_read_is_fail_closed() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, false, two_party_tokens()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    // A never-uploaded ref read in the uploads scope is a uniform NotAuthorized
    // (no existence oracle) — the fail-closed default the edit/view path inherits.
    let err = c
        .get_content(with_bearer(
            proto::GetContentRequest {
                content_ref: vec![0x33; 32],
                instance_id: Vec::new(),
            },
            "tok-alice",
        ))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::PermissionDenied);

    running.shutdown().await.unwrap();
}

/// ★ SECURITY GAP (ticket **T-UPLOADS-PRINCIPAL-SCOPE** → PR-8): the uploads-scope
/// read authorizes by content-ref ALONE — `UploadsLedger::contains` records the
/// `principal` but does NOT check it (see `kx-gateway-core::view::get_uploaded_content`).
/// So a KNOWN ref is readable cross-party. Bundles stay principal-scoped, so refs
/// are not cross-party *discoverable* in normal flows, but this pins the current
/// behavior as a tracked, test-visible fact. **PR-8 flips this one assertion to
/// `Code::PermissionDenied` when `contains` becomes principal-scoped.**
#[tokio::test]
async fn cross_party_uploaded_ref_read_is_currently_allowed_gap_pr8() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, false, two_party_tokens()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    // Alice uploads bytes → ref R.
    let r = c
        .put_content(with_bearer(
            proto::PutContentRequest {
                payload: b"alice-only bytes".to_vec(),
                media_type: "text/plain".into(),
                filename: "a.txt".into(),
            },
            "tok-alice",
        ))
        .await
        .unwrap()
        .into_inner()
        .content_ref;

    // Bob reads R in the uploads scope. TODAY this SUCCEEDS (the gap). When PR-8
    // makes `UploadsLedger::contains` principal-scoped, flip the expectation to a
    // uniform PermissionDenied (and assert Alice can still read her own ref).
    let bob = c
        .get_content(with_bearer(
            proto::GetContentRequest {
                content_ref: r,
                instance_id: Vec::new(),
            },
            "tok-bob",
        ))
        .await;
    assert!(
        bob.is_ok(),
        "PINNED GAP (T-UPLOADS-PRINCIPAL-SCOPE/PR-8): cross-party uploaded-ref read \
         is currently ALLOWED; PR-8 flips this to PermissionDenied"
    );
    assert_eq!(bob.unwrap().into_inner().payload, b"alice-only bytes");

    running.shutdown().await.unwrap();
}
