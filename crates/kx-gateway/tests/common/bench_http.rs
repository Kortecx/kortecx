//! A hermetic HTTP MCP server for the benchmark — the suite's one tool that is not a
//! bundled mock speaking to itself.
//!
//! Every other bench tool is a stdio subprocess this repo also wrote: it cannot fail an
//! auth check, cannot paginate, cannot be slow, and cannot return a shape nobody
//! anticipated. So the numbers said nothing about the paths a real integration actually
//! exercises. This fixture is still local and still deterministic — a benchmark that
//! reached the public internet would measure the weather — but the runtime reaches it the
//! way it reaches anything external: over `HttpTransport`, through admission and dial
//! egress vetting, with the `Authorization` header resolved from the secret store at
//! dispatch and dropped afterwards.
//!
//! What it exercises that a bundled stdio tool cannot:
//!
//! - **Auth.** Every call must carry `Authorization: Bearer <token>`. A missing or wrong
//!   one is answered with a JSON-RPC error, not an empty result, so the failure is
//!   visible as a refusal rather than a silence. [`Captured::saw_auth`] records whether
//!   the header arrived WITHOUT storing its value, which is what lets a test prove
//!   injection happened without a secret entering the assertion.
//! - **Pagination.** `roster/page` returns two records at a time with a `next_cursor`.
//!   The fact the oracle asks for is on the SECOND page, so an agent that stops after one
//!   call cannot answer — the chain has to read a cursor out of one result and put it in
//!   the next call's arguments.
//! - **Latency.** Each response is delayed a little, so a tool round is a real interval
//!   in the timing split rather than something indistinguishable from zero.
//! - **Error shape.** An unknown roster id is a structured JSON-RPC error carrying a
//!   code, which is a different path from "the tool ran and found nothing".
//!
//! One connection per request, each on its own thread, so a slow handler never blocks the
//! accept loop. Stops on drop.

#![allow(dead_code)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The bearer token the fixture requires, as the FULL header value — matching the
/// credential convention the HTTP transport injects (the secret holds `Bearer <token>`,
/// not the bare token).
pub const BENCH_HTTP_BEARER: &str = "Bearer kx-bench-fleet-token";

/// The env var the serve resolves the credential from. Named like an operator's, because
/// it travels the operator's path: `RegisterMcpServer.credential_ref` holds this NAME,
/// and the value is read inside the transport at dispatch.
pub const BENCH_HTTP_CRED_ENV: &str = "KX_BENCH_FLEET_TOKEN";

/// The roster the fixture serves, in cursor order. The load-bearing fact — the callsign
/// on the SECOND page — exists nowhere else, so an answer carrying it is proof the run
/// paginated rather than guessed or stopped early.
const ROSTER: [(&str, &str, &str); 4] = [
    ("kestrel", "Ilma Rask", "PENNANT-04"),
    ("harrier", "Oyelaran Dube", "PENNANT-11"),
    ("merlin", "Sabine Okonkwo", "TRESTLE-62"),
    ("goshawk", "Teodor Vasilev", "TRESTLE-88"),
];

/// How many records one page returns — small enough that the answer is never on page one.
const PAGE_SIZE: usize = 2;

/// Per-response delay. Long enough to be a visible interval in the timing split, short
/// enough that sixteen tasks do not pay for it.
const RESPONSE_DELAY: Duration = Duration::from_millis(40);

/// What one request carried, reduced to what a test may assert on.
#[derive(Debug, Clone)]
pub struct Captured {
    /// The JSON-RPC method.
    pub method: String,
    /// Whether an `Authorization` header was present. Deliberately a bool: proving the
    /// credential arrived must not require the secret's value to appear in a test, and a
    /// fixture that echoed it would be a place for one to leak.
    pub saw_auth: bool,
    /// Whether that header held the expected value.
    pub auth_ok: bool,
    /// The `cursor` argument, when the caller sent one.
    pub cursor: Option<String>,
}

/// A hermetic in-process HTTP/1.1 MCP server on `127.0.0.1:0`.
pub struct BenchHttpServer {
    addr: SocketAddr,
    captured: Arc<Mutex<Vec<Captured>>>,
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl BenchHttpServer {
    /// Bind an ephemeral loopback port and start serving.
    #[must_use]
    pub fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind the bench http fixture");
        let addr = listener.local_addr().expect("the fixture has an address");
        listener
            .set_nonblocking(false)
            .expect("blocking accept loop");
        let captured = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let handle = {
            let captured = captured.clone();
            let stop = stop.clone();
            std::thread::spawn(move || {
                for conn in listener.incoming() {
                    if stop.load(Ordering::Relaxed) {
                        return;
                    }
                    let Ok(conn) = conn else { continue };
                    let captured = captured.clone();
                    std::thread::spawn(move || serve_one(&conn, &captured));
                }
            })
        };
        Self {
            addr,
            captured,
            stop,
            handle: Some(handle),
        }
    }

    /// The endpoint a connector registration dials.
    #[must_use]
    pub fn url(&self) -> String {
        format!("http://{}/mcp", self.addr)
    }

    /// The bare `host` the operator must allowlist to reach this fixture.
    #[must_use]
    pub fn host(&self) -> String {
        self.addr.ip().to_string()
    }

    /// Everything the fixture has been asked so far.
    #[must_use]
    pub fn captured(&self) -> Vec<Captured> {
        self.captured.lock().map(|c| c.clone()).unwrap_or_default()
    }
}

impl Drop for BenchHttpServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // Unblock the accept loop with a throwaway connection.
        let _ = TcpStream::connect(self.addr);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// Read one HTTP request, answer it, close.
fn serve_one(conn: &TcpStream, captured: &Arc<Mutex<Vec<Captured>>>) {
    let mut reader = BufReader::new(conn);
    let mut authorization: Option<String> = None;
    let mut content_length = 0usize;

    // Request line, then headers until the blank line.
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return;
    }
    loop {
        let mut header = String::new();
        match reader.read_line(&mut header) {
            Ok(0) => break,
            Ok(_) => {}
            Err(_) => return,
        }
        let trimmed = header.trim_end();
        if trimmed.is_empty() {
            break;
        }
        if let Some((name, value)) = trimmed.split_once(':') {
            let name = name.trim().to_ascii_lowercase();
            let value = value.trim().to_string();
            if name == "authorization" {
                authorization = Some(value);
            } else if name == "content-length" {
                content_length = value.parse().unwrap_or(0);
            }
        }
    }

    let mut body = vec![0u8; content_length];
    if content_length > 0 && reader.read_exact(&mut body).is_err() {
        return;
    }

    let request: serde_json::Value =
        serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null);
    let id = request.get("id").cloned().unwrap_or(serde_json::json!(1));
    let method = request
        .get("method")
        .and_then(|m| m.as_str())
        .unwrap_or("")
        .to_string();
    let cursor = request
        .get("params")
        .and_then(|p| p.get("arguments"))
        .and_then(|a| a.get("cursor"))
        .and_then(|c| c.as_str())
        .map(str::to_string);

    let auth_ok = authorization.as_deref() == Some(BENCH_HTTP_BEARER);
    if let Ok(mut c) = captured.lock() {
        c.push(Captured {
            method: method.clone(),
            saw_auth: authorization.is_some(),
            auth_ok,
            cursor: cursor.clone(),
        });
    }

    std::thread::sleep(RESPONSE_DELAY);

    let payload = if !auth_ok {
        // A real API refuses before it does any work, and says so in a shape the caller
        // can tell apart from an empty answer.
        error_response(
            &id,
            -32001,
            "unauthorized: a valid bearer credential is required",
        )
    } else {
        match method.as_str() {
            "initialize" => serde_json::json!({
                "jsonrpc": "2.0", "id": id,
                "result": {
                    "protocolVersion": "2025-06-18",
                    "capabilities": {},
                    "serverInfo": {"name": "kx-bench-fleet", "version": "1"}
                }
            }),
            "tools/list" => tools_list(&id),
            "tools/call" => tools_call(&id, &request, cursor.as_deref()),
            other => error_response(&id, -32601, &format!("unknown method {other:?}")),
        }
    };

    let body = serde_json::to_vec(&payload).unwrap_or_default();
    let mut out = conn;
    let _ = write!(
        out,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = out.write_all(&body);
    let _ = out.flush();
}

fn error_response(id: &serde_json::Value, code: i64, message: &str) -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0", "id": id,
        "error": {"code": code, "message": message}
    })
}

/// The two tools the fixture exposes. `roster/page` is the paginated read; `roster/get`
/// is the by-id read whose miss is a structured error rather than an empty result.
fn tools_list(id: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0", "id": id,
        "result": {"tools": [
            {
                "name": "page",
                "description": "List fleet crew records, two at a time. Returns `records` \
                                and, when more remain, a `next_cursor` to pass back in to \
                                read the following page.",
                "inputSchema": {
                    "type": "object",
                    "properties": {"cursor": {"type": "string"}},
                    "required": []
                }
            },
            {
                "name": "get",
                "description": "Look up one fleet crew record by its vessel id.",
                "inputSchema": {
                    "type": "object",
                    "properties": {"vessel": {"type": "string"}},
                    "required": ["vessel"]
                }
            }
        ]}
    })
}

fn tools_call(
    id: &serde_json::Value,
    request: &serde_json::Value,
    cursor: Option<&str>,
) -> serde_json::Value {
    let name = request
        .get("params")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())
        .unwrap_or("");
    match name {
        "page" => {
            let start: usize = cursor.and_then(|c| c.parse().ok()).unwrap_or(0);
            let end = (start + PAGE_SIZE).min(ROSTER.len());
            let records: Vec<serde_json::Value> = ROSTER[start.min(ROSTER.len())..end]
                .iter()
                .map(|(vessel, officer, callsign)| {
                    serde_json::json!({"vessel": vessel, "officer": officer, "callsign": callsign})
                })
                .collect();
            let mut result = serde_json::json!({"records": records});
            if end < ROSTER.len() {
                if let Some(obj) = result.as_object_mut() {
                    obj.insert("next_cursor".into(), serde_json::json!(end.to_string()));
                }
            }
            serde_json::json!({"jsonrpc": "2.0", "id": id, "result": result})
        }
        "get" => {
            let vessel = request
                .get("params")
                .and_then(|p| p.get("arguments"))
                .and_then(|a| a.get("vessel"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            match ROSTER.iter().find(|(v, _, _)| *v == vessel) {
                Some((v, officer, callsign)) => serde_json::json!({
                    "jsonrpc": "2.0", "id": id,
                    "result": {"vessel": v, "officer": officer, "callsign": callsign}
                }),
                None => error_response(id, -32004, &format!("no such vessel {vessel:?}")),
            }
        }
        other => error_response(id, -32601, &format!("unknown tool {other:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fact the oracle asks for must NOT be on the first page — otherwise a run that
    /// never paginated would satisfy it and the task would measure nothing.
    #[test]
    fn the_answer_is_not_on_the_first_page() {
        let first: Vec<&str> = ROSTER[..PAGE_SIZE].iter().map(|(_, _, c)| *c).collect();
        assert!(
            !first.contains(&"TRESTLE-62"),
            "the oracle's callsign must require a second call"
        );
        assert!(
            ROSTER[PAGE_SIZE..]
                .iter()
                .any(|(_, _, c)| *c == "TRESTLE-62"),
            "and it must actually be reachable on a later page"
        );
    }
}
