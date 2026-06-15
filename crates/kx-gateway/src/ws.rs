//! The R5 WebSocket `StreamEvents` bridge — the BROWSER live-tail surface.
//!
//! A browser cannot speak gRPC server-streaming, so this re-skins the SAME
//! [`LiveTailer`](crate::live_tail::LiveTailer) event stream over a WebSocket on a
//! separate port (`--ws-listen`, default [`DEFAULT_WS_LISTEN`](crate::DEFAULT_WS_LISTEN)).
//! Each [`EventFrame`](kx_proto::proto::EventFrame) is sent as one JSON text
//! message with ids/refs rendered as lowercase hex (a SIEM/browser-ergonomic wire
//! that mirrors the R4 audit DTO — no protobuf-es decoder needed downstream).
//!
//! ## Auth (handshake, reusing R2)
//! The bearer token is read from the upgrade request's `Authorization: Bearer …`
//! header, or — for browsers that cannot set arbitrary headers — the
//! `Sec-WebSocket-Protocol: bearer, <token>` subprotocol. Either way it feeds the
//! SAME [`PrincipalResolver`](crate::auth::PrincipalResolver) (deny-all default /
//! token / dev), so an unauthenticated upgrade is rejected BEFORE any stream opens.
//!
//! ## Read-side only
//! It folds the read-only journal handle through the live tailer; it never writes
//! the journal or touches the digest. Slow consumers / ownership failures close
//! the socket with a reason; the client resumes from its last `next_seq`.

use std::borrow::Cow;
use std::sync::{Arc, Mutex};

use futures_util::{SinkExt, StreamExt};
use kx_gateway_core::{EventTailer, GlobalEventTailer, JournalReader, TokenTailer};
use kx_proto::proto;
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::handshake::server::{
    ErrorResponse, Request as HsRequest, Response as HsResponse,
};
use tokio_tungstenite::tungstenite::http::{HeaderValue, StatusCode};
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::Message;
use tonic::metadata::MetadataMap;
use tonic::{Code, Status};

use crate::auth::PrincipalResolver;

/// Run the WebSocket bridge accept loop until the task is aborted (on shutdown).
/// Each accepted connection is handled on its own task; a per-connection failure
/// is logged, never fatal to the loop.
pub(crate) async fn serve_ws(
    listener: TcpListener,
    reader: Arc<dyn JournalReader>,
    tailer: Arc<dyn EventTailer>,
    global_tailer: Arc<dyn GlobalEventTailer>,
    token_tailer: Arc<dyn TokenTailer>,
    resolver: Arc<dyn PrincipalResolver>,
) {
    loop {
        let (stream, _peer) = match listener.accept().await {
            Ok(pair) => pair,
            Err(error) => {
                tracing::warn!(%error, "ws-bridge accept failed");
                continue;
            }
        };
        let reader = reader.clone();
        let tailer = tailer.clone();
        let global_tailer = global_tailer.clone();
        let token_tailer = token_tailer.clone();
        let resolver = resolver.clone();
        tokio::spawn(async move {
            if let Err(error) =
                handle_conn(stream, reader, tailer, global_tailer, token_tailer, resolver).await
            {
                tracing::debug!(%error, "ws-bridge connection ended");
            }
        });
    }
}

/// What the handshake parsed from the upgrade request: the frozen per-run
/// channel (`?instance=<hex16>&since=N`) or — Batch C — the run-unscoped GLOBAL
/// channel (a path ending in `/events/all`, `?since=N` only). Same token auth
/// either way; an OLD server 400s the new path ("missing ?instance"), which is
/// the client's honest not-wired signal.
enum StreamTarget {
    Run([u8; 16], u64),
    All(u64),
    /// PR-4.2 (T-STREAM1): the per-mote ADVISORY token channel (a path ending in
    /// `/tokens`, `?instance=<hex16>&mote=<hex32>&since=N`). Same token auth; an
    /// OLD server 400s the path ("missing ?instance"), the not-wired signal.
    Tokens([u8; 16], [u8; 32], u64),
}

/// Handshake (auth + parse) then stream live frames to one client.
// The tungstenite handshake closure returns `Result<_, ErrorResponse>`; that error
// type is large + API-fixed, so allow `result_large_err` for the whole fn.
#[allow(clippy::result_large_err)]
async fn handle_conn(
    stream: TcpStream,
    reader: Arc<dyn JournalReader>,
    tailer: Arc<dyn EventTailer>,
    global_tailer: Arc<dyn GlobalEventTailer>,
    token_tailer: Arc<dyn TokenTailer>,
    resolver: Arc<dyn PrincipalResolver>,
) -> Result<(), tokio_tungstenite::tungstenite::Error> {
    // The handshake callback runs synchronously during the upgrade; it stashes the
    // parsed target here on success. `Mutex` (not `RefCell`) so the enclosing
    // future stays `Send` for `tokio::spawn`.
    let parsed: Arc<Mutex<Option<StreamTarget>>> = Arc::new(Mutex::new(None));
    let parsed_cb = parsed.clone();

    let ws = tokio_tungstenite::accept_hdr_async(
        stream,
        move |req: &HsRequest, mut resp: HsResponse| {
            // Auth (uniform reject; no existence oracle) + selected subprotocol.
            let selected = authorize(req, resolver.as_ref())?;
            // The channel (per-run vs global) + cursor from the path + query.
            let target = parse_target(req).map_err(|msg| bad_request(&msg))?;
            *parsed_cb
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(target);
            // Echo the selected subprotocol so a browser that offered `bearer`
            // completes the handshake.
            if let Some(proto_name) = selected {
                resp.headers_mut().insert(
                    "sec-websocket-protocol",
                    HeaderValue::from_static(proto_name),
                );
            }
            Ok(resp)
        },
    )
    .await?;

    // The handshake callback always sets the target on success; an empty slot
    // can only mean a handshake path we don't expect — close quietly rather
    // than panic a connection task.
    let Some(target) = parsed
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
    else {
        return Ok(());
    };

    match target {
        StreamTarget::Run(instance_id, since_seq) => {
            pump(ws, reader, tailer, instance_id, since_seq).await
        }
        StreamTarget::All(since_seq) => pump_global(ws, reader, global_tailer, since_seq).await,
        StreamTarget::Tokens(instance_id, mote_id, since_seq) => {
            pump_tokens(ws, reader, token_tailer, instance_id, mote_id, since_seq).await
        }
    }
}

/// Drive the live tailer → JSON-over-WS, while reacting to the client's Close/Ping.
async fn pump(
    ws: tokio_tungstenite::WebSocketStream<TcpStream>,
    reader: Arc<dyn JournalReader>,
    tailer: Arc<dyn EventTailer>,
    instance_id: [u8; 16],
    since_seq: u64,
) -> Result<(), tokio_tungstenite::tungstenite::Error> {
    let (mut write, mut read) = ws.split();

    // Ownership is checked here (the tailer's first action); on failure close with
    // a reason rather than silently dropping.
    let mut events = match tailer.stream(reader, instance_id, since_seq) {
        Ok(stream) => stream,
        Err(status) => {
            let _ = write.send(Message::Close(Some(close_for(&status)))).await;
            return write.close().await;
        }
    };

    loop {
        tokio::select! {
            frame = events.next() => match frame {
                Some(Ok(frame)) => {
                    let json = serde_json::to_string(&WsFrame::from_proto(frame))
                        .unwrap_or_else(|_| "{}".to_string());
                    write.send(Message::Text(json)).await?;
                }
                // A terminal Status (CatchupRequired / internal / permission): close
                // with a reason so the client can resume from its last next_seq.
                Some(Err(status)) => {
                    let _ = write.send(Message::Close(Some(close_for(&status)))).await;
                    return write.close().await;
                }
                None => return write.close().await,
            },
            incoming = read.next() => match incoming {
                // Client closed, or the socket ended.
                Some(Ok(Message::Close(_))) | None => return Ok(()),
                // Ping is auto-ponged by reading; ignore other client messages
                // (this is a server→client push stream).
                Some(Ok(_)) => {}
                Some(Err(error)) => return Err(error),
            },
        }
    }
}

/// Resolve the caller from the upgrade request, reusing the R2 `PrincipalResolver`.
/// Returns the selected subprotocol to echo (`Some("bearer")` when the token came
/// via `Sec-WebSocket-Protocol`), or an `ErrorResponse` to reject the handshake.
// `ErrorResponse` (the tungstenite handshake-rejection type) is large; it is the
// API's fixed shape, so allow the lint rather than box it.
#[allow(clippy::result_large_err)]
fn authorize(
    req: &HsRequest,
    resolver: &dyn PrincipalResolver,
) -> Result<Option<&'static str>, ErrorResponse> {
    // Build a fresh tonic MetadataMap from the bearer credential (version-
    // independent — no coupling to the request's http crate version).
    let mut md = MetadataMap::new();
    let mut selected = None;

    if let Some(value) = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse().ok())
    {
        md.insert("authorization", value);
    } else if let Some(token) = subprotocol_bearer(req) {
        if let Ok(value) = format!("Bearer {token}").parse() {
            md.insert("authorization", value);
            selected = Some("bearer");
        }
    }

    resolver
        .resolve(&md)
        .map_err(|status| status_to_handshake_error(&status))?;
    Ok(selected)
}

/// Drive the GLOBAL live tailer → JSON-over-WS (the Batch C `/events/all`
/// channel) — the [`pump`] twin over [`WsGlobalFrame`]. Same Close/Ping
/// handling and CatchupRequired close protocol.
async fn pump_global(
    ws: tokio_tungstenite::WebSocketStream<TcpStream>,
    reader: Arc<dyn JournalReader>,
    global_tailer: Arc<dyn GlobalEventTailer>,
    since_seq: u64,
) -> Result<(), tokio_tungstenite::tungstenite::Error> {
    let (mut write, mut read) = ws.split();

    let mut events = match global_tailer.stream_all(reader, since_seq) {
        Ok(stream) => stream,
        Err(status) => {
            let _ = write.send(Message::Close(Some(close_for(&status)))).await;
            return write.close().await;
        }
    };

    loop {
        tokio::select! {
            frame = events.next() => match frame {
                Some(Ok(frame)) => {
                    let json = serde_json::to_string(&WsGlobalFrame::from_proto(frame))
                        .unwrap_or_else(|_| "{}".to_string());
                    write.send(Message::Text(json)).await?;
                }
                Some(Err(status)) => {
                    let _ = write.send(Message::Close(Some(close_for(&status)))).await;
                    return write.close().await;
                }
                None => return write.close().await,
            },
            incoming = read.next() => match incoming {
                Some(Ok(Message::Close(_))) | None => return Ok(()),
                Some(Ok(_)) => {}
                Some(Err(error)) => return Err(error),
            },
        }
    }
}

/// Drive the live TOKEN tailer → JSON-over-WS (the PR-4.2 `/tokens` channel) —
/// the [`pump`] twin over [`WsTokenChunk`]. The ownership gate (caller owns the
/// run AND the mote belongs to it) is the tailer's first action; the same
/// Close/Ping handling and CatchupRequired close protocol as [`pump`]. ADVISORY:
/// the committed result is the authority, so a dropped token stream loses nothing.
async fn pump_tokens(
    ws: tokio_tungstenite::WebSocketStream<TcpStream>,
    reader: Arc<dyn JournalReader>,
    token_tailer: Arc<dyn TokenTailer>,
    instance_id: [u8; 16],
    mote_id: [u8; 32],
    since_seq: u64,
) -> Result<(), tokio_tungstenite::tungstenite::Error> {
    let (mut write, mut read) = ws.split();

    let mut chunks = match token_tailer.stream(reader, instance_id, mote_id, since_seq) {
        Ok(stream) => stream,
        Err(status) => {
            let _ = write.send(Message::Close(Some(close_for(&status)))).await;
            return write.close().await;
        }
    };

    loop {
        tokio::select! {
            chunk = chunks.next() => match chunk {
                Some(Ok(chunk)) => {
                    let json = serde_json::to_string(&WsTokenChunk::from_proto(chunk))
                        .unwrap_or_else(|_| "{}".to_string());
                    write.send(Message::Text(json)).await?;
                }
                Some(Err(status)) => {
                    let _ = write.send(Message::Close(Some(close_for(&status)))).await;
                    return write.close().await;
                }
                None => return write.close().await,
            },
            incoming = read.next() => match incoming {
                Some(Ok(Message::Close(_))) | None => return Ok(()),
                Some(Ok(_)) => {}
                Some(Err(error)) => return Err(error),
            },
        }
    }
}

/// Extract the token from `Sec-WebSocket-Protocol: bearer, <token>` (the browser
/// path — browsers cannot set an `Authorization` header on a WebSocket).
fn subprotocol_bearer(req: &HsRequest) -> Option<String> {
    let raw = req.headers().get("sec-websocket-protocol")?.to_str().ok()?;
    let mut parts = raw.split(',').map(str::trim);
    if parts.next() == Some("bearer") {
        parts.next().filter(|t| !t.is_empty()).map(str::to_string)
    } else {
        None
    }
}

/// Route the upgrade request to its channel and parse the cursor:
/// - a path ending in `/events/all` → the GLOBAL channel (Batch C), `?since=N`
///   only (no `instance` — the stream is run-unscoped by design);
/// - every other path → the frozen per-run channel, `?instance=<hex16>&since=N`
///   (byte-identical behavior, including the 400 on a missing instance).
fn parse_target(req: &HsRequest) -> Result<StreamTarget, String> {
    let query = req.uri().query().unwrap_or("");
    let mut instance: Option<[u8; 16]> = None;
    let mut mote: Option<[u8; 32]> = None;
    let mut since: u64 = 0;
    for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
        match key.as_ref() {
            "instance" => instance = hex_decode_16(&value),
            "mote" => mote = hex_decode_32(&value),
            "since" => since = value.parse().unwrap_or(0),
            _ => {}
        }
    }
    let path = req.uri().path().trim_end_matches('/');
    if path.ends_with("/events/all") {
        return Ok(StreamTarget::All(since));
    }
    // PR-4.2 (T-STREAM1): the per-mote token channel. MUST be matched BEFORE the
    // per-run fallthrough below — otherwise a `/tokens` request mis-routes to the
    // event tail (a `mote`-less request would 400 there; a `mote`-bearing one
    // would silently stream the WRONG channel). Fail-closed on a missing arg.
    if path.ends_with("/tokens") {
        let instance =
            instance.ok_or_else(|| "missing or malformed ?instance=<hex16>".to_string())?;
        let mote = mote.ok_or_else(|| "missing or malformed ?mote=<hex32>".to_string())?;
        return Ok(StreamTarget::Tokens(instance, mote, since));
    }
    let instance = instance.ok_or_else(|| "missing or malformed ?instance=<hex16>".to_string())?;
    Ok(StreamTarget::Run(instance, since))
}

/// Map a resolver `Status` to a handshake rejection (uniform — no oracle).
fn status_to_handshake_error(status: &Status) -> ErrorResponse {
    let code = match status.code() {
        Code::Unauthenticated => StatusCode::UNAUTHORIZED,
        Code::PermissionDenied => StatusCode::FORBIDDEN,
        _ => StatusCode::BAD_REQUEST,
    };
    let mut resp = ErrorResponse::new(Some(status.message().to_string()));
    *resp.status_mut() = code;
    resp
}

/// A 400 handshake rejection for a malformed query.
fn bad_request(message: &str) -> ErrorResponse {
    let mut resp = ErrorResponse::new(Some(message.to_string()));
    *resp.status_mut() = StatusCode::BAD_REQUEST;
    resp
}

/// Map a terminal stream `Status` to a WS close frame so the client can react
/// (e.g. resume from its last `next_seq` on a CatchupRequired).
fn close_for(status: &Status) -> CloseFrame<'static> {
    let code = match status.code() {
        Code::ResourceExhausted => CloseCode::Again,
        Code::PermissionDenied | Code::Unauthenticated => CloseCode::Policy,
        _ => CloseCode::Error,
    };
    CloseFrame {
        code,
        reason: Cow::Owned(status.message().to_string()),
    }
}

// --- JSON hex wire DTO (mirrors the R4 audit DTO: ids/refs as 64-hex strings) ---

#[derive(serde::Serialize)]
struct WsFrame {
    seq: u64,
    deltas: Vec<WsDelta>,
    next_seq: u64,
    journal_boundary: bool,
}

#[derive(serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum WsDelta {
    Committed {
        seq: u64,
        mote_id: String,
        result_ref: String,
        nd_class: &'static str,
    },
    Failed {
        seq: u64,
        mote_id: String,
        reason_class: u32,
    },
    Repudiated {
        seq: u64,
        target_mote_id: String,
        target_committed_seq: u64,
    },
    EffectStaged {
        seq: u64,
        mote_id: String,
    },
    /// An unrecognized delta kind (forward-compat: a future proto variant).
    Unknown {
        seq: u64,
    },
}

impl WsFrame {
    fn from_proto(frame: proto::EventFrame) -> Self {
        Self {
            seq: frame.seq,
            deltas: frame.deltas.into_iter().map(WsDelta::from_proto).collect(),
            next_seq: frame.next_seq,
            journal_boundary: frame.journal_boundary,
        }
    }
}

impl WsDelta {
    fn from_proto(delta: proto::EventDelta) -> Self {
        let seq = delta.seq;
        match delta.kind {
            Some(proto::event_delta::Kind::Committed(c)) => Self::Committed {
                seq,
                mote_id: hex_encode(&c.mote_id),
                result_ref: hex_encode(&c.result_ref),
                nd_class: nd_str(c.nd_class),
            },
            Some(proto::event_delta::Kind::Failed(f)) => Self::Failed {
                seq,
                mote_id: hex_encode(&f.mote_id),
                reason_class: f.reason_class,
            },
            Some(proto::event_delta::Kind::Repudiated(r)) => Self::Repudiated {
                seq,
                target_mote_id: hex_encode(&r.target_mote_id),
                target_committed_seq: r.target_committed_seq,
            },
            Some(proto::event_delta::Kind::EffectStaged(e)) => Self::EffectStaged {
                seq,
                mote_id: hex_encode(&e.mote_id),
            },
            None => Self::Unknown { seq },
        }
    }
}

// --- The GLOBAL JSON wire DTO (Batch C `/events/all`) — the WsFrame twin with
// --- per-delta `instance_id` attribution + the `run_registered` variant. The
// --- per-run wire above is NOT touched.

#[derive(serde::Serialize)]
struct WsGlobalFrame {
    seq: u64,
    deltas: Vec<WsGlobalDelta>,
    next_seq: u64,
    journal_boundary: bool,
}

#[derive(serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum WsGlobalDelta {
    RunRegistered {
        seq: u64,
        instance_id: String,
        recipe_fingerprint: String,
        registered_unix_ms: u64,
    },
    Committed {
        seq: u64,
        instance_id: String,
        mote_id: String,
        result_ref: String,
        nd_class: &'static str,
    },
    Failed {
        seq: u64,
        instance_id: String,
        mote_id: String,
        reason_class: u32,
    },
    Repudiated {
        seq: u64,
        instance_id: String,
        target_mote_id: String,
        target_committed_seq: u64,
    },
    EffectStaged {
        seq: u64,
        instance_id: String,
        mote_id: String,
    },
    /// An unrecognized delta kind (forward-compat: a future proto variant).
    Unknown { seq: u64, instance_id: String },
}

impl WsGlobalFrame {
    fn from_proto(frame: proto::GlobalEventFrame) -> Self {
        Self {
            seq: frame.seq,
            deltas: frame
                .deltas
                .into_iter()
                .map(WsGlobalDelta::from_proto)
                .collect(),
            next_seq: frame.next_seq,
            journal_boundary: frame.journal_boundary,
        }
    }
}

impl WsGlobalDelta {
    fn from_proto(delta: proto::GlobalEventDelta) -> Self {
        let seq = delta.seq;
        // EMPTY before any registration → an empty hex string (honest).
        let instance_id = hex_encode(&delta.instance_id);
        match delta.kind {
            Some(proto::global_event_delta::Kind::RunRegistered(rr)) => Self::RunRegistered {
                seq,
                instance_id,
                recipe_fingerprint: hex_encode(&rr.recipe_fingerprint),
                registered_unix_ms: rr.registered_unix_ms,
            },
            Some(proto::global_event_delta::Kind::Committed(c)) => Self::Committed {
                seq,
                instance_id,
                mote_id: hex_encode(&c.mote_id),
                result_ref: hex_encode(&c.result_ref),
                nd_class: nd_str(c.nd_class),
            },
            Some(proto::global_event_delta::Kind::Failed(f)) => Self::Failed {
                seq,
                instance_id,
                mote_id: hex_encode(&f.mote_id),
                reason_class: f.reason_class,
            },
            Some(proto::global_event_delta::Kind::Repudiated(r)) => Self::Repudiated {
                seq,
                instance_id,
                target_mote_id: hex_encode(&r.target_mote_id),
                target_committed_seq: r.target_committed_seq,
            },
            Some(proto::global_event_delta::Kind::EffectStaged(e)) => Self::EffectStaged {
                seq,
                instance_id,
                mote_id: hex_encode(&e.mote_id),
            },
            None => Self::Unknown { seq, instance_id },
        }
    }
}

/// PR-4.2 (T-STREAM1): one ADVISORY token chunk on the WS `/tokens` channel.
/// `text_piece` is the NEW model bytes for this step, rendered as a lossy-UTF-8
/// string so the browser appends it directly to the chat bubble. A token may end
/// mid-codepoint (the lossy boundary is cosmetic and reconciled away the moment
/// the committed `result_ref` is fetched whole). `done` ends the stream.
#[derive(serde::Serialize)]
struct WsTokenChunk {
    seq: u64,
    mote_id: String,
    text_piece: String,
    done: bool,
}

impl WsTokenChunk {
    fn from_proto(chunk: proto::TokenChunk) -> Self {
        // Consume `text_piece` by value (no copy on the valid-UTF-8 path); fall
        // back to lossy decoding only when a token ends mid-codepoint.
        let text_piece = match String::from_utf8(chunk.text_piece) {
            Ok(s) => s,
            Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
        };
        Self {
            seq: chunk.seq,
            mote_id: hex_encode(&chunk.mote_id),
            text_piece,
            done: chunk.done,
        }
    }
}

/// Stable lowercase nd-class tag (matches the kx-audit wire vocabulary).
fn nd_str(nd: i32) -> &'static str {
    match proto::NdClass::try_from(nd) {
        Ok(proto::NdClass::Pure) => "pure",
        Ok(proto::NdClass::ReadOnlyNondet) => "read_only_nondet",
        Ok(proto::NdClass::WorldMutating) => "world_mutating",
        _ => "unspecified",
    }
}

/// Lowercase hex of arbitrary bytes (no `unwrap`).
fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

/// Decode exactly 32 lowercase/uppercase hex chars into a 16-byte instance id.
fn hex_decode_16(s: &str) -> Option<[u8; 16]> {
    if s.len() != 32 {
        return None;
    }
    let bytes = s.as_bytes();
    let mut out = [0u8; 16];
    for (i, slot) in out.iter_mut().enumerate() {
        let hi = hex_val(bytes[2 * i])?;
        let lo = hex_val(bytes[2 * i + 1])?;
        *slot = (hi << 4) | lo;
    }
    Some(out)
}

/// Decode exactly 64 lowercase/uppercase hex chars into a 32-byte mote id (PR-4.2).
fn hex_decode_32(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 {
        return None;
    }
    let bytes = s.as_bytes();
    let mut out = [0u8; 32];
    for (i, slot) in out.iter_mut().enumerate() {
        let hi = hex_val(bytes[2 * i])?;
        let lo = hex_val(bytes[2 * i + 1])?;
        *slot = (hi << 4) | lo;
    }
    Some(out)
}

fn hex_val(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_roundtrip_16() {
        let id = [0xabu8; 16];
        let s = hex_encode(&id);
        assert_eq!(s, "ab".repeat(16));
        assert_eq!(hex_decode_16(&s), Some(id));
        assert_eq!(hex_decode_16("ab"), None, "wrong length");
        assert_eq!(hex_decode_16(&"z".repeat(32)), None, "non-hex");
    }

    #[test]
    fn nd_str_maps_known_classes() {
        assert_eq!(nd_str(proto::NdClass::Pure as i32), "pure");
        assert_eq!(
            nd_str(proto::NdClass::ReadOnlyNondet as i32),
            "read_only_nondet"
        );
        assert_eq!(
            nd_str(proto::NdClass::WorldMutating as i32),
            "world_mutating"
        );
        assert_eq!(nd_str(999), "unspecified");
    }

    #[test]
    fn global_frame_serializes_with_instance_hex_and_run_registered() {
        // Pins the Batch C global wire: per-delta instance attribution (hex16,
        // EMPTY-honest) + the run_registered variant the per-run wire never has.
        let frame = proto::GlobalEventFrame {
            seq: 3,
            deltas: vec![
                proto::GlobalEventDelta {
                    seq: 1,
                    instance_id: vec![0x5a; 16],
                    kind: Some(proto::global_event_delta::Kind::RunRegistered(
                        proto::RunRegisteredDelta {
                            recipe_fingerprint: vec![0x6b; 32],
                            registered_unix_ms: 1234,
                        },
                    )),
                },
                proto::GlobalEventDelta {
                    seq: 2,
                    instance_id: vec![0x5a; 16],
                    kind: Some(proto::global_event_delta::Kind::Committed(
                        proto::CommittedDelta {
                            mote_id: vec![0xcd; 32],
                            result_ref: vec![0xef; 32],
                            nd_class: proto::NdClass::Pure as i32,
                        },
                    )),
                },
                proto::GlobalEventDelta {
                    seq: 3,
                    instance_id: Vec::new(), // pre-registration: EMPTY, honest
                    kind: None,              // forward-compat: unknown kind
                },
            ],
            next_seq: 3,
            journal_boundary: true,
        };
        let json = serde_json::to_value(WsGlobalFrame::from_proto(frame)).unwrap();
        assert_eq!(json["seq"], 3);
        assert_eq!(json["journal_boundary"], true);
        assert_eq!(json["deltas"][0]["type"], "run_registered");
        assert_eq!(json["deltas"][0]["instance_id"], "5a".repeat(16));
        assert_eq!(json["deltas"][0]["recipe_fingerprint"], "6b".repeat(32));
        assert_eq!(json["deltas"][0]["registered_unix_ms"], 1234);
        assert_eq!(json["deltas"][1]["type"], "committed");
        assert_eq!(json["deltas"][1]["instance_id"], "5a".repeat(16));
        assert_eq!(json["deltas"][1]["mote_id"], "cd".repeat(32));
        assert_eq!(json["deltas"][1]["nd_class"], "pure");
        assert_eq!(json["deltas"][2]["type"], "unknown");
        assert_eq!(json["deltas"][2]["instance_id"], "");
    }

    #[test]
    fn frame_serializes_with_hex_ids() {
        let frame = proto::EventFrame {
            seq: 5,
            deltas: vec![proto::EventDelta {
                seq: 5,
                kind: Some(proto::event_delta::Kind::Committed(proto::CommittedDelta {
                    mote_id: vec![0xcd; 32],
                    result_ref: vec![0xef; 32],
                    nd_class: proto::NdClass::Pure as i32,
                })),
            }],
            next_seq: 5,
            journal_boundary: true,
        };
        let json = serde_json::to_value(WsFrame::from_proto(frame)).unwrap();
        assert_eq!(json["seq"], 5);
        assert_eq!(json["journal_boundary"], true);
        assert_eq!(json["deltas"][0]["type"], "committed");
        assert_eq!(json["deltas"][0]["mote_id"], "cd".repeat(32));
        assert_eq!(json["deltas"][0]["result_ref"], "ef".repeat(32));
        assert_eq!(json["deltas"][0]["nd_class"], "pure");
    }
}
