//! D139: the embedded web console — a third loopback listener serving the
//! compile-time-embedded SPA (`build.rs` → `console_assets.rs`).
//!
//! Serving rules (small on purpose):
//! - **GET/HEAD only** — anything else is `405` with an `Allow` header.
//! - An exact manifest hit serves the embedded bytes with its content type.
//! - A miss **without** a file extension is a client-side route (`/chat`,
//!   `/runs/...`) → serve `index.html` (the SPA fallback).
//! - A miss **with** an extension is a genuinely absent asset → `404`.
//! - `assets/*` (hash-named) → `Cache-Control: immutable`; everything else
//!   (notably `index.html`) → `no-cache` so a new release is picked up.
//! - `X-Content-Type-Options: nosniff` everywhere.
//!
//! There is NO runtime filesystem access: the manifest is the complete universe
//! of servable bytes, so path traversal is structurally impossible. The
//! listener is loopback-only (enforced at config time, D139.3); the SPA talks
//! to the gateway's gRPC-web port, which auto-grants ONLY this console's own
//! loopback origins.

use std::convert::Infallible;

use bytes::Bytes;
use http::{header, Method, Request, Response, StatusCode};
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;

include!(concat!(env!("OUT_DIR"), "/console_assets.rs"));

/// What a request path resolves to (pure; unit-tested).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Resolution {
    /// An exact embedded file (index into [`ASSETS`]).
    Asset(usize),
    /// A client-side route — serve `index.html`.
    SpaFallback,
    /// An extension-bearing path that is not embedded.
    NotFound,
}

/// Resolve a URL path against the embedded manifest. `path` is the raw request
/// path (leading `/`); query strings are ignored by the caller.
fn classify(path: &str) -> Resolution {
    let rel = path.trim_start_matches('/');
    let rel = if rel.is_empty() { "index.html" } else { rel };
    if let Some(idx) = ASSETS.iter().position(|a| a.path == rel) {
        return Resolution::Asset(idx);
    }
    // A dot in the LAST segment means an asset was asked for and is missing;
    // anything else is a client-side route the SPA router owns.
    let last = rel.rsplit('/').next().unwrap_or(rel);
    if last.contains('.') {
        Resolution::NotFound
    } else {
        Resolution::SpaFallback
    }
}

/// The `index.html` asset (present by `build.rs` construction).
// SAFETY: build.rs fails the BUILD when the dist has no index.html, so the
// embedded manifest always contains it.
#[allow(clippy::expect_used)]
fn index_asset() -> &'static Asset {
    ASSETS
        .iter()
        .find(|a| a.path == "index.html")
        .expect("build.rs asserts index.html exists in the embedded dist")
}

/// Build the response for one request (pure over the embedded manifest).
// SAFETY: every builder below uses only static, well-formed status codes +
// header names/values — infallible by construction (the documented-infallible
// pattern the workspace lints sanction).
#[allow(clippy::expect_used)]
fn respond(req: &Request<Incoming>) -> Response<Full<Bytes>> {
    if req.method() != Method::GET && req.method() != Method::HEAD {
        return Response::builder()
            .status(StatusCode::METHOD_NOT_ALLOWED)
            .header(header::ALLOW, "GET, HEAD")
            .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
            .body(Full::new(Bytes::new()))
            .expect("static 405 response builds");
    }

    let (status, asset) = match classify(req.uri().path()) {
        Resolution::Asset(idx) => (StatusCode::OK, &ASSETS[idx]),
        Resolution::SpaFallback => (StatusCode::OK, index_asset()),
        Resolution::NotFound => {
            return Response::builder()
                .status(StatusCode::NOT_FOUND)
                .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
                .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
                .body(Full::new(Bytes::from_static(b"not found")))
                .expect("static 404 response builds");
        }
    };

    let cache = if asset.immutable {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    // hyper elides the body for HEAD responses itself; Content-Length stays.
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, asset.content_type)
        .header(header::CACHE_CONTROL, cache)
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .body(Full::new(Bytes::from_static(asset.bytes)))
        .expect("static asset response builds")
}

/// Accept loop: serve the embedded console on `listener` until aborted (the
/// task is aborted on gateway shutdown, like the WS bridge). Accept errors are
/// logged and the loop continues — one bad connection never takes the console
/// down.
pub(crate) async fn serve_console(listener: TcpListener) {
    loop {
        match listener.accept().await {
            Ok((stream, _peer)) => {
                tokio::spawn(async move {
                    let io = TokioIo::new(stream);
                    let svc = service_fn(|req: Request<Incoming>| async move {
                        Ok::<_, Infallible>(respond(&req))
                    });
                    if let Err(error) = http1::Builder::new().serve_connection(io, svc).await {
                        tracing::debug!(%error, "console connection ended with an error");
                    }
                });
            }
            Err(error) => {
                tracing::warn!(%error, "console accept failed; continuing");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_manifest_has_the_spa_entrypoint() {
        assert!(!ASSETS.is_empty(), "build.rs embedded at least index.html");
        assert_eq!(index_asset().path, "index.html");
        assert_eq!(index_asset().content_type, "text/html; charset=utf-8");
        assert!(
            !index_asset().immutable,
            "index.html must revalidate so a new release is picked up"
        );
    }

    #[test]
    fn classify_resolves_root_assets_routes_and_misses() {
        assert!(
            matches!(classify("/"), Resolution::Asset(_)),
            "/ → index.html"
        );
        assert!(matches!(classify("/index.html"), Resolution::Asset(_)));
        // Every client-side route falls back to the SPA entrypoint.
        for route in [
            "/connect",
            "/activity",
            "/chat",
            "/runs",
            "/recipes",
            "/artifacts",
            "/datasets",
            "/systems",
            "/settings",
            "/runs/0123abcd",
        ] {
            assert_eq!(classify(route), Resolution::SpaFallback, "{route}");
        }
        // An extension-bearing miss is a real 404, never the SPA page.
        assert_eq!(classify("/nope.js"), Resolution::NotFound);
        assert_eq!(
            classify("/assets/missing-deadbeef.js"),
            Resolution::NotFound
        );
        // Traversal-shaped paths can only ever be manifest misses (no filesystem).
        assert_eq!(classify("/../Cargo.toml"), Resolution::NotFound);
    }

    #[test]
    fn hashed_assets_are_immutable_everything_else_revalidates() {
        for asset in ASSETS {
            assert_eq!(
                asset.immutable,
                asset.path.starts_with("assets/"),
                "{} cache class",
                asset.path
            );
        }
    }
}
