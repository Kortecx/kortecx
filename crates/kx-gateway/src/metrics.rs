//! W1a (T-OBS2): the Prometheus `/metrics` HTTP listener — a loopback/trusted
//! observability endpoint mirroring the embedded console listener (hyper http1,
//! GET/HEAD only, no runtime filesystem).
//!
//! Serving rules (small on purpose):
//! - `GET|HEAD /metrics` → `200` `text/plain` (the rendered RED metrics body).
//! - `GET|HEAD /` or `/health` → `200 ok` (a cheap liveness ping for a probe).
//! - any other path → `404`; a non-GET/HEAD method → `405` with an `Allow` header.
//!
//! **Unauthenticated by design** (the Prometheus scraper convention, like the
//! `grpc.health.v1` service): bind loopback or a trusted network. A non-loopback
//! bind is allowed but warns at startup (Cloud adds auth/party-scope). The body is
//! a snapshot of durable-fact RED counters ([`kx_otel::MetricsHandle`]) plus an
//! optional recent-window latency block from the telemetry exhaust — never an
//! identity or digest input, so metrics on/off/scraped leaves the digest unchanged.

use std::convert::Infallible;
use std::sync::Arc;

use bytes::Bytes;
use http::{header, Method, Request, Response, StatusCode};
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use kx_gateway_core::TelemetryView;
use kx_otel::{LatencySummary, MetricsHandle};
use tokio::net::TcpListener;

/// Prometheus text exposition content type (format version 0.0.4).
const PROM_CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

/// How many recent telemetry rows the latency window covers.
const LATENCY_WINDOW: usize = 256;

/// Compute the recent-window latency summary from the telemetry seam, or `None`
/// when no model Mote has run (an honest omission — never fabricated zeros). Uses
/// the SAME exhaust the Monitoring UI reads (`ListMoteTelemetry`).
fn latency_summary(telemetry: &dyn TelemetryView) -> Option<LatencySummary> {
    let (rows, _has_more) = telemetry.list(LATENCY_WINDOW, None, None, None).ok()?;
    let mut walls: Vec<u64> = Vec::with_capacity(rows.len());
    let mut output_tokens: u64 = 0;
    for r in &rows {
        // Only model Motes carry a latency/token signal; echo/passthrough rows
        // (empty model_id) would skew the window with non-inference timings.
        if r.model_id.is_empty() {
            continue;
        }
        walls.push(r.wall_clock_ms);
        output_tokens = output_tokens.saturating_add(r.output_tokens.unwrap_or(0));
    }
    if walls.is_empty() {
        return None;
    }
    walls.sort_unstable();
    Some(LatencySummary {
        window: walls.len() as u64,
        p50_ms: nearest_rank(&walls, 50),
        p95_ms: nearest_rank(&walls, 95),
        output_tokens,
    })
}

/// Nearest-rank percentile over an ASCENDING, non-empty slice. Integer math only
/// (no float on any path), all-`usize` indexing (no truncating casts).
fn nearest_rank(sorted: &[u64], percentile: usize) -> u64 {
    let n = sorted.len();
    // rank = ceil(percentile/100 * n), clamped to 1..=n; index = rank - 1.
    let rank = (percentile * n).div_ceil(100).max(1);
    let idx = (rank - 1).min(n - 1);
    sorted[idx]
}

/// A small `text/plain` response with `nosniff`.
// SAFETY: every builder uses only static status codes + header names/values, so
// `.body()` is infallible by construction (the documented-infallible pattern the
// workspace lints sanction; mirrors `console::respond`).
#[allow(clippy::expect_used)]
fn text(status: StatusCode, body: &'static str) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header(header::CACHE_CONTROL, "no-cache")
        .body(Full::new(Bytes::from_static(body.as_bytes())))
        .expect("static text response builds")
}

/// Build the response for one request (the rendered body is dynamic; everything
/// else is a static status).
// SAFETY: the `/metrics` builder uses only static header names/values + a freshly
// rendered owned body, so `.body()` is infallible (same sanctioned pattern).
#[allow(clippy::expect_used)]
fn respond(
    req: &Request<Incoming>,
    handle: &MetricsHandle,
    telemetry: Option<&Arc<dyn TelemetryView>>,
) -> Response<Full<Bytes>> {
    if req.method() != Method::GET && req.method() != Method::HEAD {
        return Response::builder()
            .status(StatusCode::METHOD_NOT_ALLOWED)
            .header(header::ALLOW, "GET, HEAD")
            .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
            .body(Full::new(Bytes::new()))
            .expect("static 405 response builds");
    }
    match req.uri().path() {
        "/metrics" => {
            let latency = telemetry.and_then(|t| latency_summary(t.as_ref()));
            let body = handle.render(latency.as_ref());
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, PROM_CONTENT_TYPE)
                .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
                .header(header::CACHE_CONTROL, "no-cache")
                .body(Full::new(Bytes::from(body)))
                .expect("rendered metrics response builds")
        }
        "/" | "/health" => text(StatusCode::OK, "ok\n"),
        _ => text(StatusCode::NOT_FOUND, "not found\n"),
    }
}

/// Accept loop: serve `/metrics` on `listener` until aborted (the task is aborted
/// on gateway shutdown, like the console + WS bridge). Accept errors are logged
/// and the loop continues — one bad connection never takes the endpoint down.
pub(crate) async fn serve_metrics(
    listener: TcpListener,
    handle: MetricsHandle,
    telemetry: Option<Arc<dyn TelemetryView>>,
) {
    loop {
        match listener.accept().await {
            Ok((stream, _peer)) => {
                let handle = handle.clone();
                let telemetry = telemetry.clone();
                tokio::spawn(async move {
                    let io = TokioIo::new(stream);
                    let svc = service_fn(move |req: Request<Incoming>| {
                        let handle = handle.clone();
                        let telemetry = telemetry.clone();
                        async move { Ok::<_, Infallible>(respond(&req, &handle, telemetry.as_ref())) }
                    });
                    if let Err(error) = http1::Builder::new().serve_connection(io, svc).await {
                        tracing::debug!(%error, "metrics connection ended with an error");
                    }
                });
            }
            Err(error) => {
                tracing::warn!(%error, "metrics accept failed; continuing");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_rank_matches_known_percentiles() {
        let v = vec![10, 20, 30, 40, 50, 60, 70, 80, 90, 100];
        // p50 nearest-rank: ceil(0.5*10)=5 → index 4 → 50.
        assert_eq!(nearest_rank(&v, 50), 50);
        // p95 nearest-rank: ceil(0.95*10)=10 → index 9 → 100.
        assert_eq!(nearest_rank(&v, 95), 100);
        // p100 → last.
        assert_eq!(nearest_rank(&v, 100), 100);
        // single element.
        assert_eq!(nearest_rank(&[42], 50), 42);
        assert_eq!(nearest_rank(&[42], 95), 42);
    }
}
