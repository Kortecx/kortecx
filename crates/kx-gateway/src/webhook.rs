//! D113: the local webhook ingress listener — the runtime's FIRST untrusted-INBOUND
//! surface, with its own fail-closed threat model (forgery / replay / amplification).
//!
//! `POST /trigger/<name>` with a JSON body starts a fresh registered run for the named
//! trigger (via [`crate::trigger_gateway::HostTriggerAdmin::submit`], the Invoke
//! propose-proxy). Every control is fail-closed:
//!
//! - **Authn (per-trigger).** `HMAC_SHA256` verifies `X-Kx-Signature-256` over the RAW
//!   body with the trigger's secret (resolved by NAME from the keychain), constant-time
//!   (`Mac::verify_slice`). `BEARER` constant-time-compares `Authorization: Bearer`.
//!   `NONE` is accepted ONLY on a loopback bind. An unknown/disabled/non-webhook trigger
//!   is a uniform `401` (no existence oracle).
//! - **Payload cap.** A body over [`MAX_WEBHOOK_BODY`] is refused `413` before buffering.
//! - **Idempotency.** `X-Kx-Idempotency-Key` (else a server-derived key) dedups replays
//!   in `triggers.db` — a duplicate fires NO run and returns the prior run id.
//! - **Rate-limit.** A per-trigger integer token bucket caps an authenticated-but-abusive
//!   sender (`429`).
//!
//! Host-owned (the gateway-core stays tokio-free). The SSRF egress resolver
//! (`kx-mcp::egress`) gates any future trigger callback; the MVP is fire-and-forget.

use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use hmac::{Hmac, Mac};
use http::{header, Method, Request, Response, StatusCode};
use http_body_util::{BodyExt, Full, Limited};
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use kx_gateway_core::{TriggerAdmin, TriggerAdminError};
use sha2::Sha256;
use tokio::net::TcpListener;

use crate::secrets::resolve_secret_value;
use crate::triggers_store::TriggersDb;

/// Max webhook body (256 KiB) — generous for an alert/event payload, tight enough to
/// deny an unbounded-body amplification. Refused `413` before buffering.
const MAX_WEBHOOK_BODY: usize = 256 * 1024;

/// Per-trigger token bucket: 20 burst, 10 events/sec sustained (the MCP-gateway default).
const RL_BURST: u32 = 20;
const RL_REFILL_PER_SEC: u32 = 10;

type HmacSha256 = Hmac<Sha256>;

/// The shared state a webhook connection serves over.
pub(crate) struct WebhookState {
    pub triggers: Arc<TriggersDb>,
    pub admin: Arc<dyn TriggerAdmin>,
    /// True when the webhook listener is bound to a loopback address (gates `NONE` auth).
    pub bind_is_loopback: bool,
    rate: WebhookRateLimiter,
}

impl WebhookState {
    pub(crate) fn new(
        triggers: Arc<TriggersDb>,
        admin: Arc<dyn TriggerAdmin>,
        bind_is_loopback: bool,
    ) -> Self {
        Self {
            triggers,
            admin,
            bind_is_loopback,
            rate: WebhookRateLimiter::new(RL_BURST, RL_REFILL_PER_SEC),
        }
    }
}

/// A per-key integer token bucket (no float — refill is integer ms math).
struct WebhookRateLimiter {
    burst: u32,
    refill_per_sec: u32,
    buckets: Mutex<HashMap<String, (u32, u64)>>, // key -> (tokens, last_refill_ms)
}

impl WebhookRateLimiter {
    fn new(burst: u32, refill_per_sec: u32) -> Self {
        Self {
            burst,
            refill_per_sec,
            buckets: Mutex::new(HashMap::new()),
        }
    }

    /// Try to consume one token for `key` at `now_ms`. `false` ⇒ rate-limited.
    fn try_acquire(&self, key: &str, now_ms: u64) -> bool {
        let Ok(mut buckets) = self.buckets.lock() else {
            return true; // a poisoned lock must not wedge the endpoint
        };
        let entry = buckets
            .entry(key.to_string())
            .or_insert((self.burst, now_ms));
        let elapsed = now_ms.saturating_sub(entry.1);
        let refill =
            u32::try_from(elapsed * u64::from(self.refill_per_sec) / 1000).unwrap_or(u32::MAX);
        if refill > 0 {
            entry.0 = entry.0.saturating_add(refill).min(self.burst);
            entry.1 = now_ms;
        }
        if entry.0 > 0 {
            entry.0 -= 1;
            true
        } else {
            false
        }
    }
}

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// Constant-time byte compare (length is allowed to leak — signatures are fixed-length).
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Decode a lowercase/uppercase hex string to bytes; `None` on any non-hex/odd input.
fn hex_decode(s: &str) -> Option<Vec<u8>> {
    let s = s.trim();
    if !s.len().is_multiple_of(2) {
        return None;
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    let bytes = s.as_bytes();
    let nib = |c: u8| -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            b'A'..=b'F' => Some(c - b'A' + 10),
            _ => None,
        }
    };
    for pair in bytes.chunks_exact(2) {
        out.push((nib(pair[0])? << 4) | nib(pair[1])?);
    }
    Some(out)
}

/// Verify an HMAC-SHA256 signature (hex, optional `sha256=` prefix) over `body`,
/// constant-time. `false` on a bad key / malformed sig / mismatch.
fn verify_hmac_sha256(secret: &str, body: &[u8], sig_header: &str) -> bool {
    let hexsig = sig_header
        .trim()
        .strip_prefix("sha256=")
        .unwrap_or(sig_header.trim());
    let Some(sig) = hex_decode(hexsig) else {
        return false;
    };
    let Ok(mut mac) = HmacSha256::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    mac.update(body);
    mac.verify_slice(&sig).is_ok()
}

/// A small JSON response with `nosniff`.
#[allow(clippy::expect_used)]
fn json(status: StatusCode, body: String) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header(header::CACHE_CONTROL, "no-store")
        .body(Full::new(Bytes::from(body)))
        .expect("static json response builds")
}

/// The uniform unauthorized response — no existence oracle (unknown trigger, disabled
/// trigger, bad signature, and missing auth are byte-identical).
fn unauthorized() -> Response<Full<Bytes>> {
    json(
        StatusCode::UNAUTHORIZED,
        "{\"error\":\"unauthorized\"}".to_string(),
    )
}

/// Map a post-auth submit error to a status (the caller is authenticated here, so a
/// specific code is acceptable — it is not an unauth oracle).
fn submit_error(e: &TriggerAdminError) -> Response<Full<Bytes>> {
    let (code, msg) = match e {
        TriggerAdminError::InvalidArgument(d) => (StatusCode::BAD_REQUEST, d.clone()),
        TriggerAdminError::NotAuthorized => (StatusCode::FORBIDDEN, "not authorized".to_string()),
        TriggerAdminError::Unsupported(d) => (StatusCode::CONFLICT, d.clone()),
        TriggerAdminError::NotFound(d) => (StatusCode::NOT_FOUND, d.clone()),
        TriggerAdminError::Storage(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal error".to_string(),
        ),
    };
    json(code, format!("{{\"error\":{}}}", json_string(&msg)))
}

/// Minimal JSON string escaping (quotes + backslashes + control chars).
fn json_string(s: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Handle one webhook request.
async fn handle(req: Request<Incoming>, state: Arc<WebhookState>) -> Response<Full<Bytes>> {
    let (parts, body) = req.into_parts();
    if parts.method != Method::POST {
        return Response::builder()
            .status(StatusCode::METHOD_NOT_ALLOWED)
            .header(header::ALLOW, "POST")
            .body(Full::new(Bytes::new()))
            .unwrap_or_else(|_| json(StatusCode::METHOD_NOT_ALLOWED, String::new()));
    }
    // Path: /trigger/<name>
    let Some(name) = parts
        .uri
        .path()
        .strip_prefix("/trigger/")
        .map(str::to_string)
    else {
        return unauthorized(); // any other path is a uniform 401 (no route oracle)
    };
    if name.is_empty() {
        return unauthorized();
    }
    // Look up the trigger; an unknown / non-webhook / disabled trigger is a uniform 401.
    let cfg = match state.triggers.get(&name) {
        Ok(Some(c)) if c.kind == "webhook" && c.enabled => c,
        _ => return unauthorized(),
    };

    // Rate-limit (per trigger) BEFORE reading the body (cap amplification cost).
    if !state.rate.try_acquire(&name, now_unix_ms()) {
        return json(
            StatusCode::TOO_MANY_REQUESTS,
            "{\"error\":\"rate limited\"}".to_string(),
        );
    }

    // Read the body with a hard cap (413 if over / on a read error).
    let raw = match Limited::new(body, MAX_WEBHOOK_BODY).collect().await {
        Ok(c) => c.to_bytes(),
        Err(_) => {
            return json(
                StatusCode::PAYLOAD_TOO_LARGE,
                "{\"error\":\"payload too large\"}".to_string(),
            )
        }
    };

    // Authn — fail closed on every path.
    let authed = match cfg.auth.as_str() {
        "none" => state.bind_is_loopback, // NONE only on a loopback bind
        "hmac_sha256" => {
            let Some(secret) = resolve_secret_value(&cfg.auth_secret_ref) else {
                return unauthorized(); // can't verify ⇒ refuse
            };
            parts
                .headers
                .get("x-kx-signature-256")
                .and_then(|v| v.to_str().ok())
                .is_some_and(|sig| verify_hmac_sha256(&secret, &raw, sig))
        }
        "bearer" => {
            let Some(secret) = resolve_secret_value(&cfg.auth_secret_ref) else {
                return unauthorized();
            };
            parts
                .headers
                .get(header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.strip_prefix("Bearer "))
                .is_some_and(|tok| ct_eq(tok.as_bytes(), secret.as_bytes()))
        }
        _ => false,
    };
    if !authed {
        return unauthorized();
    }

    // Idempotency key (header, else server-derived in submit).
    let idem = parts
        .headers
        .get("x-kx-idempotency-key")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let payload = String::from_utf8_lossy(&raw).into_owned();

    match state.admin.submit(&name, &idem, &payload).await {
        Ok(out) => json(
            StatusCode::OK,
            format!(
                "{{\"instance_id\":\"{}\",\"deduped\":{}}}",
                hex_encode(&out.instance_id),
                out.deduped
            ),
        ),
        Err(e) => submit_error(&e),
    }
}

/// Accept loop: serve the webhook ingress on `listener` until the task is aborted (on
/// gateway shutdown, like the metrics + WS listeners). One bad connection never takes
/// the endpoint down.
pub(crate) async fn serve_webhook(listener: TcpListener, state: Arc<WebhookState>) {
    loop {
        match listener.accept().await {
            Ok((stream, _peer)) => {
                let state = state.clone();
                tokio::spawn(async move {
                    let io = TokioIo::new(stream);
                    let svc = service_fn(move |req: Request<Incoming>| {
                        let state = state.clone();
                        async move { Ok::<_, Infallible>(handle(req, state).await) }
                    });
                    if let Err(error) = http1::Builder::new().serve_connection(io, svc).await {
                        tracing::debug!(%error, "webhook connection ended with an error");
                    }
                });
            }
            Err(error) => {
                tracing::warn!(%error, "webhook accept failed; continuing");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_roundtrip_and_reject_bad() {
        assert_eq!(hex_decode("00ff1a"), Some(vec![0x00, 0xff, 0x1a]));
        assert_eq!(hex_decode("00FF1A"), Some(vec![0x00, 0xff, 0x1a]));
        assert_eq!(hex_decode("0"), None, "odd length");
        assert_eq!(hex_decode("zz"), None, "non-hex");
        assert_eq!(hex_encode(&[0x00, 0xff, 0x1a]), "00ff1a");
    }

    #[test]
    fn ct_eq_matches_only_equal() {
        assert!(ct_eq(b"abc", b"abc"));
        assert!(!ct_eq(b"abc", b"abd"));
        assert!(!ct_eq(b"abc", b"ab"));
    }

    #[test]
    fn hmac_verify_accepts_correct_rejects_tampered() {
        let secret = "topsecret";
        let body = br#"{"prompt":"hello"}"#;
        // Compute the reference signature the way a sender would.
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body);
        let sig = hex_encode(&mac.finalize().into_bytes());

        assert!(
            verify_hmac_sha256(secret, body, &sig),
            "correct sig accepted"
        );
        assert!(
            verify_hmac_sha256(secret, body, &format!("sha256={sig}")),
            "the sha256= prefix is accepted"
        );
        assert!(
            !verify_hmac_sha256("wrongkey", body, &sig),
            "wrong key rejected"
        );
        assert!(
            !verify_hmac_sha256(secret, b"tampered", &sig),
            "tampered body rejected"
        );
        assert!(
            !verify_hmac_sha256(secret, body, "not-hex"),
            "malformed sig rejected"
        );
    }

    #[test]
    fn rate_limiter_caps_then_refills() {
        let rl = WebhookRateLimiter::new(2, 10);
        assert!(rl.try_acquire("t", 0));
        assert!(rl.try_acquire("t", 0));
        assert!(!rl.try_acquire("t", 0), "burst exhausted");
        // 200ms later ⇒ 10/sec * 0.2s = 2 tokens refilled.
        assert!(rl.try_acquire("t", 200), "refilled after elapsed time");
        // A different key has its own bucket.
        assert!(rl.try_acquire("other", 0));
    }
}
