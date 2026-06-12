//! In-process gateway spikes: **warm-up** (`start` → health `SERVING`) and
//! **submit→Committed** latency.
//!
//! Each iteration hosts a *fresh* in-process gateway over an ephemeral journal,
//! so neither metric is perturbed by cross-run dedup (a fresh journal ⇒ a fresh
//! echo run every time). All timing is at the **client/dispatch boundary** —
//! never inside the sole-writer commit path or the digest fold (Golden Rule
//! 10(b) / Rule 8 Pass-A perf). FFI-free: the default gateway closure has no
//! llama.cpp, so any contributor can profile their box.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::Path;
use std::time::{Duration, Instant};

use tempfile::TempDir;
use tonic::transport::Channel;
use tonic_health::pb::health_check_response::ServingStatus;
use tonic_health::pb::health_client::HealthClient;
use tonic_health::pb::HealthCheckRequest;

use kx_gateway::{demo_submit_run_request, start, ConsoleMode, GatewayConfig};
use kx_proto::proto;
use kx_proto::proto::kx_gateway_client::KxGatewayClient;

use crate::error::ProfileError;

const POLL: Duration = Duration::from_millis(10);
const ACCEPT_TRIES: u32 = 500; // ≤ 5 s for the listener to accept
const SERVING_TRIES: u32 = 500; // ≤ 5 s to report SERVING
const COMMIT_POLL: Duration = Duration::from_millis(20);
const COMMIT_TRIES: u32 = 500; // ≤ 10 s for the echo Mote to commit

/// Raw per-iteration latency samples (milliseconds).
#[derive(Debug, Clone)]
pub struct LatencySamples {
    /// Time from `start` to health `SERVING`, per iteration.
    pub warmup_ms: Vec<f64>,
    /// Time from `SubmitRun` to the echo Mote reaching `Committed`, per iteration.
    pub submit_ms: Vec<f64>,
}

/// Measure warm-up + submit→Committed over `iterations` fresh in-process
/// gateways.
///
/// # Errors
/// Returns [`ProfileError`] if a gateway fails to start/serve/shutdown, a
/// client call fails, or a stage times out.
pub async fn measure(iterations: usize) -> Result<LatencySamples, ProfileError> {
    let mut warmup_ms = Vec::with_capacity(iterations);
    let mut submit_ms = Vec::with_capacity(iterations);

    for _ in 0..iterations {
        let dir = TempDir::new().map_err(|e| ProfileError::Gateway(e.to_string()))?;

        // M1 — warm-up: start → accepting → SERVING.
        let t0 = Instant::now();
        let running = start(config(dir.path())?)
            .await
            .map_err(|e| ProfileError::Gateway(e.to_string()))?;
        let addr = running.local_addr();
        let channel = connect(addr).await?;
        wait_for_serving(&channel).await?;
        warmup_ms.push(elapsed_ms(t0));

        // M2 — submit→Committed of the FFI-free echo demo.
        let t1 = Instant::now();
        let instance = submit_demo(&channel).await?;
        wait_for_committed(&channel, &instance).await?;
        submit_ms.push(elapsed_ms(t1));

        running
            .shutdown()
            .await
            .map_err(|e| ProfileError::Gateway(e.to_string()))?;
    }

    Ok(LatencySamples {
        warmup_ms,
        submit_ms,
    })
}

/// An ephemeral loopback, dev-auth gateway config rooted at `dir`.
pub(crate) fn config(dir: &Path) -> Result<GatewayConfig, ProfileError> {
    let parse = |s: &str| -> Result<SocketAddr, ProfileError> {
        s.parse()
            .map_err(|e| ProfileError::Gateway(format!("bad listen addr {s}: {e}")))
    };
    Ok(GatewayConfig {
        listen: parse("127.0.0.1:0")?,
        ws_listen: parse("127.0.0.1:0")?,
        journal_path: dir.join("kx.db"),
        content_root: dir.join("blobs"),
        max_lease: 16,
        dev_allow_local: true,
        auth_tokens: HashMap::new(),
        catalog_dir: None,
        tls: None,
        cors_origins: Vec::new(),
        console_listen: ConsoleMode::Disabled,
        content_max_bytes: kx_gateway::DEFAULT_CONTENT_MAX_BYTES,
    })
}

/// Wait for the listener to accept, then build a channel.
pub(crate) async fn connect(addr: SocketAddr) -> Result<Channel, ProfileError> {
    for _ in 0..ACCEPT_TRIES {
        if tokio::net::TcpStream::connect(addr).await.is_ok() {
            let uri = format!("http://{addr}");
            return Channel::from_shared(uri)
                .map_err(|e| ProfileError::Client(e.to_string()))?
                .connect()
                .await
                .map_err(|e| ProfileError::Client(e.to_string()));
        }
        tokio::time::sleep(POLL).await;
    }
    Err(ProfileError::Timeout {
        what: format!("{addr} to accept connections"),
        elapsed_ms: u64::from(ACCEPT_TRIES) * 10,
    })
}

/// Poll the `grpc.health.v1` overall status until `SERVING` (warm-up signal —
/// matches `kx health`).
async fn wait_for_serving(channel: &Channel) -> Result<(), ProfileError> {
    let mut health = HealthClient::new(channel.clone());
    for _ in 0..SERVING_TRIES {
        if let Ok(resp) = health
            .check(HealthCheckRequest {
                service: String::new(),
            })
            .await
        {
            if resp.into_inner().status() == ServingStatus::Serving {
                return Ok(());
            }
        }
        tokio::time::sleep(POLL).await;
    }
    Err(ProfileError::Timeout {
        what: "health SERVING".to_string(),
        elapsed_ms: u64::from(SERVING_TRIES) * 10,
    })
}

/// Submit the FFI-free echo demo run; return its journaled `instance_id`.
async fn submit_demo(channel: &Channel) -> Result<Vec<u8>, ProfileError> {
    let mut client = KxGatewayClient::new(channel.clone());
    let handle = client
        .submit_run(demo_submit_run_request())
        .await
        .map_err(|s| ProfileError::Client(s.to_string()))?
        .into_inner();
    Ok(handle.instance_id)
}

/// Poll the projection until any Mote of the run is `Committed`.
async fn wait_for_committed(channel: &Channel, instance_id: &[u8]) -> Result<(), ProfileError> {
    let mut client = KxGatewayClient::new(channel.clone());
    let committed = proto::MoteSnapshotState::Committed as i32;
    for _ in 0..COMMIT_TRIES {
        let view = client
            .get_projection(proto::GetProjectionRequest {
                instance_id: instance_id.to_vec(),
                at_seq: None,
            })
            .await
            .map_err(|s| ProfileError::Client(s.to_string()))?
            .into_inner();
        if view.motes.iter().any(|m| m.state == committed) {
            return Ok(());
        }
        tokio::time::sleep(COMMIT_POLL).await;
    }
    Err(ProfileError::Timeout {
        what: "a Committed Mote".to_string(),
        elapsed_ms: u64::from(COMMIT_TRIES) * 20,
    })
}

/// Milliseconds elapsed since `t`, as an `f64` (no integer cast).
fn elapsed_ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

/// Nearest-rank percentile `p` (1..=100) of `samples`, computed with integer
/// rank arithmetic (no float cast on the sample count). Empty ⇒ `0.0`.
#[must_use]
pub fn percentile(samples: &[f64], p: usize) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    let n = sorted.len();
    // 1-based ceil(p/100 * n), clamped into [0, n-1].
    let rank = (p * n).div_ceil(100);
    let idx = rank.saturating_sub(1).min(n - 1);
    sorted[idx]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentile_nearest_rank() {
        let s = vec![10.0, 20.0, 30.0, 40.0, 50.0];
        assert_eq!(percentile(&s, 50), 30.0);
        assert_eq!(percentile(&s, 99), 50.0);
        assert_eq!(percentile(&s, 100), 50.0);
        assert_eq!(percentile(&[], 50), 0.0);
        assert_eq!(percentile(&[42.0], 99), 42.0);
    }
}
