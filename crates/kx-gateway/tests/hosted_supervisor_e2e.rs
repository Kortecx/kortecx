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

/// The project `package.json` the supervisor materialized, found by walking the gateway's
/// data dir. Located by search rather than by recomputing `<catalog>/hosted/<hash>`: a
/// second copy of that derivation would agree with the code by construction, which is
/// exactly the property a test must not have.
fn find_materialized_package_json(root: &std::path::Path) -> Option<String> {
    for entry in std::fs::read_dir(root).ok()? {
        let path = entry.ok()?.path();
        if path.is_dir() {
            if let Some(found) = find_materialized_package_json(&path) {
                return Some(found);
            }
        } else if path.file_name().is_some_and(|n| n == "package.json") {
            return std::fs::read_to_string(&path).ok();
        }
    }
    None
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

    // The SDK dependency in the MATERIALIZED project — the bytes npm will actually read.
    //
    // The gateway serves exactly one version of `@kortecx/sdk` from its own scoped
    // registry, derived at build time from `bindings/typescript/package.json`, so the
    // template cannot carry a version and the supervisor pins it at write time. Assert the
    // written file, not the template: a template guard cannot see a writer that stopped
    // pinning. Both configurations are asserted, since only their pair is falsifiable —
    // WITHOUT the console there is no registry, so the unpinned range is correct and a
    // pinned one would be a fiction; WITH it, an unpinned range means the pin was skipped.
    let written = find_materialized_package_json(dir.path())
        .expect("the supervisor materialized a package.json");
    let project: serde_json::Value =
        serde_json::from_str(&written).expect("the materialized package.json parses");
    let range = project["dependencies"]["@kortecx/sdk"]
        .as_str()
        .expect("the vite-react project declares the SDK dependency");
    if cfg!(feature = "console") {
        assert_ne!(
            range, "*",
            "this build serves an SDK registry, so the project must ask for a concrete \
             version — an unpinned range means the write-time pin was skipped"
        );
    } else {
        assert_eq!(
            range, "*",
            "this build serves no SDK registry, so there is no version to pin to"
        );
    }

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

/// Find a file by NAME anywhere under the gateway's data dir (the workdir hash is
/// the supervisor's own derivation — recomputing it here would agree by construction).
fn find_file(root: &std::path::Path, name: &str) -> Option<std::path::PathBuf> {
    for entry in std::fs::read_dir(root).ok()? {
        let path = entry.ok()?.path();
        if path.is_dir() {
            if let Some(found) = find_file(&path, name) {
                return Some(found);
            }
        } else if path.file_name().is_some_and(|n| n == name) {
            return Some(path);
        }
    }
    None
}

/// Drive `handle` (already saved) to Running and return its port.
async fn start_to_running(c: &mut KxGatewayClient<Channel>, handle: &str) -> u32 {
    c.start_hosted_app(proto::StartHostedAppRequest {
        handle: handle.into(),
        rebuild: false,
    })
    .await
    .expect("start")
    .into_inner();
    for _ in 0..80 {
        let st = c
            .get_hosted_app_status(proto::GetHostedAppStatusRequest {
                handle: handle.into(),
            })
            .await
            .expect("status")
            .into_inner();
        assert_ne!(
            st.state,
            proto::HostedAppState::HostedFailed as i32,
            "hosted app failed: {}",
            st.detail
        );
        if st.state == proto::HostedAppState::HostedRunning as i32 {
            return st.port;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    panic!("the hosted app never reached Running");
}

/// Stop kills the WHOLE process tree, not just the direct child.
///
/// The fixture forks a sleeper (the `npm run dev` → vite grandchild shape) and
/// records its pid. `start_kill` alone — the pre-group-kill behaviour — reaped
/// only the direct child and left the grandchild running; this asserts the
/// GRANDCHILD dies on stop, which only the process-group kill achieves.
#[cfg(unix)]
#[tokio::test]
async fn stop_kills_the_whole_process_tree() {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    let fake = env!("CARGO_BIN_EXE_hosted_fake_server");
    let handle = "team/apps/forker";
    c.save_app(proto::SaveAppRequest {
        handle: handle.into(),
        envelope_json: hosted_envelope("forker", handle, &format!("{fake} --fork-child")),
        source_digest: Vec::new(),
    })
    .await
    .expect("save");
    start_to_running(&mut c, handle).await;

    let pid_file = find_file(dir.path(), "child.pid").expect("the fixture recorded its child");
    let child_pid: i32 = std::fs::read_to_string(&pid_file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let alive = |pid: i32| nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None).is_ok();
    assert!(
        alive(child_pid),
        "the grandchild is alive while the app runs"
    );

    c.stop_hosted_app(proto::StopHostedAppRequest {
        handle: handle.into(),
    })
    .await
    .expect("stop");
    let mut dead = false;
    for _ in 0..50 {
        if !alive(child_pid) {
            dead = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        dead,
        "the GRANDCHILD must die on stop — a direct-child kill leaves the real server running"
    );
}

/// Hosted children never inherit the gateway's environment.
///
/// The gateway's env can carry operator secrets; before the hygiene pass the
/// whole thing reached every npm child. The fixture dumps its COMPLETE env: a
/// canary set in the gateway process must be absent, and HOME must be the
/// workdir-scoped one (proof the allowlist, not inheritance, produced the env).
#[tokio::test]
async fn hosted_children_never_inherit_the_gateway_environment() {
    // Uniquely named; nothing else reads it, so the process-global set is benign.
    std::env::set_var("KX_TEST_SECRET_CANARY", "leak-me-if-you-can");
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    let fake = env!("CARGO_BIN_EXE_hosted_fake_server");
    let handle = "team/apps/envdump";
    c.save_app(proto::SaveAppRequest {
        handle: handle.into(),
        envelope_json: hosted_envelope(
            "envdump",
            handle,
            &format!("{fake} --dump-env envdump.txt"),
        ),
        source_digest: Vec::new(),
    })
    .await
    .expect("save");
    start_to_running(&mut c, handle).await;

    let dump_path = find_file(dir.path(), "envdump.txt").expect("the fixture dumped its env");
    let dump = std::fs::read_to_string(&dump_path).unwrap();
    assert!(
        !dump.contains("KX_TEST_SECRET_CANARY"),
        "the gateway's env must NOT reach a hosted child:\n{dump}"
    );
    let home_line = dump
        .lines()
        .find(|l| l.starts_with("HOME="))
        .expect("the child has a HOME");
    assert!(
        home_line.contains(".kx-home"),
        "HOME is the workdir-scoped one, not the operator's: {home_line}"
    );
    assert!(
        dump.lines().any(|l| l.starts_with("PATH=")),
        "the minimal PATH is present"
    );
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
