//! D213 Experience lane — the REAL hosted-app path (live witness): a hosted app is
//! materialized to a real Vite-React project tree, `npm install`ed, and served by a real
//! `vite` dev server, then hit over HTTP for a 200. Unlike the hermetic
//! `hosted_supervisor_e2e` (a std-only fake server, no Node), this proves a GENERATED app
//! genuinely BUILDS + RUNS — so it is `#[ignore]` (needs Node/npm) + gated on `hosted-apps`.
//!
//! Run locally: `cargo test -p kx-gateway --features hosted-apps hosted_app_serves_real_vite -- --ignored --nocapture`
//! (first run downloads the npm deps; allow a couple of minutes).

#![cfg(feature = "hosted-apps")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use kx_gateway::start;
use kx_proto::proto;
use kx_proto::proto::kx_gateway_client::KxGatewayClient;
use tonic::transport::Channel;

mod common;

async fn client(addr: SocketAddr) -> KxGatewayClient<Channel> {
    common::connect_client(addr).await
}

fn http_get(port: u16) -> std::io::Result<String> {
    let mut stream = TcpStream::connect(("127.0.0.1", port))?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.write_all(b"GET / HTTP/1.0\r\nHost: localhost\r\nConnection: close\r\n\r\n")?;
    let mut buf = String::new();
    let _ = stream.read_to_string(&mut buf);
    Ok(buf)
}

#[tokio::test]
#[ignore = "needs Node/npm — real Vite install + dev server"]
async fn hosted_app_serves_real_vite() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    // A real hosted (experience) app — Vite-React, no command overrides ⇒ the supervisor
    // materializes the framework template, runs the REAL `npm install`, and starts `vite`.
    let envelope = kx_app::AppEnvelope::new_experience(
        "landing",
        kx_app::HostedConfig {
            framework: kx_app::HostedFramework::ViteReact,
            ..Default::default()
        },
        "team/apps/landing",
    )
    .to_canonical_json()
    .unwrap();
    c.save_app(proto::SaveAppRequest {
        handle: "team/apps/landing".into(),
        envelope_json: envelope,
        source_digest: Vec::new(),
    })
    .await
    .expect("save the hosted app")
    .into_inner();

    c.start_hosted_app(proto::StartHostedAppRequest {
        handle: "team/apps/landing".into(),
        rebuild: false,
    })
    .await
    .expect("start the hosted app");

    // Poll to Running — the first run installs deps + boots Vite, so allow generous time.
    let mut port = 0u32;
    for _ in 0..600 {
        let st = c
            .get_hosted_app_status(proto::GetHostedAppStatusRequest {
                handle: "team/apps/landing".into(),
            })
            .await
            .expect("status")
            .into_inner();
        if st.state == proto::HostedAppState::HostedRunning as i32 {
            port = st.port;
            break;
        }
        assert_ne!(
            st.state,
            proto::HostedAppState::HostedFailed as i32,
            "hosted app failed: {}\n{}",
            st.detail,
            st.recent_logs.join("\n")
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    assert!(port > 0, "the real Vite dev server reached Running");

    // The real Vite server serves the app's index.html (HTTP 200).
    let port_u16 = u16::try_from(port).unwrap();
    let resp = tokio::task::spawn_blocking(move || http_get(port_u16))
        .await
        .unwrap()
        .expect("the Vite dev server accepts a connection");
    assert!(resp.contains("200"), "served a 200: {resp:?}");
    assert!(
        resp.contains("<div id=\"root\">"),
        "served the Vite index: {resp:?}"
    );

    let stopped = c
        .stop_hosted_app(proto::StopHostedAppRequest {
            handle: "team/apps/landing".into(),
        })
        .await
        .expect("stop")
        .into_inner();
    assert!(stopped.stopped, "the running Vite server was stopped");
}
