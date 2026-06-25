//! [`OllamaClient`] — a thin blocking HTTP client over a pooled [`ureq::Agent`]
//! against the Ollama REST API.
//!
//! The agent is built **once** (its connection pool is reused across calls) with a
//! short connect timeout so a hung daemon can never block startup. Control-plane
//! calls (`/api/version`, `/api/tags`, `/api/ps`, keep-alive load/unload) carry a
//! fixed per-call timeout; the long `/api/generate` call runs its send+read on a
//! worker thread under a wall-clock watchdog (mirroring `kx-mcp`'s
//! `HttpTransport`), so the caller always returns within the warrant budget even if
//! the daemon slow-tricks.

use std::io::{BufRead, BufReader, Read};
use std::net::IpAddr;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

use kx_inference::TokenSink;

use crate::error::OllamaError;

/// Per-call timeout for the control-plane calls (`version`/`tags`/`ps`/keep-alive).
/// Short on purpose: these run at startup + on lifecycle controls, where a hung
/// daemon must not stall the gateway.
const CONTROL_TIMEOUT_MS: u64 = 2_000;

/// Connect timeout for every dial — a closed port fails fast (the probe-time
/// "Ollama absent" signal) instead of waiting on the OS default.
const CONNECT_TIMEOUT_MS: u64 = 1_500;

/// Slack added to the per-call ureq timeout for `/api/generate`, so a worker the
/// watchdog has already abandoned still self-terminates. Larger than the watchdog
/// grace so the watchdog wins the race and the caller sees a clean `Timeout`.
const WORKER_BACKSTOP_SLACK_MS: u64 = 5_000;

/// Scheduling slack the watchdog allows beyond the budget before declaring a
/// timeout (covers thread wake-up jitter).
const WATCHDOG_GRACE_MS: u64 = 250;

/// Hard cap on a generation response body (defense against an unbounded daemon).
const MAX_GENERATE_BYTES: u64 = 64 * 1024 * 1024;

/// The result of a single `/api/generate` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenOutcome {
    /// The generated completion text.
    pub text: String,
    /// The number of output tokens the daemon reported (`eval_count`); `0` when
    /// the daemon did not report it.
    pub eval_count: u32,
}

/// A model's `/api/show` metadata, fetched in one round-trip at discovery (PR-B2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShowMeta {
    /// The declared context window (`<arch>.context_length`), `0` when absent.
    pub context_length: u32,
    /// `true` iff the model declares vision (`/api/show` `capabilities ∋ "vision"`
    /// or a `projector_info` block). Display/discovery only (SN-8).
    pub vision: bool,
}

/// A blocking HTTP client for one Ollama daemon endpoint.
pub struct OllamaClient {
    agent: ureq::Agent,
    /// Normalized base URL with no trailing slash (e.g. `http://127.0.0.1:11434`).
    base: String,
}

impl std::fmt::Debug for OllamaClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OllamaClient")
            .field("base", &self.base)
            .finish_non_exhaustive()
    }
}

impl OllamaClient {
    /// Build a client for `base_url` (e.g. `http://127.0.0.1:11434`).
    ///
    /// **Security (SN-8).** The default is **loopback only**: a non-loopback host
    /// is refused unless `allow_remote` is `true` (the operator's explicit opt-in).
    /// The URL is operator config — never model / client / Mote-controlled.
    ///
    /// # Errors
    /// [`OllamaError::Refused`] if `base_url` is not an `http(s)` URL with a host,
    /// or if the host is non-loopback and `allow_remote` is `false`.
    pub fn new(base_url: &str, allow_remote: bool) -> Result<Self, OllamaError> {
        let trimmed = base_url.trim().trim_end_matches('/');
        if !(trimmed.starts_with("http://") || trimmed.starts_with("https://")) {
            return Err(OllamaError::Refused(format!(
                "endpoint must be an http(s) URL: {base_url}"
            )));
        }
        let host = host_of_base(trimmed)
            .ok_or_else(|| OllamaError::Refused(format!("endpoint URL has no host: {base_url}")))?;
        if !allow_remote && !is_loopback_host(&host) {
            return Err(OllamaError::Refused(format!(
                "non-loopback Ollama host {host} refused; set the allow-remote opt-in \
                 to dial a remote daemon"
            )));
        }
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_millis(CONNECT_TIMEOUT_MS))
            // No fixed read timeout: control calls set a short per-call timeout, and
            // generate is bounded by its own worker-thread watchdog.
            .build();
        Ok(Self {
            agent,
            base: trimmed.to_string(),
        })
    }

    /// `GET /api/version` — the reachability probe. Returns the daemon version.
    ///
    /// # Errors
    /// [`OllamaError::Unreachable`] when the daemon is not running (the host's
    /// auto-detect treats this as "absent → guide").
    pub fn version(&self) -> Result<String, OllamaError> {
        let body = self.get("/api/version")?;
        let value: serde_json::Value = parse_json(&body)?;
        Ok(value
            .get("version")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_string())
    }

    /// `GET /api/tags` — the installed-model tags (e.g. `gemma3:12b`).
    ///
    /// # Errors
    /// [`OllamaError::Unreachable`] / [`OllamaError::Status`] / [`OllamaError::Protocol`].
    pub fn tags(&self) -> Result<Vec<String>, OllamaError> {
        let body = self.get("/api/tags")?;
        let value: serde_json::Value = parse_json(&body)?;
        Ok(model_names(&value))
    }

    /// `GET /api/ps` — the models currently resident in the daemon's RAM/VRAM.
    ///
    /// # Errors
    /// [`OllamaError::Unreachable`] / [`OllamaError::Status`] / [`OllamaError::Protocol`].
    pub fn ps(&self) -> Result<Vec<String>, OllamaError> {
        let body = self.get("/api/ps")?;
        let value: serde_json::Value = parse_json(&body)?;
        Ok(model_names(&value))
    }

    /// `POST /api/show` — the model's declared context window (`model_info`
    /// `<arch>.context_length`). The body is `{ "model": <tag> }`. The returned
    /// number is the direct analogue of the GGUF `.context_length` the in-process
    /// llama backend reads, so `kx models list` shows the same kind of value for
    /// either engine. Carries the short control-plane timeout (it is called once
    /// per tag at startup — a hung daemon must not stall serving).
    ///
    /// # Errors
    /// [`OllamaError::Unreachable`] / [`OllamaError::Status`] / [`OllamaError::Protocol`]
    /// (the last also when the response carries no `*.context_length` key).
    pub fn show_context_length(&self, model: &str) -> Result<u32, OllamaError> {
        let value = self.show_raw(model)?;
        context_length_of(&value)
            .ok_or_else(|| OllamaError::Protocol("no context_length in /api/show".to_string()))
    }

    /// `POST /api/show` — the model's metadata in ONE round-trip: the declared
    /// context window AND whether the model is vision-capable (PR-B2). Used at
    /// discovery so a served vision tag is detected without a second `/api/show`
    /// call. Both fields honest-degrade — a missing context window is `0`, a model
    /// with no vision signal is `false`; the call still succeeds (only a transport /
    /// status / decode failure errors).
    ///
    /// # Errors
    /// [`OllamaError::Unreachable`] / [`OllamaError::Status`] / [`OllamaError::Protocol`].
    pub fn show_meta(&self, model: &str) -> Result<ShowMeta, OllamaError> {
        let value = self.show_raw(model)?;
        Ok(ShowMeta {
            context_length: context_length_of(&value).unwrap_or(0),
            vision: vision_of(&value),
        })
    }

    /// Shared `POST /api/show` body fetch + JSON parse (the control-plane timeout —
    /// it is called once per tag at startup, so a hung daemon must not stall serving).
    fn show_raw(&self, model: &str) -> Result<serde_json::Value, OllamaError> {
        let body = serde_json::json!({ "model": model });
        let bytes = to_body(&body)?;
        let url = format!("{}/api/show", self.base);
        let resp = self
            .agent
            .post(&url)
            .timeout(Duration::from_millis(CONTROL_TIMEOUT_MS))
            .set("Content-Type", "application/json")
            .send_bytes(&bytes)
            .map_err(classify)?;
        let text = resp
            .into_string()
            .map_err(|e| OllamaError::Protocol(e.to_string()))?;
        parse_json(&text)
    }

    /// Load (`keep_alive = -1`) or unload (`keep_alive = 0`) `model` via an
    /// empty-prompt `/api/generate` — the daemon's documented warm/evict control.
    ///
    /// # Errors
    /// [`OllamaError::Unreachable`] / [`OllamaError::Status`] / [`OllamaError::Protocol`].
    pub fn set_keep_alive(&self, model: &str, keep_alive: i64) -> Result<(), OllamaError> {
        let body = serde_json::json!({
            "model": model,
            "prompt": "",
            "keep_alive": keep_alive,
            "stream": false,
        });
        let bytes = to_body(&body)?;
        let url = format!("{}/api/generate", self.base);
        match self
            .agent
            .post(&url)
            .timeout(Duration::from_millis(CONTROL_TIMEOUT_MS))
            .set("Content-Type", "application/json")
            .send_bytes(&bytes)
        {
            Ok(_) => Ok(()),
            Err(e) => Err(classify(e)),
        }
    }

    /// `POST /api/embed` — embed `text` with `model`, returning the dense vector.
    ///
    /// # Errors
    /// [`OllamaError::Unreachable`] / [`OllamaError::Status`] / [`OllamaError::Protocol`].
    pub fn embed(&self, model: &str, text: &str) -> Result<Vec<f32>, OllamaError> {
        let body = serde_json::json!({ "model": model, "input": text });
        let bytes = to_body(&body)?;
        let url = format!("{}/api/embed", self.base);
        let resp = self
            .agent
            .post(&url)
            .timeout(Duration::from_millis(CONTROL_TIMEOUT_MS))
            .set("Content-Type", "application/json")
            .send_bytes(&bytes)
            .map_err(classify)?;
        let text = resp
            .into_string()
            .map_err(|e| OllamaError::Protocol(e.to_string()))?;
        let value: serde_json::Value = parse_json(&text)?;
        // `/api/embed` returns `{ "embeddings": [[..]] }` (batched); take the first.
        let first = value
            .get("embeddings")
            .and_then(serde_json::Value::as_array)
            .and_then(|rows| rows.first())
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| OllamaError::Protocol("no embeddings in response".to_string()))?;
        first
            .iter()
            .map(|v| {
                v.as_f64().map(f64_to_f32).ok_or_else(|| {
                    OllamaError::Protocol("non-numeric embedding element".to_string())
                })
            })
            .collect()
    }

    /// `POST /api/generate` (raw mode) — generate a completion for `prompt`.
    ///
    /// `raw: true` so the daemon tokenizes `prompt` verbatim (no second template
    /// pass): the serve path already rendered the chat prompt, exactly as the
    /// in-process llama backend consumes its rendered string. `options` is the
    /// Ollama `options` object (`num_predict` / temperature / seed / …). When `sink`
    /// is `Some`, the daemon streams NDJSON and each token piece is forwarded to
    /// the sink; the accumulated text is byte-identical to the non-streaming path.
    /// The whole call is bounded by `wall_clock_ms` via a worker-thread watchdog.
    ///
    /// # Errors
    /// [`OllamaError::Timeout`] on budget expiry, plus the transport / status /
    /// protocol classes.
    pub fn generate(
        &self,
        model: &str,
        prompt: &str,
        options: &serde_json::Value,
        wall_clock_ms: u64,
        images: &[String],
        sink: Option<TokenSink>,
    ) -> Result<GenOutcome, OllamaError> {
        let budget_ms = if wall_clock_ms == 0 {
            CONTROL_TIMEOUT_MS.max(30_000)
        } else {
            wall_clock_ms
        };
        let budget = Duration::from_millis(budget_ms);
        let worker_timeout =
            Duration::from_millis(budget_ms.saturating_add(WORKER_BACKSTOP_SLACK_MS));

        let streaming = sink.is_some();
        let mut body = serde_json::json!({
            "model": model,
            "prompt": prompt,
            "raw": true,
            "stream": streaming,
            "options": options,
        });
        // PR-B2 vision: a Multimodal dispatch passes base64-encoded image(s); they
        // ride the documented `/api/generate` `images` array. ABSENT (text dispatch)
        // ⇒ the key is omitted ⇒ the body is byte-identical to the pre-PR-B2 text
        // path. `raw: true` is preserved — the prompt is still the verbatim rendered
        // chat string; the daemon splices the image(s) per the model's projector.
        if !images.is_empty() {
            body["images"] = serde_json::json!(images);
        }
        let url = format!("{}/api/generate", self.base);
        let agent = self.agent.clone();

        let (tx, rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let outcome = run_generate(&agent, &url, worker_timeout, &body, sink.as_ref());
            let _ = tx.send(outcome);
        });

        match rx.recv_timeout(budget.saturating_add(Duration::from_millis(WATCHDOG_GRACE_MS))) {
            Ok(result) => {
                let _ = worker.join();
                result
            }
            // The worker self-terminates when its own ureq timeout fires; joining
            // here would re-block the caller past the budget, defeating the watchdog.
            Err(RecvTimeoutError::Timeout) => Err(OllamaError::Timeout { wall_clock_ms }),
            Err(RecvTimeoutError::Disconnected) => Err(OllamaError::Protocol(
                "generate worker disconnected".to_string(),
            )),
        }
    }

    /// `POST /api/pull` (stream) — download `tag` from the Ollama registry, invoking
    /// `on_progress(status, completed, total)` for each NDJSON progress object so the
    /// caller can surface live byte progress. Returns `Ok(())` once the daemon reports
    /// a terminal `success`.
    ///
    /// Model Control v2 (the "quick/easy" Ollama acquisition path). Resumable: the
    /// daemon resumes an interrupted pull server-side, so a re-issued pull continues
    /// from where it left off. NO short control timeout — a pull legitimately runs for
    /// minutes; the HOST drives this on a blocking task it owns (and can abandon).
    ///
    /// # Errors
    /// [`OllamaError::Unreachable`] / [`OllamaError::Status`] when the daemon is down /
    /// rejects the request; [`OllamaError::Protocol`] on a malformed stream, a pull
    /// `error` object (e.g. an unknown model), or a stream that ends without `success`.
    pub fn pull(
        &self,
        tag: &str,
        on_progress: &mut dyn FnMut(&str, u64, u64),
    ) -> Result<(), OllamaError> {
        let body = serde_json::json!({ "model": tag, "stream": true });
        let bytes = to_body(&body)?;
        let url = format!("{}/api/pull", self.base);
        let resp = self
            .agent
            .post(&url)
            .set("Content-Type", "application/json")
            .send_bytes(&bytes)
            .map_err(classify)?;
        // NDJSON: one small progress object per line, many lines. No body cap (the
        // lines are tiny); the daemon terminates the stream on success/error.
        let mut saw_success = false;
        for line in BufReader::new(resp.into_reader()).lines() {
            let line = line.map_err(|e| OllamaError::Protocol(e.to_string()))?;
            match parse_pull_line(&line)? {
                Some(PullTick::Error(message)) => {
                    return Err(OllamaError::Protocol(format!("pull failed: {message}")));
                }
                Some(PullTick::Progress {
                    status,
                    completed,
                    total,
                }) => on_progress(&status, completed, total),
                Some(PullTick::Success) => {
                    on_progress("success", 0, 0);
                    saw_success = true;
                }
                None => {}
            }
        }
        if saw_success {
            Ok(())
        } else {
            Err(OllamaError::Protocol(
                "pull stream ended without a success status".to_string(),
            ))
        }
    }

    /// `GET path` with the control-plane timeout, returning the response body.
    fn get(&self, path: &str) -> Result<String, OllamaError> {
        let url = format!("{}{path}", self.base);
        let resp = self
            .agent
            .get(&url)
            .timeout(Duration::from_millis(CONTROL_TIMEOUT_MS))
            .call()
            .map_err(classify)?;
        resp.into_string()
            .map_err(|e| OllamaError::Protocol(e.to_string()))
    }
}

/// Issue the POST and read the body (streaming NDJSON when `sink` is set). Runs on
/// the worker thread under the caller's wall-clock watchdog.
fn run_generate(
    agent: &ureq::Agent,
    url: &str,
    worker_timeout: Duration,
    body: &serde_json::Value,
    sink: Option<&TokenSink>,
) -> Result<GenOutcome, OllamaError> {
    let bytes = to_body(body)?;
    let resp = agent
        .post(url)
        .timeout(worker_timeout)
        .set("Content-Type", "application/json")
        .send_bytes(&bytes)
        .map_err(classify)?;
    let reader = resp.into_reader().take(MAX_GENERATE_BYTES);
    match sink {
        Some(sink) => read_stream(reader, sink),
        None => read_single(reader),
    }
}

/// Read a non-streaming `/api/generate` response (`stream:false`): one JSON object.
fn read_single(mut reader: impl Read) -> Result<GenOutcome, OllamaError> {
    let mut buf = String::new();
    reader
        .read_to_string(&mut buf)
        .map_err(|e| OllamaError::Protocol(e.to_string()))?;
    let value: serde_json::Value = parse_json(&buf)?;
    Ok(GenOutcome {
        text: value
            .get("response")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_string(),
        eval_count: eval_count_of(&value),
    })
}

/// Read a streaming `/api/generate` response (`stream:true`): NDJSON, one object
/// per line. Each `response` piece is forwarded to `sink`; the accumulated text +
/// the final `eval_count` are returned (byte-identical to the non-streaming path).
fn read_stream(reader: impl Read, sink: &TokenSink) -> Result<GenOutcome, OllamaError> {
    let mut text = String::new();
    let mut eval_count = 0u32;
    for line in BufReader::new(reader).lines() {
        let line = line.map_err(|e| OllamaError::Protocol(e.to_string()))?;
        if line.trim().is_empty() {
            continue;
        }
        let value: serde_json::Value = parse_json(&line)?;
        if let Some(piece) = value.get("response").and_then(serde_json::Value::as_str) {
            if !piece.is_empty() {
                sink(piece.as_bytes());
                text.push_str(piece);
            }
        }
        let count = eval_count_of(&value);
        if count > 0 {
            eval_count = count;
        }
        if value
            .get("done")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
        {
            break;
        }
    }
    Ok(GenOutcome { text, eval_count })
}

/// One parsed `/api/pull` NDJSON progress line.
enum PullTick {
    /// A download-progress object (`{status, completed?, total?}`).
    Progress {
        /// The daemon's status string (e.g. `"pulling <digest>"`, `"verifying sha256"`).
        status: String,
        /// Bytes downloaded so far (`0` when the daemon omits it for this line).
        completed: u64,
        /// Total bytes for the current layer (`0` when unknown).
        total: u64,
    },
    /// A pull `error` object — the pull failed (carries the daemon's message).
    Error(String),
    /// The terminal `{"status":"success"}` line.
    Success,
}

/// Parse one `/api/pull` NDJSON line into a [`PullTick`] (pure — unit-tested with
/// recorded daemon output, no live daemon). A blank line is `None` (skip).
///
/// # Errors
/// [`OllamaError::Protocol`] on a non-JSON line.
fn parse_pull_line(line: &str) -> Result<Option<PullTick>, OllamaError> {
    if line.trim().is_empty() {
        return Ok(None);
    }
    let value: serde_json::Value = parse_json(line)?;
    if let Some(message) = value.get("error").and_then(serde_json::Value::as_str) {
        return Ok(Some(PullTick::Error(message.to_string())));
    }
    let status = value
        .get("status")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    if status.eq_ignore_ascii_case("success") {
        return Ok(Some(PullTick::Success));
    }
    let completed = value
        .get("completed")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let total = value
        .get("total")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    Ok(Some(PullTick::Progress {
        status: status.to_string(),
        completed,
        total,
    }))
}

/// Extract `eval_count` (clamped into `u32`) from a generate response object.
fn eval_count_of(value: &serde_json::Value) -> u32 {
    value
        .get("eval_count")
        .and_then(serde_json::Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .unwrap_or(0)
}

/// Extract the model's declared context window from an `/api/show` response. The
/// daemon reports it under `model_info` as `"<arch>.context_length"` (e.g.
/// `gemma3.context_length`, `llama.context_length`, `qwen2.context_length`). Matched
/// by suffix (arch-agnostic — exactly like the GGUF reader keys on `.context_length`)
/// and clamped into `u32`. `None` when absent / non-numeric so the catalog honest-
/// degrades to `0` rather than fabricating a window.
fn context_length_of(value: &serde_json::Value) -> Option<u32> {
    value
        .get("model_info")
        .and_then(serde_json::Value::as_object)?
        .iter()
        .find(|(k, _)| k.ends_with(".context_length"))
        .and_then(|(_, v)| v.as_u64())
        .and_then(|n| u32::try_from(n).ok())
}

/// Whether an `/api/show` response declares vision (PR-B2). Ollama surfaces this two
/// ways depending on the model/daemon version: a top-level `"capabilities"` array
/// containing `"vision"` (newer daemons), OR a `"projector_info"` block (a model
/// with a bundled mmproj projector). Either signal ⇒ vision-capable. Conservative:
/// absent both ⇒ `false` (honest-degrade — never claim a capability the daemon
/// doesn't report).
fn vision_of(value: &serde_json::Value) -> bool {
    let has_capability = value
        .get("capabilities")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|caps| {
            caps.iter()
                .filter_map(serde_json::Value::as_str)
                .any(|c| c.eq_ignore_ascii_case("vision"))
        });
    let has_projector = value
        .get("projector_info")
        .is_some_and(|p| !p.is_null());
    has_capability || has_projector
}

/// Narrow a JSON `f64` embedding element to the `f32` embeddings are stored in.
#[allow(clippy::cast_possible_truncation)] // embeddings are f32; the daemon emits f64 JSON numbers
fn f64_to_f32(f: f64) -> f32 {
    f as f32
}

/// Parse a JSON body, mapping a decode failure to [`OllamaError::Protocol`].
fn parse_json(body: &str) -> Result<serde_json::Value, OllamaError> {
    serde_json::from_str(body).map_err(|e| OllamaError::Protocol(e.to_string()))
}

/// Serialize a request body (the workspace `ureq` has no `json` feature, so we
/// serialize ourselves and `send_bytes`). Serializing a `Value` is infallible in
/// practice; the `Result` is mapped fail-closed regardless.
fn to_body(value: &serde_json::Value) -> Result<Vec<u8>, OllamaError> {
    serde_json::to_vec(value).map_err(|e| OllamaError::Protocol(e.to_string()))
}

/// Collect the `name` of every entry under a `{ "models": [ { "name": .. } ] }`
/// response (`/api/tags` and `/api/ps` share this shape).
fn model_names(value: &serde_json::Value) -> Vec<String> {
    value
        .get("models")
        .and_then(serde_json::Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|m| m.get("name").and_then(serde_json::Value::as_str))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Map a `ureq::Error` to the typed [`OllamaError`] classing.
fn classify(err: ureq::Error) -> OllamaError {
    match err {
        ureq::Error::Status(code, _) => OllamaError::Status(code),
        ureq::Error::Transport(t) => OllamaError::Unreachable(t.to_string()),
    }
}

/// Extract the bare host from a normalized `http(s)://host[:port][/...]` base URL.
fn host_of_base(base: &str) -> Option<String> {
    let after_scheme = base
        .strip_prefix("http://")
        .or_else(|| base.strip_prefix("https://"))?;
    let authority = after_scheme.split('/').next().unwrap_or(after_scheme);
    // Drop any userinfo (`user:pass@host`).
    let authority = authority.rsplit('@').next().unwrap_or(authority);
    if authority.is_empty() {
        return None;
    }
    // Bracketed IPv6 literal: `[::1]:11434`.
    if let Some(rest) = authority.strip_prefix('[') {
        return rest
            .split(']')
            .next()
            .filter(|h| !h.is_empty())
            .map(str::to_string);
    }
    // `host:port` or bare `host` — split on the LAST colon to drop the port.
    Some(
        authority
            .rsplit_once(':')
            .map_or(authority, |(h, _)| h)
            .to_string(),
    )
}

/// `true` iff `host` names the loopback interface (`localhost`, `127.0.0.0/8`, `::1`).
fn is_loopback_host(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    host.parse::<IpAddr>()
        .map(|ip| ip.is_loopback())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_of_base_extracts_host() {
        assert_eq!(
            host_of_base("http://127.0.0.1:11434").as_deref(),
            Some("127.0.0.1")
        );
        assert_eq!(
            host_of_base("http://localhost").as_deref(),
            Some("localhost")
        );
        assert_eq!(
            host_of_base("https://ollama.example.com:443/x").as_deref(),
            Some("ollama.example.com")
        );
        assert_eq!(host_of_base("http://[::1]:11434").as_deref(), Some("::1"));
    }

    #[test]
    fn loopback_classification() {
        assert!(is_loopback_host("localhost"));
        assert!(is_loopback_host("127.0.0.1"));
        assert!(is_loopback_host("127.5.5.5"));
        assert!(is_loopback_host("::1"));
        assert!(!is_loopback_host("10.0.0.5"));
        assert!(!is_loopback_host("ollama.example.com"));
    }

    #[test]
    fn new_refuses_non_http_scheme() {
        let err = OllamaClient::new("ftp://127.0.0.1:11434", false).expect_err("non-http refused");
        assert!(matches!(err, OllamaError::Refused(_)));
    }

    #[test]
    fn new_refuses_non_loopback_without_optin() {
        let err = OllamaClient::new("http://10.0.0.5:11434", false)
            .expect_err("non-loopback without opt-in refused");
        assert!(matches!(err, OllamaError::Refused(_)));
    }

    #[test]
    fn new_allows_non_loopback_with_optin() {
        let c = OllamaClient::new("http://10.0.0.5:11434", true).expect("opt-in allows remote");
        assert_eq!(c.base, "http://10.0.0.5:11434");
    }

    #[test]
    fn new_allows_loopback() {
        let c = OllamaClient::new("http://127.0.0.1:11434/", false).expect("loopback ok");
        assert_eq!(c.base, "http://127.0.0.1:11434"); // trailing slash trimmed
    }

    #[test]
    fn context_length_of_reads_arch_keyed_window() {
        // Arch-agnostic: any `<arch>.context_length` key under model_info.
        let gemma = serde_json::json!({ "model_info": { "gemma3.context_length": 131_072 } });
        assert_eq!(context_length_of(&gemma), Some(131_072));
        let qwen = serde_json::json!({ "model_info": { "qwen2.context_length": 32_768 } });
        assert_eq!(context_length_of(&qwen), Some(32_768));
    }

    #[test]
    fn vision_of_reads_capability_array_and_projector() {
        // (a) the newer `capabilities` array signal (case-insensitive).
        let caps = serde_json::json!({ "capabilities": ["completion", "vision"] });
        assert!(vision_of(&caps));
        let caps_upper = serde_json::json!({ "capabilities": ["Completion", "Vision"] });
        assert!(vision_of(&caps_upper));
        // (b) the `projector_info` (bundled mmproj) signal.
        let proj = serde_json::json!({ "projector_info": { "general.architecture": "clip" } });
        assert!(vision_of(&proj));
    }

    #[test]
    fn vision_of_degrades_when_absent_or_text_only() {
        // A text-only model: no vision capability, no projector ⇒ false (never claim it).
        let text_only = serde_json::json!({ "capabilities": ["completion"] });
        assert!(!vision_of(&text_only));
        let bare = serde_json::json!({ "model_info": { "gemma3.context_length": 131_072 } });
        assert!(!vision_of(&bare));
        let null_proj = serde_json::json!({ "projector_info": serde_json::Value::Null });
        assert!(!vision_of(&null_proj));
    }

    #[test]
    fn pull_line_parses_recorded_api_pull_ndjson() {
        // B3: the `/api/pull` progress parser, driven by recorded daemon NDJSON
        // (no live daemon). The shape mirrors the documented `/api/pull` stream.
        assert!(parse_pull_line("   ").unwrap().is_none());
        assert!(matches!(
            parse_pull_line(r#"{"status":"pulling manifest"}"#).unwrap(),
            Some(PullTick::Progress {
                completed: 0,
                total: 0,
                ..
            })
        ));
        match parse_pull_line(
            r#"{"status":"pulling abc123","digest":"sha256:abc123","total":2142590208,"completed":241970}"#,
        )
        .unwrap()
        {
            Some(PullTick::Progress {
                status,
                completed,
                total,
            }) => {
                assert_eq!(status, "pulling abc123");
                assert_eq!(completed, 241_970);
                assert_eq!(total, 2_142_590_208);
            }
            _ => panic!("expected a progress tick"),
        }
        assert!(matches!(
            parse_pull_line(r#"{"status":"success"}"#).unwrap(),
            Some(PullTick::Success)
        ));
        match parse_pull_line(r#"{"error":"model 'nope' not found"}"#).unwrap() {
            Some(PullTick::Error(m)) => assert!(m.contains("not found")),
            _ => panic!("expected an error tick"),
        }
        // A non-JSON line is a protocol error, never a silent skip.
        assert!(parse_pull_line("not json").is_err());
    }

    #[test]
    fn context_length_of_degrades_when_absent_or_non_numeric() {
        // No `*.context_length` key ⇒ None (honest-degrade to 0 at the call site).
        let none = serde_json::json!({ "model_info": { "general.architecture": "gemma3" } });
        assert_eq!(context_length_of(&none), None);
        // A non-numeric value ⇒ None, never a fabricated window.
        let bad = serde_json::json!({ "model_info": { "gemma3.context_length": "lots" } });
        assert_eq!(context_length_of(&bad), None);
        // Missing model_info entirely ⇒ None.
        assert_eq!(context_length_of(&serde_json::json!({})), None);
    }
}
