//! The `workflow` bench family's hermetic HTTP fixture + the stored workflow
//! definitions the family runs.
//!
//! The fixture is a plain path-routed HTTP server (NOT the MCP JSON-RPC fixture in
//! [`super::bench_http`] — a deterministic `http` STEP dials a plain endpoint, it does
//! not speak tool-discovery). Every oracle token the family scores on lives ONLY in
//! this fixture's route bodies: it is never in a task instruction, never in a stored
//! envelope, so a passing answer is underivable without the runtime actually making
//! the dial the workflow declared.
//!
//! Two routes carry the family's hardest claims:
//! - `/escort` REFUSES a request without the exact bearer (the credential the runtime
//!   must resolve BY NAME at dispatch) — a bearer-less dial commits a 401 body and the
//!   oracle fails honestly.
//! - `/depot` refuses every dial carrying the FIRST-seen `Idempotency-Key` forever.
//!   The worker's at-least-once redispatch re-sends the SAME identity (same key), so
//!   only a retry ladder that minted a FRESH attempt identity can ever read the code —
//!   the fixture proves the fresh-token-fresh-attempt claim, not a call counter.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use kx_proto::proto;
use kx_proto::proto::kx_gateway_client::KxGatewayClient;
use tonic::transport::Channel;

use super::bench_http::BENCH_HTTP_BEARER;

/// Every fixture-borne oracle token the family's expectations score on. The
/// provisioner asserts each is ABSENT from the stored envelope bytes — an envelope
/// that carried its own oracle would let a run "pass" without dialing anything.
pub const BENCH_WORKFLOW_ORACLES: [&str; 9] = [
    "Veiko Sarn",
    "HALYARD-11",
    "BOWLINE-42",
    "CLEAT-7",
    "WEIR-OPEN-31",
    "WEIR-SHUT-58",
    "GATE-HOLD-51",
    "EMBER-RELAY-19",
    "LANTERN-GREEN-88",
];

/// One request the fixture answered (or refused).
#[derive(Debug, Clone)]
pub struct RoutedCall {
    /// `GET` / `POST`.
    pub method: String,
    /// The dialed path (`/escort`, `/depot`, …).
    pub path: String,
    /// The `Idempotency-Key` header, when the caller sent one.
    pub idempotency_key: Option<String>,
    /// Whether the fixture refused this dial (401 or 503).
    pub refused: bool,
}

/// The plain-HTTP routed fixture. Ephemeral loopback port; thread-per-connection
/// (a parallel group dials concurrently); shut down on drop.
pub struct BenchRoutedServer {
    addr: SocketAddr,
    captured: Arc<Mutex<Vec<RoutedCall>>>,
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl BenchRoutedServer {
    /// Bind an ephemeral loopback port and start serving.
    #[must_use]
    pub fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind the routed bench fixture");
        let addr = listener.local_addr().expect("the fixture has an address");
        listener
            .set_nonblocking(false)
            .expect("blocking accept loop");
        let captured = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let handle = {
            let captured = captured.clone();
            let stop = stop.clone();
            // The depot's flake key lives with the accept loop: first-seen is a
            // property of the fixture's whole lifetime, not one connection's.
            let first_depot_key: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
            std::thread::spawn(move || {
                for conn in listener.incoming() {
                    if stop.load(Ordering::Relaxed) {
                        return;
                    }
                    let Ok(conn) = conn else { continue };
                    let captured = captured.clone();
                    let first_depot_key = first_depot_key.clone();
                    std::thread::spawn(move || route_one(&conn, &captured, &first_depot_key));
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

    /// The fixture's base URL (no trailing slash).
    #[must_use]
    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Everything the fixture has been asked so far.
    #[must_use]
    pub fn captured(&self) -> Vec<RoutedCall> {
        self.captured.lock().map(|c| c.clone()).unwrap_or_default()
    }
}

impl Drop for BenchRoutedServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // Unblock the accept loop with a throwaway connection.
        let _ = TcpStream::connect(self.addr);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// Read one HTTP request, answer by path, close.
fn route_one(
    conn: &TcpStream,
    captured: &Arc<Mutex<Vec<RoutedCall>>>,
    first_depot_key: &Arc<Mutex<Option<String>>>,
) {
    let mut reader = BufReader::new(conn);
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return;
    }
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();

    let mut authorization: Option<String> = None;
    let mut idempotency_key: Option<String> = None;
    let mut content_length = 0usize;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).is_err() || header.trim().is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            let value = value.trim().to_string();
            match name.to_ascii_lowercase().as_str() {
                "authorization" => authorization = Some(value),
                "idempotency-key" => idempotency_key = Some(value),
                "content-length" => content_length = value.parse().unwrap_or(0),
                _ => {}
            }
        }
    }
    // Consume the body so the client's write never sees a reset mid-send.
    if content_length > 0 {
        let mut body = vec![0u8; content_length.min(1 << 16)];
        let _ = reader.read_exact(&mut body);
    }

    let (status, body) = match (method.as_str(), path.as_str()) {
        // The credentialed record: the bearer is checked EXACTLY (the runtime resolved
        // it by name at dispatch). A miss commits a 401 body — visible, scored 0.
        ("GET", "/escort") => {
            if authorization.as_deref() == Some(BENCH_HTTP_BEARER) {
                ("200 OK", r#"{"vessel":"kestrel","officer":"Veiko Sarn"}"#)
            } else {
                ("401 Unauthorized", r#"{"error":"unauthorized"}"#)
            }
        }
        ("GET", "/muster/a") => ("200 OK", r#"{"callsign":"HALYARD-11"}"#),
        ("GET", "/muster/b") => ("200 OK", r#"{"callsign":"BOWLINE-42"}"#),
        ("GET", "/muster/c") => ("200 OK", r#"{"callsign":"CLEAT-7"}"#),
        ("GET", "/reading/high") => ("200 OK", r#"{"reading":87}"#),
        ("GET", "/reading/low") => ("200 OK", r#"{"reading":12}"#),
        ("GET", "/gate/open") => ("200 OK", r#"{"order":"WEIR-OPEN-31"}"#),
        ("GET", "/gate/shut") => ("200 OK", r#"{"order":"WEIR-SHUT-58"}"#),
        ("GET", "/hold") => ("200 OK", r#"{"order":"GATE-HOLD-51"}"#),
        // The identity-keyed flake (see the module doc).
        ("POST", "/depot") => {
            let key = idempotency_key.clone().unwrap_or_default();
            let refused = {
                let mut first = match first_depot_key.lock() {
                    Ok(g) => g,
                    Err(poisoned) => poisoned.into_inner(),
                };
                if first.is_none() {
                    *first = Some(key.clone());
                }
                first.as_deref() == Some(key.as_str())
            };
            if refused {
                (
                    "503 Service Unavailable",
                    r#"{"error":"depot refuses this key"}"#,
                )
            } else {
                ("200 OK", r#"{"code":"EMBER-RELAY-19"}"#)
            }
        }
        ("GET", "/beacon") => ("200 OK", r#"{"signal":"LANTERN-GREEN-88"}"#),
        ("GET", "/dead") => ("503 Service Unavailable", r#"{"error":"permanently down"}"#),
        _ => ("404 Not Found", r#"{"error":"no such route"}"#),
    };
    let refused = !status.starts_with("200");
    if let Ok(mut log) = captured.lock() {
        log.push(RoutedCall {
            method,
            path,
            idempotency_key,
            refused,
        });
    }

    let response = format!(
        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    let mut conn = conn;
    let _ = conn.write_all(response.as_bytes());
}

/// The seven stored workflow definitions the family runs, keyed by bench handle.
/// Every step is deterministic — the family measures the RUNTIME (carry, join,
/// predicate, timer, retry identity, failure placeholder), never the model.
#[must_use]
pub fn bench_workflow_envelopes(base: &str) -> Vec<(&'static str, Vec<u8>)> {
    use kx_gateway::eval_bench::{
        BENCH_WF_COND_HIGH_HANDLE, BENCH_WF_COND_LOW_HANDLE, BENCH_WF_CONTINUE_HANDLE,
        BENCH_WF_PARALLEL_HANDLE, BENCH_WF_RETRY_HANDLE, BENCH_WF_SEQUENTIAL_HANDLE,
        BENCH_WF_WAIT_HANDLE,
    };
    let canonical = |name: &str, blueprint: serde_json::Value| -> Vec<u8> {
        kx_app::WorkflowEnvelope::new(name, blueprint)
            .to_canonical_json()
            .expect("a bench workflow envelope canonicalizes")
    };

    // Sequential carry: credentialed http fetch → first_non_skip join over ONE parent
    // (a verbatim carry — the terminal's bytes ARE step 1's committed observation).
    // The env credential already carries its full header value, so the scheme is
    // empty (inject raw).
    let sequential = canonical(
        "bench sequential carry",
        serde_json::json!({
            "seed": 0,
            "steps": [
                { "kind": "http", "args": {
                    "url": format!("{base}/escort"),
                    "secret_name": super::bench_http::BENCH_HTTP_CRED_ENV,
                    "secret_scheme": ""
                } },
                { "kind": "pure", "params": { "kx.cond.join": "first_non_skip" } }
            ],
            "edges": [ { "parent": 0, "child": 1 } ]
        }),
    );

    // Parallel + join: three concurrent dials, a 3-of-3 quorum aggregate carrying
    // every callsign.
    let parallel = canonical(
        "bench parallel join",
        serde_json::json!({
            "seed": 0,
            "steps": [
                { "kind": "http", "args": { "url": format!("{base}/muster/a") } },
                { "kind": "http", "args": { "url": format!("{base}/muster/b") } },
                { "kind": "http", "args": { "url": format!("{base}/muster/c") } },
                { "kind": "pure", "params": { "kx.join.quorum": "3" } }
            ],
            "edges": [
                { "parent": 0, "child": 3 },
                { "parent": 1, "child": 3 },
                { "parent": 2, "child": 3 }
            ]
        }),
    );

    // The conditional sluice, in both directions. The PAIR is the oracle: a predicate
    // that never reads its parent emits the same order twice and fails one task; a
    // conditional that ran both arms leaves two survivors and fails the join outright.
    let sluice = |name: &str, source: &str| {
        canonical(
            name,
            serde_json::json!({
                "seed": 0,
                "steps": [
                    { "kind": "http", "args": { "url": format!("{base}/reading/{source}") } },
                    { "kind": "conditional", "params": {
                        "kx.cond.predicate": "{\"op\":\"contains\",\"value\":\":87\"}"
                    } },
                    { "kind": "http", "args": { "url": format!("{base}/gate/open") },
                      "params": { "kx.cond.skip_guard": "true", "kx.cond.arm": "then" } },
                    { "kind": "http", "args": { "url": format!("{base}/gate/shut") },
                      "params": { "kx.cond.skip_guard": "true", "kx.cond.arm": "else" } },
                    { "kind": "pure", "params": { "kx.cond.join": "first_non_skip" } }
                ],
                "edges": [
                    { "parent": 0, "child": 1 },
                    { "parent": 1, "child": 2, "edge": "control" },
                    { "parent": 1, "child": 3, "edge": "control" },
                    { "parent": 2, "child": 4 },
                    { "parent": 3, "child": 4 }
                ]
            }),
        )
    };

    // Durable wait: the hold order is committed BEFORE the timer; the wait step's
    // commit is a verbatim pass-through of it, ≥ 3 s later (the timers sentinel
    // reads the elapsed wall clock BY NAME).
    let wait = canonical(
        "bench wait then carry",
        serde_json::json!({
            "seed": 0,
            "steps": [
                { "kind": "http", "args": { "url": format!("{base}/hold") } },
                { "kind": "pure", "params": { "kx.wait.delay_ms": "3000" } }
            ],
            "edges": [ { "parent": 0, "child": 1 } ]
        }),
    );

    // Retry: the keyed-flake depot under retry{3, 500ms}. The terminal is the retry
    // LAUNCH itself — its committed bytes are the winning attempt's.
    let retry = canonical(
        "bench retry recovers",
        serde_json::json!({
            "seed": 0,
            "steps": [
                { "kind": "http",
                  "args": { "url": format!("{base}/depot"), "method": "POST", "body": "{}" },
                  "params": {
                      "kx.step.failure_mode": "retry",
                      "kx.step.retry_max": "3",
                      "kx.step.retry_backoff_ms": "500"
                  } }
            ]
        }),
    );

    // Continue: one branch is permanently down under `continue`; the 1-of-2 quorum
    // releases on the survivor and the aggregate must never leak the placeholder.
    let cont = canonical(
        "bench continue placeholder",
        serde_json::json!({
            "seed": 0,
            "steps": [
                { "kind": "http", "args": { "url": format!("{base}/beacon") } },
                { "kind": "http",
                  "args": { "url": format!("{base}/dead") },
                  "params": {
                      "kx.step.failure_mode": "continue",
                      "kx.step.retry_max": "1"
                  } },
                { "kind": "pure", "params": { "kx.join.quorum": "1" } }
            ],
            "edges": [
                { "parent": 0, "child": 2 },
                { "parent": 1, "child": 2 }
            ]
        }),
    );

    vec![
        (BENCH_WF_SEQUENTIAL_HANDLE, sequential),
        (BENCH_WF_PARALLEL_HANDLE, parallel),
        (
            BENCH_WF_COND_HIGH_HANDLE,
            sluice("bench sluice high", "high"),
        ),
        (BENCH_WF_COND_LOW_HANDLE, sluice("bench sluice low", "low")),
        (BENCH_WF_WAIT_HANDLE, wait),
        (BENCH_WF_RETRY_HANDLE, retry),
        (BENCH_WF_CONTINUE_HANDLE, cont),
    ]
}

/// Save the seven definitions and read each back. `false` (with the reason printed)
/// on any failure — the caller HARD-asserts: a silent save failure would score seven
/// 0s that read as the runtime failing, not the harness.
pub async fn provision_workflow_fixtures(c: &mut KxGatewayClient<Channel>, base: &str) -> bool {
    for (handle, envelope) in bench_workflow_envelopes(base) {
        let saved = c
            .save_workflow(proto::SaveWorkflowRequest {
                handle: handle.to_string(),
                envelope_json: envelope.clone(),
                source_digest: Vec::new(),
                lifecycle: String::new(),
            })
            .await;
        if let Err(e) = saved {
            eprintln!("eval-bench: workflow fixture {handle} failed to save: {e}");
            return false;
        }
        let got = match c
            .get_workflow(proto::GetWorkflowRequest {
                handle: handle.to_string(),
            })
            .await
        {
            Ok(resp) => resp.into_inner(),
            Err(e) => {
                eprintln!("eval-bench: workflow fixture {handle} failed read-back: {e}");
                return false;
            }
        };
        if !got.found || got.envelope_json != envelope {
            eprintln!(
                "eval-bench: workflow fixture {handle} read back wrong (found={})",
                got.found
            );
            return false;
        }
        // The leak gate: a stored definition that carried an oracle token would let
        // the family pass without the runtime doing the thing it measures.
        let stored = String::from_utf8_lossy(&got.envelope_json).into_owned();
        for oracle in BENCH_WORKFLOW_ORACLES {
            assert!(
                !stored.contains(oracle),
                "workflow fixture {handle} leaks oracle {oracle:?} into its stored envelope"
            );
        }
    }
    true
}
