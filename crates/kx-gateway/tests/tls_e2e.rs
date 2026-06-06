//! A1 — TLS handshake end-to-end.
//!
//! Proves the in-binary rustls TLS path: a gateway started with `--tls-cert`/
//! `--tls-key` (a self-signed cert generated in-test by `rcgen`) accepts a TLS
//! client that trusts that CA and round-trips a real RPC; and that a PLAINTEXT
//! client to the TLS port fails (no silent downgrade). Mirrors `serve_e2e.rs`.

#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;

use std::time::Duration;

use kx_gateway::{start, TlsPaths};
use kx_proto::proto;
use kx_proto::proto::kx_gateway_client::KxGatewayClient;
use tonic::transport::{Certificate, ClientTlsConfig, Endpoint};

/// Generate a self-signed cert (SAN `localhost` + `127.0.0.1`), write the PEMs to
/// `dir`, and return `(TlsPaths, cert_pem_bytes)` (the cert doubles as the CA the
/// client trusts).
fn self_signed(dir: &std::path::Path) -> (TlsPaths, Vec<u8>) {
    let certified =
        rcgen::generate_simple_self_signed(vec!["localhost".to_string(), "127.0.0.1".to_string()])
            .expect("generate self-signed cert");
    let cert_pem = certified.cert.pem();
    let key_pem = certified.key_pair.serialize_pem();
    let cert_path = dir.join("cert.pem");
    let key_path = dir.join("key.pem");
    std::fs::write(&cert_path, &cert_pem).unwrap();
    std::fs::write(&key_path, &key_pem).unwrap();
    (
        TlsPaths {
            cert_path,
            key_path,
        },
        cert_pem.into_bytes(),
    )
}

/// A TLS client trusting `ca_pem`, dialing `https://localhost:<port>`.
async fn tls_client(port: u16, ca_pem: &[u8]) -> KxGatewayClient<tonic::transport::Channel> {
    let tls = ClientTlsConfig::new()
        .ca_certificate(Certificate::from_pem(ca_pem))
        .domain_name("localhost");
    let endpoint = Endpoint::from_shared(format!("https://localhost:{port}"))
        .unwrap()
        .tls_config(tls)
        .unwrap();
    // Retry briefly while the serve task finishes binding.
    for _ in 0..100 {
        if let Ok(ch) = endpoint.connect().await {
            return KxGatewayClient::new(ch);
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("TLS client could not connect on :{port}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tls_gateway_accepts_a_trusting_client_and_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let (tls, ca_pem) = self_signed(dir.path());
    // dev-allow-local (loopback bind) + TLS: no token needed for the round-trip.
    let mut cfg = common::gateway_config(&dir, true, std::collections::HashMap::new());
    cfg.tls = Some(tls);
    let running = start(cfg).await.expect("start TLS gateway");
    let port = running.local_addr().port();

    let mut client = tls_client(port, &ca_pem).await;
    // A read RPC over TLS: ListSignatures takes no instance id and exercises the
    // full TLS handshake -> auth -> RPC -> response path.
    let resp = client
        .list_signatures(proto::ListSignaturesRequest {})
        .await
        .expect("ListSignatures over TLS succeeds");
    // A fresh catalog is empty; the point is the RPC completed over TLS.
    assert!(resp.into_inner().signatures.is_empty());

    // A real submit RPC over TLS: the gateway proxies to the embedded coordinator
    // and returns a 16-byte server-derived instance id (the work path over TLS).
    let instance_id = common::submit_pure_run(&mut client, 7).await;
    assert_eq!(
        instance_id.len(),
        16,
        "submit over TLS returns a 16-byte instance id"
    );

    running.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn plaintext_client_to_tls_gateway_fails() {
    let dir = tempfile::tempdir().unwrap();
    let (tls, _ca) = self_signed(dir.path());
    let mut cfg = common::gateway_config(&dir, true, std::collections::HashMap::new());
    cfg.tls = Some(tls);
    let running = start(cfg).await.expect("start TLS gateway");
    let port = running.local_addr().port();

    // A PLAINTEXT (http://) client to the TLS port must NOT succeed at an RPC —
    // either the connect or the first request fails (no silent downgrade).
    let plaintext = format!("http://127.0.0.1:{port}");
    let outcome = async {
        let mut c = KxGatewayClient::connect(plaintext).await?;
        c.list_signatures(proto::ListSignaturesRequest {}).await?;
        Ok::<(), Box<dyn std::error::Error>>(())
    }
    .await;
    assert!(
        outcome.is_err(),
        "a plaintext client must not complete an RPC against a TLS gateway"
    );

    running.shutdown().await.unwrap();
}
