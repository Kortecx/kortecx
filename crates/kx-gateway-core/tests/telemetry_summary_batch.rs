//! W1a-3 — the per-model token-economy rollup (`ListTelemetrySummary`) at the
//! gateway-core service boundary. Covers:
//! - the seam is `None` ⇒ the RPC degrades to `Unimplemented` (old-host degrade);
//! - the DEFAULT `TelemetryView::summarize` (the path a non-sqlite host takes)
//!   folds `list()` pages and groups by model — including ACROSS the 500-row
//!   page boundary, so a long run is summed honestly, never a page window;
//! - the service maps the rollup to proto faithfully (the parity contract).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::sync::Arc;

use common::{build_run, service_from, spawn_with_party, MockSubmitter};
use kx_gateway_core::{GatewayError, MoteTelemetryEntry, TelemetryView};
use kx_proto::proto;
use tonic::Code;

/// A canned [`TelemetryView`] backed by an in-memory row set. Only `list()` is
/// implemented — `summarize()` therefore uses the SEAM DEFAULT (the page-fold),
/// which is exactly the path this test exercises.
struct MockTelemetry {
    /// Rows in descending-seq order (newest-first), the `list()` contract.
    rows: Vec<MoteTelemetryEntry>,
}

impl TelemetryView for MockTelemetry {
    fn list(
        &self,
        limit: usize,
        instance_id: Option<[u8; 16]>,
        mote_id: Option<[u8; 32]>,
        before_seq: Option<u64>,
    ) -> Result<(Vec<MoteTelemetryEntry>, bool), GatewayError> {
        let mut hit: Vec<MoteTelemetryEntry> = self
            .rows
            .iter()
            .filter(|r| instance_id.is_none_or(|i| r.instance_id == i))
            .filter(|r| mote_id.is_none_or(|m| r.mote_id == m))
            .filter(|r| before_seq.is_none_or(|b| r.seq < b))
            .cloned()
            .collect();
        hit.sort_by(|a, b| b.seq.cmp(&a.seq));
        let has_more = hit.len() > limit;
        hit.truncate(limit);
        Ok((hit, has_more))
    }
}

fn row(seq: u64, model: &str, out: Option<u64>, wall: u64) -> MoteTelemetryEntry {
    let mut mote = [0u8; 32];
    mote[..8].copy_from_slice(&seq.to_le_bytes());
    MoteTelemetryEntry {
        mote_id: mote,
        instance_id: [0x11; 16],
        wall_clock_ms: wall,
        input_tokens: None,
        output_tokens: out,
        model_id: model.to_string(),
        tool_id: String::new(),
        started_unix_ms: 1,
        seq,
    }
}

fn summary_req() -> proto::ListTelemetrySummaryRequest {
    proto::ListTelemetrySummaryRequest { instance_id: None }
}

#[tokio::test]
async fn summary_default_impl_folds_and_groups_by_model() {
    // model-a: 2 motes (out 10 + 20, wall 1 + 2); model-b: 1 (out 5, wall 7);
    // one non-model (echo) mote: excluded from per-model rows, counted in totals.
    let rows = vec![
        row(5, "model-a", Some(10), 1),
        row(4, "model-a", Some(20), 2),
        row(3, "model-b", Some(5), 7),
        row(2, "", None, 4),
    ];
    let mock = Arc::new(MockTelemetry { rows });
    let service =
        service_from(build_run(), Arc::new(MockSubmitter::default())).with_telemetry_view(mock);
    let mut client = spawn_with_party(service, "tester").await;

    let resp = client
        .list_telemetry_summary(summary_req())
        .await
        .unwrap()
        .into_inner();

    // Per-model rows, descending output tokens (model-a 30 > model-b 5).
    assert_eq!(resp.rows.len(), 2, "echo mote excluded from per-model rows");
    assert_eq!(resp.rows[0].model_id, "model-a");
    assert_eq!(resp.rows[0].count, 2);
    assert_eq!(resp.rows[0].total_output_tokens, 30);
    assert_eq!(resp.rows[0].total_wall_clock_ms, 3);
    assert_eq!(resp.rows[1].model_id, "model-b");
    assert_eq!(resp.rows[1].total_output_tokens, 5);
    // Window-wide totals count the echo mote too.
    assert_eq!(resp.total_motes, 4);
    assert_eq!(resp.total_output_tokens, 35);
}

#[tokio::test]
async fn summary_default_impl_folds_past_the_page_boundary() {
    // 1200 model motes (> the 500-row page) — the default fold MUST page through
    // all of them, so a long run is summed honestly, never a single-page window.
    let mut rows = Vec::new();
    for seq in 1..=1200u64 {
        rows.push(row(seq, "model-a", Some(1), 1));
    }
    let mock = Arc::new(MockTelemetry { rows });
    let service =
        service_from(build_run(), Arc::new(MockSubmitter::default())).with_telemetry_view(mock);
    let mut client = spawn_with_party(service, "tester").await;

    let resp = client
        .list_telemetry_summary(summary_req())
        .await
        .unwrap()
        .into_inner();
    assert_eq!(resp.rows.len(), 1);
    assert_eq!(resp.rows[0].count, 1200, "every page folded, not just 500");
    assert_eq!(resp.total_motes, 1200);
    assert_eq!(resp.total_output_tokens, 1200);
}

#[tokio::test]
async fn summary_without_seam_is_unimplemented() {
    // No telemetry view wired ⇒ the old-host forward-compat degrade.
    let service = service_from(build_run(), Arc::new(MockSubmitter::default()));
    let mut client = spawn_with_party(service, "tester").await;
    let err = client
        .list_telemetry_summary(summary_req())
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::Unimplemented);
}

#[tokio::test]
async fn summary_rejects_a_malformed_instance_filter() {
    let mock = Arc::new(MockTelemetry { rows: vec![] });
    let service =
        service_from(build_run(), Arc::new(MockSubmitter::default())).with_telemetry_view(mock);
    let mut client = spawn_with_party(service, "tester").await;
    let err = client
        .list_telemetry_summary(proto::ListTelemetrySummaryRequest {
            instance_id: Some(vec![0x01, 0x02]), // not 16 bytes
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::InvalidArgument);
}
