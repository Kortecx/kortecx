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
    /// `GET /npm/@kortecx/sdk` — the package METADATA (a minimal npm packument).
    SdkPackument,
    /// `GET /npm/@kortecx/sdk/-/…​.tgz` — the package BYTES.
    SdkTarball,
}

/// The scoped-registry root this gateway answers on.
const NPM_ROOT: &str = "npm/";
/// The one package it hosts.
const SDK_PACKAGE: &str = "@kortecx/sdk";

/// Resolve an `/npm/…` path, or `None` when it is not a registry request.
///
/// Deliberately exact rather than prefix-matched: this is a registry for ONE package, and a
/// path that merely starts with the right bytes should 404 like anything else rather than be
/// answered with the SDK.
fn classify_npm(rel: &str) -> Option<Resolution> {
    let rest = rel.strip_prefix(NPM_ROOT)?;
    // npm encodes the scope separator: it requests `@kortecx%2fsdk`, not `@kortecx/sdk`. Found
    // live — the exact match fell through to the SPA and npm choked on `<!doctype html>`. Only
    // `%2f`/`%2F` needs decoding here (the only special byte in a scoped package name); a full
    // percent-decoder would be scope creep for one known encoding.
    let decoded = rest.replace("%2f", "/").replace("%2F", "/");
    if decoded == SDK_PACKAGE {
        return Some(Resolution::SdkPackument);
    }
    // npm fetches the tarball at whatever `dist.tarball` said; we always publish the
    // `<name>/-/sdk-<version>.tgz` form, so accept exactly that.
    let tarball = format!("{SDK_PACKAGE}/-/sdk-{SDK_VERSION}.tgz");
    (decoded == tarball).then_some(Resolution::SdkTarball)
}

/// The npm packument for the embedded SDK, as JSON bytes.
///
/// `authority` is the request's own `Host`, so the `dist.tarball` URL points back at the port
/// this gateway actually bound — the packument cannot advertise a URL the client cannot reach,
/// and no configuration has to be kept in sync with the listener.
///
/// `integrity` is computed over the embedded bytes rather than recorded at build time: npm
/// verifies it, so a value derived from anything other than the bytes being served would be a
/// second source of truth waiting to disagree.
fn packument(authority: &str, tarball: &[u8]) -> Vec<u8> {
    use base64::Engine as _;
    use sha2::{Digest, Sha512};
    let digest = Sha512::digest(tarball);
    let integrity = format!(
        "sha512-{}",
        base64::engine::general_purpose::STANDARD.encode(digest)
    );
    let url = format!("http://{authority}/{NPM_ROOT}{SDK_PACKAGE}/-/sdk-{SDK_VERSION}.tgz");
    serde_json::json!({
        "name": SDK_PACKAGE,
        "dist-tags": { "latest": SDK_VERSION },
        "versions": {
            SDK_VERSION: {
                "name": SDK_PACKAGE,
                "version": SDK_VERSION,
                "dist": { "tarball": url, "integrity": integrity },
            }
        },
    })
    .to_string()
    .into_bytes()
}

/// Serve the registry: the packument, or the tarball bytes.
///
/// A build that packed no SDK answers **501 with a remedy** rather than 404. A 404 would read
/// as "no such package" and send the author hunting for a typo, when the truth is that this
/// binary carries no package to serve.
// SAFETY: static, well-formed status codes and header values throughout (the
// documented-infallible pattern the workspace lints sanction).
#[allow(clippy::expect_used)]
fn npm_response(req: &Request<Incoming>) -> Response<Full<Bytes>> {
    let Some(tarball) = SDK_TARBALL else {
        return Response::builder()
            .status(StatusCode::NOT_IMPLEMENTED)
            .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
            .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
            .body(Full::new(Bytes::from_static(
                b"this build carries no @kortecx/sdk package (build with `just console-dist`, \
                  or set KX_SDK_TARBALL to an `npm pack` tarball)",
            )))
            .expect("static 501 response builds");
    };
    let (body, ctype) = if classify(req.uri().path()) == Resolution::SdkTarball {
        (Bytes::from_static(tarball), "application/octet-stream")
    } else {
        // The authority the CLIENT used, so the tarball URL is reachable from wherever the
        // request came from. A missing Host (HTTP/1.0) falls back to the loopback default the
        // console listener binds.
        let authority = req
            .headers()
            .get(header::HOST)
            .and_then(|h| h.to_str().ok())
            .unwrap_or("127.0.0.1:8888");
        (
            Bytes::from(packument(authority, tarball)),
            "application/json; charset=utf-8",
        )
    };
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, ctype)
        .header(header::CACHE_CONTROL, "no-cache")
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .body(Full::new(body))
        .expect("registry response builds")
}

/// Resolve a URL path against the embedded manifest. `path` is the raw request
/// path (leading `/`); query strings are ignored by the caller.
fn classify(path: &str) -> Resolution {
    let rel = path.trim_start_matches('/');
    let rel = if rel.is_empty() { "index.html" } else { rel };
    // The registry lives under a reserved prefix, checked BEFORE the SPA fallback — otherwise
    // `/npm/@kortecx/sdk` (no extension) would be served index.html and npm would try to parse
    // the console as a packument.
    if let Some(r) = classify_npm(rel) {
        return r;
    }
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
        Resolution::SdkPackument | Resolution::SdkTarball => return npm_response(req),
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

    /// The registry prefix must resolve BEFORE the SPA fallback. `/npm/@kortecx/sdk` carries no
    /// file extension, so without this it would be served `index.html` and npm would try to
    /// parse the console as package metadata — a confusing failure a long way from its cause.
    #[test]
    fn the_registry_prefix_wins_over_the_spa_fallback() {
        assert_eq!(
            classify("/npm/@kortecx/sdk"),
            Resolution::SdkPackument,
            "the packument path must not fall through to index.html"
        );
        // npm actually requests the scope-encoded form — the shape that fell through live.
        assert_eq!(
            classify("/npm/@kortecx%2fsdk"),
            Resolution::SdkPackument,
            "the %2f-encoded scope npm sends must resolve too"
        );
        assert_eq!(
            classify(&format!("/npm/@kortecx/sdk/-/sdk-{SDK_VERSION}.tgz")),
            Resolution::SdkTarball
        );
    }

    /// One package, matched exactly. A path that merely starts with the right bytes must 404
    /// like anything else rather than be answered with the SDK.
    #[test]
    fn a_near_miss_under_the_registry_prefix_is_not_the_sdk() {
        for path in [
            "/npm/@kortecx/sdk-evil",
            "/npm/@kortecx/other",
            "/npm/@kortecx/sdk/-/sdk-9.9.9.tgz",
        ] {
            assert!(
                !matches!(
                    classify(path),
                    Resolution::SdkPackument | Resolution::SdkTarball
                ),
                "{path} must not resolve to the SDK"
            );
        }
    }

    /// The packument's `dist.tarball` must point back at the authority the CLIENT used, and its
    /// `integrity` must be over the bytes actually served — npm verifies it, so a value derived
    /// from anything else is a second source of truth waiting to disagree.
    #[test]
    fn the_packument_advertises_a_reachable_url_and_a_real_integrity() {
        use base64::Engine as _;
        use sha2::{Digest, Sha512};
        let bytes = b"not really a tarball";
        let doc = packument("127.0.0.1:9999", bytes);
        let v: serde_json::Value = serde_json::from_slice(&doc).expect("valid JSON");

        let dist = &v["versions"][SDK_VERSION]["dist"];
        assert_eq!(
            dist["tarball"].as_str().unwrap(),
            format!("http://127.0.0.1:9999/npm/@kortecx/sdk/-/sdk-{SDK_VERSION}.tgz"),
            "the URL must use the request's own authority, not a configured one"
        );
        let want = format!(
            "sha512-{}",
            base64::engine::general_purpose::STANDARD.encode(Sha512::digest(bytes))
        );
        assert_eq!(dist["integrity"].as_str().unwrap(), want);
        assert_eq!(v["dist-tags"]["latest"].as_str().unwrap(), SDK_VERSION);
    }

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
