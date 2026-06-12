//! Batch A content-path spikes (Golden Rule 10): **`PutContent`** (a 1 MiB
//! client upload — the first client write path) and **`GetContentBatch`** (a full
//! 64-ref × 4 KiB fetch — the N+1-collapse read). One in-process gateway hosts
//! all iterations (the per-op cost is what we measure, not warm-up — that spike
//! already exists); every put uses DISTINCT bytes so dedup never short-circuits
//! the write path under measurement.

use std::time::Instant;

use tempfile::TempDir;
use tonic::transport::Channel;

use kx_gateway::start;
use kx_proto::proto;
use kx_proto::proto::kx_gateway_client::KxGatewayClient;

use crate::error::ProfileError;
use crate::spikes;

const PUT_BYTES: usize = 1024 * 1024; // 1 MiB per upload
const BATCH_REFS: usize = 64; // the server's ref cap
const BATCH_ITEM_BYTES: usize = 4 * 1024; // preview-sized items (the UI's shape)

/// Raw per-iteration latency samples (milliseconds).
#[derive(Debug, Clone)]
pub struct ContentSamples {
    /// One 1 MiB `PutContent` round trip, per iteration (distinct bytes each).
    pub put_1mib_ms: Vec<f64>,
    /// One full 64-ref × 4 KiB `GetContentBatch` round trip, per iteration.
    pub batch_64x4k_ms: Vec<f64>,
}

/// Measure the Batch A content spikes over `iterations` against one gateway.
///
/// # Errors
/// Returns [`ProfileError`] if the gateway fails to start/serve or a call fails.
pub async fn measure(iterations: usize) -> Result<ContentSamples, ProfileError> {
    let dir = TempDir::new().map_err(|e| ProfileError::Gateway(e.to_string()))?;
    let running = start(spikes::config(dir.path())?)
        .await
        .map_err(|e| ProfileError::Gateway(e.to_string()))?;
    let channel = spikes::connect(running.local_addr()).await?;
    let mut client = KxGatewayClient::new(channel.clone())
        // The 64×4 KiB batch response is small, but keep headroom symmetric
        // with the production clients (they raise their decode limits too).
        .max_decoding_message_size(64 * 1024 * 1024);

    // Seed the batch corpus ONCE: 64 distinct 4 KiB blobs.
    let mut refs: Vec<Vec<u8>> = Vec::with_capacity(BATCH_REFS);
    for i in 0..BATCH_REFS {
        let mut payload = vec![0u8; BATCH_ITEM_BYTES];
        payload[..8].copy_from_slice(&(i as u64).to_le_bytes());
        let resp = put(&mut client, payload).await?;
        refs.push(resp);
    }

    let mut put_1mib_ms = Vec::with_capacity(iterations);
    let mut batch_64x4k_ms = Vec::with_capacity(iterations);
    for i in 0..iterations {
        // M-putA — a DISTINCT 1 MiB upload (never a dedup hit).
        let mut payload = vec![0xC3u8; PUT_BYTES];
        payload[..8].copy_from_slice(&(u64::MAX - i as u64).to_le_bytes());
        let t0 = Instant::now();
        let _ = put(&mut client, payload).await?;
        put_1mib_ms.push(elapsed_ms(t0));

        // M-batchA — the full 64-ref fetch (uploads scope).
        let t1 = Instant::now();
        let resp = client
            .get_content_batch(proto::GetContentBatchRequest {
                instance_id: Vec::new(),
                content_refs: refs.clone(),
                max_bytes_per_item: None,
            })
            .await
            .map_err(|s| ProfileError::Client(s.to_string()))?
            .into_inner();
        batch_64x4k_ms.push(elapsed_ms(t1));
        if resp.items.len() != BATCH_REFS {
            return Err(ProfileError::Client(format!(
                "batch returned {} of {BATCH_REFS} items",
                resp.items.len()
            )));
        }
    }

    running
        .shutdown()
        .await
        .map_err(|e| ProfileError::Gateway(e.to_string()))?;
    Ok(ContentSamples {
        put_1mib_ms,
        batch_64x4k_ms,
    })
}

async fn put(
    client: &mut KxGatewayClient<Channel>,
    payload: Vec<u8>,
) -> Result<Vec<u8>, ProfileError> {
    Ok(client
        .put_content(proto::PutContentRequest {
            payload,
            media_type: "application/octet-stream".into(),
            filename: "spike.bin".into(),
        })
        .await
        .map_err(|s| ProfileError::Client(s.to_string()))?
        .into_inner()
        .content_ref)
}

fn elapsed_ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}
