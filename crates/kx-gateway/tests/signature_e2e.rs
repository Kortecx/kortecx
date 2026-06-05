//! End-to-end witnesses for the catalog signature RPCs over a REAL bound port
//! (R2a):
//!
//! - register → the server derives the id → get round-trips the exact manifest →
//!   list enumerates it → re-register is idempotent;
//! - an unknown id → `not_found`, a wrong-length id / malformed manifest →
//!   `invalid_argument` (fail-closed, but a *public* discovery surface — not the
//!   collapsed no-oracle of the execution surface);
//! - the registry is **durable across a restart** (the G1a SQLite backend);
//! - the bearer-token resolver gates the surface (no/!wrong token →
//!   `unauthenticated`; a valid token authorizes).

#![cfg(feature = "embedded-worker")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Duration;

use kx_catalog::{canonical_config, RecipeSnapshot, SignatureEntry, TaskSignature};
use kx_gateway::start;
use kx_proto::proto;
use kx_proto::proto::kx_gateway_client::KxGatewayClient;
use tonic::transport::Channel;
use tonic::Request;

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

/// A manifest = the canonical-encoded `SignatureEntry` for `fp` (a distinct
/// fingerprint → a distinct task signature → a distinct server-derived id).
fn manifest(fp: [u8; 32]) -> Vec<u8> {
    let entry = SignatureEntry::new(
        TaskSignature::model_invariant(kx_mote::MoteDefHash(fp)),
        kx_workflow::ManifestId(fp),
        RecipeSnapshot::new(fp),
    );
    bincode::serde::encode_to_vec(&entry, canonical_config()).unwrap()
}

/// Wrap a payload in a request carrying an `authorization: Bearer <token>` header.
fn with_bearer<T>(payload: T, token: &str) -> Request<T> {
    let mut req = Request::new(payload);
    req.metadata_mut()
        .insert("authorization", format!("Bearer {token}").parse().unwrap());
    req
}

#[tokio::test]
async fn signature_register_get_list_round_trip() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    let m = manifest([7; 32]);
    let reg = c
        .register_signature(proto::RegisterSignatureRequest {
            manifest: m.clone(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(reg.signature_id.len(), 32, "server-derived 32-byte id");

    // GetSignature byte-round-trips the exact manifest.
    let got = c
        .get_signature(proto::GetSignatureRequest {
            signature_id: reg.signature_id.clone(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(got.signature_id, reg.signature_id);
    assert_eq!(
        got.manifest, m,
        "GetSignature byte-round-trips the manifest"
    );

    // ListSignatures enumerates it.
    let list = c
        .list_signatures(proto::ListSignaturesRequest {})
        .await
        .unwrap()
        .into_inner();
    assert!(list
        .signatures
        .iter()
        .any(|s| s.signature_id == reg.signature_id));
    assert!(list.signatures.iter().all(|s| !s.name.is_empty()));

    // Re-register the identical manifest → the same id (idempotent).
    let reg2 = c
        .register_signature(proto::RegisterSignatureRequest { manifest: m })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(reg2.signature_id, reg.signature_id);

    running.shutdown().await.unwrap();
}

#[tokio::test]
async fn get_unknown_is_not_found_and_malformed_is_invalid_argument() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    let unknown = c
        .get_signature(proto::GetSignatureRequest {
            signature_id: vec![0xab; 32],
        })
        .await;
    assert_eq!(unknown.unwrap_err().code(), tonic::Code::NotFound);

    let badlen = c
        .get_signature(proto::GetSignatureRequest {
            signature_id: vec![0u8; 5],
        })
        .await;
    assert_eq!(badlen.unwrap_err().code(), tonic::Code::InvalidArgument);

    let malformed = c
        .register_signature(proto::RegisterSignatureRequest {
            manifest: b"not a signature entry".to_vec(),
        })
        .await;
    assert_eq!(malformed.unwrap_err().code(), tonic::Code::InvalidArgument);

    running.shutdown().await.unwrap();
}

#[tokio::test]
async fn registered_signature_survives_restart() {
    let dir = tempfile::TempDir::new().unwrap();
    let m = manifest([9; 32]);

    // First server: register, then shut down gracefully.
    let id = {
        let running = start(common::gateway_config(&dir, true, HashMap::new()))
            .await
            .unwrap();
        let mut c = client(running.local_addr()).await;
        let reg = c
            .register_signature(proto::RegisterSignatureRequest {
                manifest: m.clone(),
            })
            .await
            .unwrap()
            .into_inner();
        running.shutdown().await.unwrap();
        reg.signature_id
    };

    // Fresh server on the SAME catalog dir (default = the journal's dir): the
    // durable SQLite registry re-serves the signature.
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;
    let got = c
        .get_signature(proto::GetSignatureRequest {
            signature_id: id.clone(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        got.manifest, m,
        "the registered signature survives a restart"
    );
    running.shutdown().await.unwrap();
}

#[tokio::test]
async fn bearer_token_gates_the_signature_surface() {
    let dir = tempfile::TempDir::new().unwrap();
    let mut tokens = HashMap::new();
    tokens.insert("s3cr3t".to_string(), "alice@acme".to_string());
    let running = start(common::gateway_config(&dir, false, tokens))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    // No credential → unauthenticated.
    let no_tok = c.list_signatures(proto::ListSignaturesRequest {}).await;
    assert_eq!(no_tok.unwrap_err().code(), tonic::Code::Unauthenticated);

    // Wrong credential → unauthenticated (indistinguishable from no credential).
    let wrong = c
        .list_signatures(with_bearer(proto::ListSignaturesRequest {}, "nope"))
        .await;
    assert_eq!(wrong.unwrap_err().code(), tonic::Code::Unauthenticated);

    // Valid credential → authorized.
    let reg = c
        .register_signature(with_bearer(
            proto::RegisterSignatureRequest {
                manifest: manifest([3; 32]),
            },
            "s3cr3t",
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(reg.signature_id.len(), 32);

    running.shutdown().await.unwrap();
}
