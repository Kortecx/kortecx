//! Batch B inspector-path spike (Golden Rule 10): **`GetMoteDetail`** — the
//! per-mote unary the console's node inspector fires once per drawer open.
//! One in-process gateway hosts a committed echo run; the FIRST call is the
//! COLD path (instance fold + content-store get + canonical decode), every
//! subsequent call the WARM path (the host's def cache). The submit→Committed
//! spike (`spikes::measure`) doubles as the admission-persist overhead
//! measurement — compare its p50 against the pre-PR-2 private baseline.

use std::time::Instant;

use tempfile::TempDir;
use tonic::transport::Channel;

use kx_gateway::{demo_submit_run_request, start};
use kx_proto::proto;
use kx_proto::proto::kx_gateway_client::KxGatewayClient;

use crate::error::ProfileError;
use crate::spikes;

const COMMIT_POLL: std::time::Duration = std::time::Duration::from_millis(20);
const COMMIT_TRIES: u32 = 500;

/// Raw latency samples (milliseconds).
#[derive(Debug, Clone)]
pub struct MoteDetailSamples {
    /// The FIRST `GetMoteDetail` after commit (fold + store get + decode).
    pub detail_cold_ms: Vec<f64>,
    /// Subsequent `GetMoteDetail` round trips (the cached-def path), per iteration.
    pub detail_warm_ms: Vec<f64>,
}

/// Measure the Batch B inspector spike over `iterations` against one gateway.
///
/// # Errors
/// Returns [`ProfileError`] if the gateway fails to start/serve, the demo run
/// fails to commit, or a call fails / answers without the def.
pub async fn measure(iterations: usize) -> Result<MoteDetailSamples, ProfileError> {
    let dir = TempDir::new().map_err(|e| ProfileError::Gateway(e.to_string()))?;
    let running = start(spikes::config(dir.path())?)
        .await
        .map_err(|e| ProfileError::Gateway(e.to_string()))?;
    let channel = spikes::connect(running.local_addr()).await?;
    let mut client = KxGatewayClient::new(channel.clone());

    // Drive the FFI-free echo demo to Committed (the def blob persists at
    // admission; the def HASH appears on the Committed fact).
    let handle = client
        .submit_run(demo_submit_run_request())
        .await
        .map_err(|s| ProfileError::Client(s.to_string()))?
        .into_inner();
    let instance_id = handle.instance_id;
    let mote_id = wait_for_committed_mote(&channel, &instance_id).await?;

    // M-detailB-cold — the first resolve (fold + content get + decode).
    let mut detail_cold_ms = Vec::with_capacity(1);
    let t0 = Instant::now();
    let detail = get_detail(&mut client, &instance_id, &mote_id).await?;
    detail_cold_ms.push(elapsed_ms(t0));
    if !detail.def_found {
        return Err(ProfileError::Client(
            "GetMoteDetail answered def_found=false for a freshly-admitted mote".into(),
        ));
    }

    // M-detailB-warm — the cached-def path (what repeated drawer opens cost).
    let mut detail_warm_ms = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let t1 = Instant::now();
        let _ = get_detail(&mut client, &instance_id, &mote_id).await?;
        detail_warm_ms.push(elapsed_ms(t1));
    }

    running
        .shutdown()
        .await
        .map_err(|e| ProfileError::Gateway(e.to_string()))?;
    Ok(MoteDetailSamples {
        detail_cold_ms,
        detail_warm_ms,
    })
}

async fn get_detail(
    client: &mut KxGatewayClient<Channel>,
    instance_id: &[u8],
    mote_id: &[u8],
) -> Result<proto::MoteDetail, ProfileError> {
    Ok(client
        .get_mote_detail(proto::GetMoteDetailRequest {
            instance_id: instance_id.to_vec(),
            mote_id: mote_id.to_vec(),
        })
        .await
        .map_err(|s| ProfileError::Client(s.to_string()))?
        .into_inner())
}

/// Poll the projection until a Mote commits; return its id.
async fn wait_for_committed_mote(
    channel: &Channel,
    instance_id: &[u8],
) -> Result<Vec<u8>, ProfileError> {
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
        if let Some(m) = view.motes.iter().find(|m| m.state == committed) {
            return Ok(m.mote_id.clone());
        }
        tokio::time::sleep(COMMIT_POLL).await;
    }
    Err(ProfileError::Timeout {
        what: "a Committed Mote".to_string(),
        elapsed_ms: u64::from(COMMIT_TRIES) * 20,
    })
}

fn elapsed_ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}
