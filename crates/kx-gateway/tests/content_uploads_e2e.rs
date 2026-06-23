//! Batch A end-to-end over a REAL bound tonic port — the GR8 proofs for the
//! FIRST client write path to the content store:
//!
//! - **no-journal-write**: a PutContent burst moves the journal head by ZERO
//!   (the upload is a content-store write, never a journal write — the digest
//!   cannot move by construction).
//! - **size-cap fail-closed over the transport**: cap+1 ⇒ `RESOURCE_EXHAUSTED`
//!   (the handler's honest refusal, not a transport mangle), and an 8 MiB
//!   payload — over tonic's 4 MiB DEFAULT decode limit — lands (the raised
//!   decode limit, the highest-likelihood field bug).
//! - **auth**: deny-all (no `--dev-allow-local`) refuses PutContent outright.
//! - **no existence oracle**: never-existed vs exists-but-out-of-scope refs are
//!   INDISTINGUISHABLE on both the single and the batch path (D120.1).
//! - **durability**: the uploads sidecar survives a restart (the scope is
//!   re-served; identical bytes re-report `deduplicated`).
//! - **scale envelope**: a full 64-ref batch at the 512 KiB per-item clamp
//!   (a ~32 MiB response) completes through the transport.

#![cfg(feature = "embedded-worker")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Duration;

use kx_content::ContentRef;
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
            // Mirror the server's raised decode limit on the CLIENT side too
            // (the batch response can reach ~32 MiB; tonic clients also default
            // to 4 MiB).
            return c.max_decoding_message_size(64 * 1024 * 1024);
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("client connects to the gateway at {endpoint}");
}

fn put_req(payload: Vec<u8>) -> proto::PutContentRequest {
    proto::PutContentRequest {
        payload,
        media_type: "application/octet-stream".into(),
        filename: "blob.bin".into(),
    }
}

#[tokio::test]
async fn put_content_round_trips_with_zero_journal_writes() {
    let dir = TempDir::new().unwrap();
    let journal_path = dir.path().join("kx.db");
    let running = start(config(&dir, true)).await.unwrap();
    let mut c = client(running.local_addr()).await;

    // The journal head BEFORE the upload burst (a second read handle — WAL
    // SQLite serves concurrent readers).
    let head_before = SqliteJournal::open(&journal_path)
        .unwrap()
        .current_seq()
        .unwrap();

    let payload = b"the first client-written blob".to_vec();
    let put = c
        .put_content(put_req(payload.clone()))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        put.content_ref,
        ContentRef::of(&payload).0.to_vec(),
        "the ref is SERVER-derived blake3 (SN-8)"
    );
    assert!(!put.deduplicated);

    // Identical bytes again: same ref, dedup flagged.
    let again = c
        .put_content(put_req(payload.clone()))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(again.content_ref, put.content_ref);
    assert!(again.deduplicated);

    // The uploads scope serves it (EMPTY instance_id).
    let blob = c
        .get_content(proto::GetContentRequest {
            content_ref: put.content_ref.clone(),
            instance_id: Vec::new(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(blob.payload, payload);

    // THE no-journal-write proof: the burst moved the head by ZERO entries.
    let head_after = SqliteJournal::open(&journal_path)
        .unwrap()
        .current_seq()
        .unwrap();
    assert_eq!(
        head_before, head_after,
        "PutContent must never write the journal (digest-invariant by construction)"
    );

    running.shutdown().await.unwrap();
}

#[tokio::test]
async fn size_cap_fails_closed_and_the_decode_limit_is_raised() {
    let dir = TempDir::new().unwrap();
    let mut cfg = config(&dir, true);
    cfg.content_max_bytes = 10 * 1024 * 1024; // 10 MiB cap for the test
    let running = start(cfg).await.unwrap();
    let mut c = client(running.local_addr()).await;

    // 8 MiB: OVER tonic's 4 MiB default decode limit, UNDER the cap — must land
    // (the raised transport limit is the point; this is the field-bug witness).
    let eight_mib = vec![0xA5u8; 8 * 1024 * 1024];
    let put = c
        .put_content(put_req(eight_mib.clone()))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(put.size, eight_mib.len() as u64);

    // Cap+1: the handler's HONEST refusal (RESOURCE_EXHAUSTED), and the blob
    // must not be readable afterwards (it never touched the store).
    let over = vec![0x5Au8; 10 * 1024 * 1024 + 1];
    let over_ref = ContentRef::of(&over).0.to_vec();
    let err = c.put_content(put_req(over)).await.unwrap_err();
    assert_eq!(err.code(), Code::ResourceExhausted);
    let denied = c
        .get_content(proto::GetContentRequest {
            content_ref: over_ref,
            instance_id: Vec::new(),
        })
        .await
        .unwrap_err();
    assert_eq!(denied.code(), Code::PermissionDenied);

    running.shutdown().await.unwrap();
}

#[tokio::test]
async fn deny_all_refuses_put_content() {
    let dir = TempDir::new().unwrap();
    let running = start(config(&dir, false)).await.unwrap(); // NO dev flag ⇒ deny-all
    let mut c = client(running.local_addr()).await;

    let err = c.put_content(put_req(b"nope".to_vec())).await.unwrap_err();
    assert!(
        matches!(err.code(), Code::Unauthenticated | Code::PermissionDenied),
        "an unauthenticated upload must be refused, got {:?}",
        err.code()
    );
    running.shutdown().await.unwrap();
}

#[tokio::test]
async fn unauthorized_and_missing_refs_are_indistinguishable() {
    let dir = TempDir::new().unwrap();
    let running = start(config(&dir, true)).await.unwrap();
    let mut c = client(running.local_addr()).await;

    // Establish a COMMITTED run so one ref EXISTS in the store but is outside
    // the uploads scope.
    let handle = c
        .submit_run(kx_gateway::pure_run_request())
        .await
        .unwrap()
        .into_inner();
    let (_mote, committed_ref) = await_committed(&mut c, &handle.instance_id).await;

    // Single GetContent, uploads scope: never-existed vs exists-but-out-of-scope
    // produce the IDENTICAL status + message.
    let never = c
        .get_content(proto::GetContentRequest {
            content_ref: vec![0x77; 32],
            instance_id: Vec::new(),
        })
        .await
        .unwrap_err();
    let exists = c
        .get_content(proto::GetContentRequest {
            content_ref: committed_ref.to_vec(),
            instance_id: Vec::new(),
        })
        .await
        .unwrap_err();
    assert_eq!(never.code(), exists.code());
    assert_eq!(never.message(), exists.message());

    // Batch, run scope: a never-existed ref and an uploaded-but-not-this-run's
    // ref yield byte-identical UNIFORM empty items.
    let uploaded = c
        .put_content(put_req(b"mine, not the run's".to_vec()))
        .await
        .unwrap()
        .into_inner();
    let batch = c
        .get_content_batch(proto::GetContentBatchRequest {
            instance_id: handle.instance_id.clone(),
            content_refs: vec![
                vec![0x77; 32],         // never existed
                uploaded.content_ref,   // exists (uploads scope), not this run's
                committed_ref.to_vec(), // the run's own committed result
            ],
            max_bytes_per_item: None,
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(batch.items.len(), 3);
    let (a, b, owned) = (&batch.items[0], &batch.items[1], &batch.items[2]);
    assert!(a.payload.is_empty() && a.full_size == 0);
    assert!(b.payload.is_empty() && b.full_size == 0);
    assert_eq!(
        (a.payload.clone(), a.full_size, a.truncated),
        (b.payload.clone(), b.full_size, b.truncated),
        "no existence oracle: the two denials are indistinguishable"
    );
    assert!(!owned.payload.is_empty(), "the run's own ref resolves");

    running.shutdown().await.unwrap();
}

#[tokio::test]
async fn uploads_scope_survives_a_restart() {
    let dir = TempDir::new().unwrap();
    let payload = b"durable upload".to_vec();
    let put_ref;
    {
        let running = start(config(&dir, true)).await.unwrap();
        let mut c = client(running.local_addr()).await;
        put_ref = c
            .put_content(put_req(payload.clone()))
            .await
            .unwrap()
            .into_inner()
            .content_ref;
        running.shutdown().await.unwrap();
    }
    let running = start(config(&dir, true)).await.unwrap();
    let mut c = client(running.local_addr()).await;
    let blob = c
        .get_content(proto::GetContentRequest {
            content_ref: put_ref.clone(),
            instance_id: Vec::new(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        blob.payload, payload,
        "the uploads sidecar survives a restart"
    );
    // Identical bytes after the restart: the store still dedups at the same ref.
    let again = c.put_content(put_req(payload)).await.unwrap().into_inner();
    assert_eq!(again.content_ref, put_ref);
    assert!(again.deduplicated);
    running.shutdown().await.unwrap();
}

#[tokio::test]
async fn list_models_is_an_honest_empty_list_on_an_ffi_free_serve() {
    let dir = TempDir::new().unwrap();
    let running = start(config(&dir, true)).await.unwrap();
    let mut c = client(running.local_addr()).await;
    let resp = c
        .list_models(proto::ListModelsRequest {})
        .await
        .unwrap()
        .into_inner();
    assert!(
        resp.models.is_empty(),
        "no model on an FFI-free serve — an EMPTY list, not unimplemented"
    );
    running.shutdown().await.unwrap();
}

/// POC-1 Settings "Workspace": `GetServerInfo` projects the NON-SECRET config to an
/// AUTHENTICATED caller. The dev-local serve resolves a caller (the CallerParty gate
/// passes, like the other read RPCs); the response carries the real dirs + a POSTURE
/// LABEL, and NEVER a secret — the `auth_mode` field is a label such as `dev-local`
/// (not a bearer-token value), and the response type has NO token/TLS-key field to
/// leak (the token-never-leaks negative is type-level).
#[tokio::test]
async fn get_server_info_projects_non_secret_config_to_an_authed_caller() {
    let dir = TempDir::new().unwrap();
    let running = start(config(&dir, true)).await.unwrap();
    let mut c = client(running.local_addr()).await;
    let info = c
        .get_server_info(proto::GetServerInfoRequest {})
        .await
        .unwrap()
        .into_inner();
    // Real config facts are projected.
    assert!(
        !info.content_root.is_empty(),
        "the content root is projected"
    );
    assert!(
        !info.journal_path.is_empty(),
        "the journal path is projected"
    );
    assert!(info.max_lease > 0, "the lease batch size is projected");
    assert!(
        info.listen_addr.contains("127.0.0.1"),
        "the loopback gRPC bind is projected"
    );
    // The auth POSTURE is a LABEL — never a secret. `dev_allow_local` ⇒ `dev-local`.
    assert_eq!(
        info.auth_mode, "dev-local",
        "the auth posture is a LABEL, never a bearer-token value"
    );
    assert!(!info.tls_enabled, "a plaintext loopback serve");
    running.shutdown().await.unwrap();
}

/// The stated scale envelope: a FULL 64-ref batch whose every item sits at the
/// 512 KiB per-item clamp — a ~32 MiB response — completes through the
/// transport, order-preserved, with honest truncation metadata.
#[tokio::test]
async fn full_batch_at_the_item_clamp_fits_the_transport() {
    const CLAMP: usize = 512 * 1024;
    let dir = TempDir::new().unwrap();
    let running = start(config(&dir, true)).await.unwrap();
    let mut c = client(running.local_addr()).await;

    let mut refs = Vec::with_capacity(64);
    for i in 0..64u32 {
        // Distinct payloads, each 1 KiB OVER the clamp (so every item truncates).
        let mut payload = vec![0u8; CLAMP + 1024];
        payload[..4].copy_from_slice(&i.to_le_bytes());
        let put = c.put_content(put_req(payload)).await.unwrap().into_inner();
        refs.push(put.content_ref);
    }

    let resp = c
        .get_content_batch(proto::GetContentBatchRequest {
            instance_id: Vec::new(),
            content_refs: refs.clone(),
            max_bytes_per_item: None,
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(resp.items.len(), 64);
    for (i, item) in resp.items.iter().enumerate() {
        assert_eq!(item.content_ref, refs[i], "request order preserved");
        assert_eq!(item.payload.len(), CLAMP, "clamped at the server bound");
        assert!(item.truncated);
        assert_eq!(
            item.full_size as usize,
            CLAMP + 1024,
            "full_size stays honest"
        );
    }

    // One over the ref cap: refused outright, O(1) (no store touch).
    let mut over = refs;
    over.push(vec![0x66; 32]);
    let err = c
        .get_content_batch(proto::GetContentBatchRequest {
            instance_id: Vec::new(),
            content_refs: over,
            max_bytes_per_item: None,
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::InvalidArgument);

    running.shutdown().await.unwrap();
}

/// Poll `GetProjection` until the run's single Mote is `Committed`.
async fn await_committed(
    client: &mut KxGatewayClient<Channel>,
    instance_id: &[u8],
) -> ([u8; 32], [u8; 32]) {
    for _ in 0..100 {
        let view = client
            .get_projection(proto::GetProjectionRequest {
                instance_id: instance_id.to_vec(),
                at_seq: None,
            })
            .await
            .unwrap()
            .into_inner();
        if let Some(m) = view
            .motes
            .iter()
            .find(|m| m.state == proto::MoteSnapshotState::Committed as i32)
        {
            let mote_id: [u8; 32] = m.mote_id.clone().try_into().unwrap();
            let result_ref: [u8; 32] = m
                .result_ref
                .clone()
                .expect("a committed Mote carries a result_ref")
                .try_into()
                .unwrap();
            return (mote_id, result_ref);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the submitted Mote never reached Committed");
}
