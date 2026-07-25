//! D213 Experience lane — the REAL hosted-app path (live witness): a hosted app is
//! materialized to a real Vite-React project tree, `npm install`ed, and served by a real
//! `vite` dev server, then hit over HTTP for a 200. Unlike the hermetic
//! `hosted_supervisor_e2e` (a std-only fake server, no Node), this proves a GENERATED app
//! genuinely BUILDS + RUNS — so it is `#[ignore]` (needs Node/npm).
//!
//! **It needs the `console` feature too, and used to claim otherwise.** A Vite-React hosted
//! project depends on `@kortecx/sdk`, which is on no public registry — the serving gateway
//! hosts it, on its console listener. Without that listener `npm install` reaches
//! registry.npmjs.org and gets a `404` for the package itself, so this witness could NEVER
//! pass on a `hosted-apps`-only build. Being `#[ignore]`d, nothing noticed. Verified against
//! `main` before this change: identical `E404 … '@kortecx/sdk@^0.1.1' could not be found`.
//!
//! Run locally: `cargo test -p kx-gateway --features console,hosted-apps --test
//! hosted_app_live_serve -- --ignored --nocapture` (first run downloads the npm deps).

#![cfg(all(feature = "hosted-apps", feature = "console"))]
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

/// A concrete loopback port for the console listener.
///
/// Not `:0`: the supervisor writes the scaffolded project's `.npmrc` from the CONFIGURED
/// `console_listen` rather than the address the listener resolved, so an ephemeral `:0`
/// reaches npm as a literal `http://127.0.0.1:0/npm/` and fails `EADDRNOTAVAIL` — found
/// running this witness. A real serve always configures a port, so this is faithful; the
/// derivation is worth tightening separately (the packument already does the opposite,
/// correct thing — it echoes the request's own Host, so it can never advertise a URL the
/// client cannot reach).
fn free_console_port() -> u16 {
    let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);
    port
}

/// A gateway config with a real console listener — the registry a hosted project installs
/// its SDK from.
fn config_with_console(dir: &tempfile::TempDir) -> kx_gateway::GatewayConfig {
    let mut cfg = common::gateway_config(dir, true, HashMap::new());
    cfg.console_listen = kx_gateway::ConsoleMode::Listen(
        format!("127.0.0.1:{}", free_console_port())
            .parse()
            .unwrap(),
    );
    cfg
}

#[tokio::test]
#[ignore = "needs Node/npm — real Vite install + dev server"]
async fn hosted_app_serves_real_vite() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(config_with_console(&dir)).await.unwrap();
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

/// The SDK channel, proven by the client that actually has to work: **npm**.
///
/// The gateway hosts `@kortecx/sdk` on its console listener as a scoped registry, and the
/// supervisor pins the scaffolded project to the version being served. Nothing on this path
/// can be proven by a stub — the range is resolved by npm, against a packument, over HTTP.
/// The assertion is on the INSTALLED package's own manifest, on disk, rather than on what
/// the project asked for.
///
/// `#[ignore]` (needs Node/npm, and installs for real). Run locally:
/// `cargo test -p kx-gateway --features console,hosted-apps
///   hosted_app_installs_the_sdk_from_its_own_gateway -- --ignored --nocapture`
#[tokio::test]
#[ignore = "needs Node/npm — real npm install against the gateway's own registry"]
async fn hosted_app_installs_the_sdk_from_its_own_gateway() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(config_with_console(&dir)).await.unwrap();
    let mut c = client(running.local_addr()).await;

    let envelope = kx_app::AppEnvelope::new_experience(
        "sdk-channel",
        kx_app::HostedConfig {
            framework: kx_app::HostedFramework::ViteReact,
            ..Default::default()
        },
        "team/apps/sdk-channel",
    )
    .to_canonical_json()
    .unwrap();
    c.save_app(proto::SaveAppRequest {
        handle: "team/apps/sdk-channel".into(),
        envelope_json: envelope,
        source_digest: Vec::new(),
    })
    .await
    .expect("save the hosted app");

    c.start_hosted_app(proto::StartHostedAppRequest {
        handle: "team/apps/sdk-channel".into(),
        rebuild: false,
    })
    .await
    .expect("start the hosted app");

    let mut port = 0u32;
    for _ in 0..600 {
        let st = c
            .get_hosted_app_status(proto::GetHostedAppStatusRequest {
                handle: "team/apps/sdk-channel".into(),
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
            "hosted app failed — if this is an npm resolution error, the scaffold asked for \
             a version this gateway's registry does not serve: {}\n{}",
            st.detail,
            st.recent_logs.join("\n")
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    assert!(
        port > 0,
        "the app reached Running (so `npm install` resolved)"
    );

    // npm actually installed the package the gateway served — the proof is on disk, in the
    // installed package's own manifest, not in what the project asked for.
    let installed = find_installed_sdk_manifest(dir.path())
        .expect("npm installed @kortecx/sdk into the project's node_modules");
    let installed: serde_json::Value = serde_json::from_str(&installed).expect("parses");
    assert_eq!(
        installed["name"].as_str(),
        Some("@kortecx/sdk"),
        "the installed package is the SDK"
    );
    println!(
        "  ✓ npm resolved @kortecx/sdk@{} from the gateway's own registry",
        installed["version"].as_str().unwrap_or("?")
    );

    let port_u16 = u16::try_from(port).unwrap();
    let resp = tokio::task::spawn_blocking(move || http_get(port_u16))
        .await
        .unwrap()
        .expect("the Vite dev server accepts a connection");
    assert!(resp.contains("200"), "served a 200: {resp:?}");

    c.stop_hosted_app(proto::StopHostedAppRequest {
        handle: "team/apps/sdk-channel".into(),
    })
    .await
    .expect("stop");
}

/// The installed `@kortecx/sdk/package.json` under the project's `node_modules`.
fn find_installed_sdk_manifest(root: &std::path::Path) -> Option<String> {
    for entry in std::fs::read_dir(root).ok()? {
        let path = entry.ok()?.path();
        if !path.is_dir() {
            continue;
        }
        if path.file_name().is_some_and(|n| n == "node_modules") {
            let manifest = path.join("@kortecx").join("sdk").join("package.json");
            if manifest.is_file() {
                return std::fs::read_to_string(&manifest).ok();
            }
        }
        if let Some(found) = find_installed_sdk_manifest(&path) {
            return Some(found);
        }
    }
    None
}
