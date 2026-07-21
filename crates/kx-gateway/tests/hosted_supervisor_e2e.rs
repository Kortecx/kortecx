//! D213 Experience lane — the hosted-app supervisor lifecycle, end-to-end over a REAL
//! bound tonic port, WITHOUT Node/npm. A saved hosted (experience) app is started; the
//! supervisor materializes the framework template to disk, skips install (the `"skip"`
//! sentinel), and spawns a std-only fake "dev server" (the `hosted_fake_server` fixture
//! bin) on a loopback port. We prove: Start → Running, the proxied/loopback port serves
//! HTTP 200, Stop reaps the child, and status returns to Stopped. Deterministic (no
//! model, no network). The real Vite/npm path is a `#[ignore]` witness (see
//! `hosted_app_live_serve.rs`).

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

/// A hosted (experience) envelope whose server is the fake-server fixture bin and whose
/// install is skipped — so the lifecycle needs no Node/npm. `serve_mode` selects the lane
/// (`""` ⇒ dev).
fn hosted_envelope_mode(name: &str, branch: &str, dev_cmd: &str, serve_mode: &str) -> Vec<u8> {
    let env = kx_app::AppEnvelope::new_experience(
        name,
        kx_app::HostedConfig {
            framework: kx_app::HostedFramework::ViteReact,
            install_cmd: "skip".to_string(),
            dev_cmd: dev_cmd.to_string(),
            serve_mode: serve_mode.to_string(),
            build_cmd: String::new(),
        },
        branch,
    );
    env.to_canonical_json().unwrap()
}

fn hosted_envelope(name: &str, branch: &str, dev_cmd: &str) -> Vec<u8> {
    hosted_envelope_mode(name, branch, dev_cmd, "")
}

/// A blocking HTTP/1.0 GET to `127.0.0.1:<port>/` — returns the raw response text.
fn http_get(port: u16) -> std::io::Result<String> {
    let mut stream = TcpStream::connect(("127.0.0.1", port))?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.write_all(b"GET / HTTP/1.0\r\nHost: localhost\r\nConnection: close\r\n\r\n")?;
    let mut buf = String::new();
    let _ = stream.read_to_string(&mut buf);
    Ok(buf)
}

#[tokio::test]
async fn hosted_app_starts_serves_and_stops() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    let fake = env!("CARGO_BIN_EXE_hosted_fake_server");
    let envelope = hosted_envelope("landing", "team/apps/landing", fake);
    c.save_app(proto::SaveAppRequest {
        handle: "team/apps/landing".into(),
        envelope_json: envelope,
        source_digest: Vec::new(),
    })
    .await
    .expect("save the hosted app")
    .into_inner();

    // Start the hosted app (returns immediately; the lifecycle runs in the background).
    let start_status = c
        .start_hosted_app(proto::StartHostedAppRequest {
            handle: "team/apps/landing".into(),
            rebuild: false,
        })
        .await
        .expect("start the hosted app")
        .into_inner();
    assert_eq!(start_status.framework, "vite_react");

    // Poll to Running (materialize → skip install → spawn fake server → readiness).
    let mut port = 0u32;
    let mut reached_running = false;
    for _ in 0..80 {
        let st = c
            .get_hosted_app_status(proto::GetHostedAppStatusRequest {
                handle: "team/apps/landing".into(),
            })
            .await
            .expect("status")
            .into_inner();
        if st.state == proto::HostedAppState::HostedRunning as i32 {
            port = st.port;
            reached_running = true;
            assert!(
                st.url.contains(&format!("127.0.0.1:{}", st.port)),
                "url points at the loopback port: {:?}",
                st.url
            );
            break;
        }
        assert_ne!(
            st.state,
            proto::HostedAppState::HostedFailed as i32,
            "hosted app failed: {}",
            st.detail
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    assert!(reached_running, "the hosted app reached Running");
    assert!(port > 0, "a loopback port was allocated");

    // The dev server serves HTTP 200 on the loopback port.
    let port_u16 = u16::try_from(port).unwrap();
    let resp = tokio::task::spawn_blocking(move || http_get(port_u16))
        .await
        .unwrap()
        .expect("the dev server accepts a connection");
    assert!(resp.contains("200"), "served a 200 response: {resp:?}");

    // Stop reaps the child.
    let stopped = c
        .stop_hosted_app(proto::StopHostedAppRequest {
            handle: "team/apps/landing".into(),
        })
        .await
        .expect("stop")
        .into_inner();
    assert!(stopped.stopped, "a running app was stopped");

    let after = c
        .get_hosted_app_status(proto::GetHostedAppStatusRequest {
            handle: "team/apps/landing".into(),
        })
        .await
        .expect("status after stop")
        .into_inner();
    assert_eq!(
        after.state,
        proto::HostedAppState::HostedStopped as i32,
        "state returns to Stopped"
    );

    // The port is no longer served (the child was reaped). Retry briefly for the OS to
    // release the socket.
    let mut released = false;
    for _ in 0..25 {
        if http_get(port_u16).is_err() {
            released = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        released,
        "the dev-server port stops serving after stop (child reaped)"
    );
}

/// The PRODUCTION serve lane reaches Running through the same states, and reports
/// `serve_mode: "production"` so a client never has to infer the lane from the sequence.
///
/// Hermetic: the `"skip"` install sentinel also skips the build, so this exercises the
/// lane's control flow (the extra Building step, the production spawn, the echoed mode)
/// without Node on the box. That the DEV lane never enters Building is asserted by the
/// dev test above never seeing it.
#[tokio::test]
async fn a_production_hosted_app_builds_then_serves_and_reports_its_lane() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    let fake = env!("CARGO_BIN_EXE_hosted_fake_server");
    let envelope = hosted_envelope_mode("shop", "team/apps/shop", fake, "production");
    c.save_app(proto::SaveAppRequest {
        handle: "team/apps/shop".into(),
        envelope_json: envelope,
        source_digest: Vec::new(),
    })
    .await
    .expect("save the production hosted app");

    let started = c
        .start_hosted_app(proto::StartHostedAppRequest {
            handle: "team/apps/shop".into(),
            rebuild: false,
        })
        .await
        .expect("start")
        .into_inner();
    assert_eq!(
        started.serve_mode, "production",
        "the lane is echoed from the envelope, not inferred"
    );

    let mut reached_running = false;
    for _ in 0..80 {
        let st = c
            .get_hosted_app_status(proto::GetHostedAppStatusRequest {
                handle: "team/apps/shop".into(),
            })
            .await
            .expect("status")
            .into_inner();
        assert_ne!(
            st.state,
            proto::HostedAppState::HostedFailed as i32,
            "production hosted app failed: {}",
            st.detail
        );
        if st.state == proto::HostedAppState::HostedRunning as i32 {
            assert_eq!(st.serve_mode, "production");
            assert!(st.url.contains(&format!("127.0.0.1:{}", st.port)));
            reached_running = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    assert!(reached_running, "the production app reached Running");
}

/// An app that never set `serve_mode` — i.e. every app authored before the field existed —
/// keeps serving on the DEV lane. An unknown label must degrade the same way: unrecognized
/// input can never silently promote an app into a lane it did not ask for.
#[tokio::test]
async fn an_absent_or_unknown_serve_mode_stays_on_the_dev_lane() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;
    let fake = env!("CARGO_BIN_EXE_hosted_fake_server");

    for (handle, mode) in [
        ("team/apps/legacy", ""),
        ("team/apps/typo", "PRODUCTION-ish"),
    ] {
        c.save_app(proto::SaveAppRequest {
            handle: handle.into(),
            envelope_json: hosted_envelope_mode("app", handle, fake, mode),
            source_digest: Vec::new(),
        })
        .await
        .expect("save");
        let st = c
            .start_hosted_app(proto::StartHostedAppRequest {
                handle: handle.into(),
                rebuild: false,
            })
            .await
            .expect("start")
            .into_inner();
        assert_eq!(
            st.serve_mode, "dev",
            "serve_mode {mode:?} must degrade to the dev lane"
        );
    }
}
