//! A2 — `grpc.health.v1.Health` service end-to-end.
//!
//! A running gateway serves the standard health service (alongside `KxGateway`,
//! NOT behind the auth interceptor) and reports SERVING for the overall ("")
//! service — what `kx health`, `grpc_health_probe`, and k8s gRPC probes read.

#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;

use std::time::Duration;

use kx_gateway::start;
use tonic::transport::Channel;
use tonic_health::pb::health_check_response::ServingStatus;
use tonic_health::pb::health_client::HealthClient;
use tonic_health::pb::HealthCheckRequest;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gateway_reports_serving_on_grpc_health_unauthenticated() {
    let dir = tempfile::tempdir().unwrap();
    // Deny-all auth (no dev-allow-local, no tokens): the health service must STILL
    // answer — it is not behind the auth interceptor (a probe is unauthenticated).
    let cfg = common::gateway_config(&dir, false, std::collections::HashMap::new());
    let running = start(cfg).await.expect("start gateway");
    let endpoint = format!("http://{}", running.local_addr());

    // Connect a bare health client (no bearer token), retrying while the serve task
    // finishes binding.
    let mut client = {
        let mut found = None;
        for _ in 0..100 {
            if let Ok(ch) = Channel::from_shared(endpoint.clone())
                .unwrap()
                .connect()
                .await
            {
                found = Some(HealthClient::new(ch));
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        found.expect("connect to the health service")
    };

    let status = client
        .check(HealthCheckRequest {
            service: String::new(),
        })
        .await
        .expect("grpc.health.v1 Check succeeds without auth")
        .into_inner()
        .status();
    assert_eq!(status, ServingStatus::Serving);

    running.shutdown().await.unwrap();
}
