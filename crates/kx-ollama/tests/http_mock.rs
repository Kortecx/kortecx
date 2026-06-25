//! End-to-end tests for `kx-ollama` against an in-test HTTP server (a
//! `std::net::TcpListener` — no external daemon). Covers the client's REST calls,
//! NDJSON streaming, the backend's warrant gates, lifecycle, and the security
//! refusals.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::pedantic,
    clippy::field_reassign_with_default
)]

use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use kx_inference::{InferenceBackend, InferenceError, InferenceInput, InferenceParams, TokenSink};
use kx_mote::ModelId;
use kx_ollama::{OllamaBackend, OllamaClient, OllamaError};
use kx_warrant::WarrantSpec;

/// Spawn an in-test HTTP server that routes by `(method, path, body)`. The handler
/// returns `(status, json_body)`. Detached: it serves connections until the process
/// exits (each test makes a bounded number of calls).
fn spawn_mock<F>(handler: F) -> String
where
    F: Fn(&str, &str, &str) -> (u16, String) + Send + Sync + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let addr = listener.local_addr().expect("addr");
    let handler = Arc::new(handler);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            let handler = handler.clone();
            std::thread::spawn(move || serve_one(stream, &*handler));
        }
    });
    format!("http://{addr}")
}

/// Serve a single request on `stream`: parse the request line + Content-Length body,
/// route, and write the response (Connection: close).
fn serve_one(
    mut stream: TcpStream,
    handler: &(impl Fn(&str, &str, &str) -> (u16, String) + ?Sized),
) {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    let header_end = loop {
        let n = match stream.read(&mut tmp) {
            Ok(0) | Err(_) => return,
            Ok(n) => n,
        };
        buf.extend_from_slice(&tmp[..n]);
        if let Some(pos) = find_subslice(&buf, b"\r\n\r\n") {
            break pos;
        }
        if buf.len() > 1 << 20 {
            return;
        }
    };
    let head = String::from_utf8_lossy(&buf[..header_end]).to_string();
    let mut lines = head.lines();
    let request_line = lines.next().unwrap_or("");
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("").to_string();
    let content_len: usize = head
        .lines()
        .find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.trim()
                .eq_ignore_ascii_case("content-length")
                .then(|| v.trim().parse::<usize>().ok())
                .flatten()
        })
        .unwrap_or(0);
    let mut body = buf[header_end + 4..].to_vec();
    while body.len() < content_len {
        let n = match stream.read(&mut tmp) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        body.extend_from_slice(&tmp[..n]);
    }
    let body = String::from_utf8_lossy(&body[..content_len.min(body.len())]).to_string();

    let (status, resp_body) = handler(&method, &path, &body);
    let response = format!(
        "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{resp_body}",
        resp_body.len()
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// The canonical Ollama route handler covering every endpoint the tests exercise.
fn ollama_routes(method: &str, path: &str, body: &str) -> (u16, String) {
    match (method, path) {
        ("GET", "/api/version") => (200, r#"{"version":"0.1.42"}"#.to_string()),
        ("GET", "/api/tags") => (
            200,
            r#"{"models":[{"name":"gemma3:12b"},{"name":"qwen2.5:3b"}]}"#.to_string(),
        ),
        ("GET", "/api/ps") => (200, r#"{"models":[{"name":"gemma3:12b"}]}"#.to_string()),
        ("POST", "/api/show") => {
            // Per-tag declared window; arch-keyed under model_info (arch-agnostic match).
            if body.contains("qwen2.5:3b") {
                (
                    200,
                    r#"{"model_info":{"qwen2.context_length":32768}}"#.to_string(),
                )
            } else {
                (
                    200,
                    r#"{"model_info":{"gemma3.context_length":131072}}"#.to_string(),
                )
            }
        }
        ("POST", "/api/embed") => (200, r#"{"embeddings":[[0.25,0.5,0.75]]}"#.to_string()),
        ("POST", "/api/generate") => {
            if !body.contains("\"raw\":true") {
                // keep-alive load/unload (empty-prompt control call).
                (200, r#"{"done":true}"#.to_string())
            } else if body.contains("\"stream\":true") {
                (
                    200,
                    "{\"response\":\"Hello\",\"done\":false}\n\
                     {\"response\":\" world\",\"done\":false}\n\
                     {\"response\":\"\",\"done\":true,\"eval_count\":5}"
                        .to_string(),
                )
            } else {
                (
                    200,
                    r#"{"response":"Hello world","done":true,"eval_count":5}"#.to_string(),
                )
            }
        }
        _ => (404, r#"{"error":"not found"}"#.to_string()),
    }
}

fn warrant_for(model: &str, max_out: u32) -> WarrantSpec {
    let mut w = WarrantSpec::default();
    w.model_route.model_id = ModelId(model.to_string());
    w.model_route.max_output_tokens = max_out;
    w.resource_ceiling.wall_clock_ms = 5_000;
    w
}

fn params(max_out: u32) -> InferenceParams {
    let mut p = InferenceParams::default();
    p.max_output_tokens = max_out;
    p
}

fn served(client: Arc<OllamaClient>) -> OllamaBackend {
    let mut models = BTreeSet::new();
    models.insert("gemma3:12b".to_string());
    OllamaBackend::new(client, models)
}

#[test]
fn version_tags_ps() {
    let base = spawn_mock(ollama_routes);
    let client = OllamaClient::new(&base, false).unwrap();
    assert_eq!(client.version().unwrap(), "0.1.42");
    assert_eq!(client.tags().unwrap(), vec!["gemma3:12b", "qwen2.5:3b"]);
    assert_eq!(client.ps().unwrap(), vec!["gemma3:12b"]);
}

#[test]
fn discover_filters_by_allowlist() {
    let base = spawn_mock(ollama_routes);
    let client = Arc::new(OllamaClient::new(&base, false).unwrap());
    let allow = vec!["gemma3:12b".to_string()];
    let backend = OllamaBackend::discover(client, Some(&allow)).unwrap();
    assert_eq!(backend.model_ids(), vec![ModelId("gemma3:12b".to_string())]);
    assert!(backend.supports(&ModelId("gemma3:12b".to_string())));
    assert!(!backend.supports(&ModelId("qwen2.5:3b".to_string())));
}

#[test]
fn discover_populates_context_len_from_show() {
    // `/api/show` per tag at discovery ⇒ the declared window reaches the catalog.
    // Per-tag + arch-agnostic: gemma3.* vs qwen2.* both resolve by `.context_length`.
    let base = spawn_mock(ollama_routes);
    let client = Arc::new(OllamaClient::new(&base, false).unwrap());
    let backend = OllamaBackend::discover(client, None).unwrap();
    assert_eq!(
        backend.context_len(&ModelId("gemma3:12b".to_string())),
        131072
    );
    assert_eq!(
        backend.context_len(&ModelId("qwen2.5:3b".to_string())),
        32768
    );
}

#[test]
fn discover_degrades_to_zero_ctx_when_show_unavailable() {
    // A daemon that serves tags but 404s `/api/show` ⇒ ctx stays 0; serving is never
    // blocked (the honest-degrade path).
    let base = spawn_mock(|method, path, body| {
        if (method, path) == ("POST", "/api/show") {
            return (404, r#"{"error":"not found"}"#.to_string());
        }
        ollama_routes(method, path, body)
    });
    let client = Arc::new(OllamaClient::new(&base, false).unwrap());
    let backend = OllamaBackend::discover(client, None).unwrap();
    assert_eq!(backend.context_len(&ModelId("gemma3:12b".to_string())), 0);
}

#[test]
fn dispatch_non_streaming_returns_completion() {
    let base = spawn_mock(ollama_routes);
    let client = Arc::new(OllamaClient::new(&base, false).unwrap());
    let backend = served(client);
    let out = backend
        .dispatch(
            &ModelId("gemma3:12b".to_string()),
            &InferenceInput::text("hi"),
            &params(64),
            &warrant_for("gemma3:12b", 64),
        )
        .unwrap();
    assert_eq!(out.bytes, b"Hello world");
    assert_eq!(out.output_tokens, 5);
    assert_eq!(out.backend_name, "kx-ollama");
}

#[test]
fn dispatch_streaming_feeds_the_token_sink() {
    let base = spawn_mock(ollama_routes);
    let client = Arc::new(OllamaClient::new(&base, false).unwrap());
    let backend = served(client);
    let pieces: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let sink_pieces = pieces.clone();
    let sink: TokenSink = Arc::new(move |p: &[u8]| {
        sink_pieces
            .lock()
            .unwrap()
            .push(String::from_utf8_lossy(p).to_string());
    });
    let out = backend
        .dispatch_streaming(
            &ModelId("gemma3:12b".to_string()),
            &InferenceInput::text("hi"),
            &params(64),
            &warrant_for("gemma3:12b", 64),
            Some(sink),
        )
        .unwrap();
    assert_eq!(out.bytes, b"Hello world");
    assert_eq!(out.output_tokens, 5);
    assert_eq!(*pieces.lock().unwrap(), vec!["Hello", " world"]);
}

#[test]
fn dispatch_refuses_wrong_model_route() {
    let base = spawn_mock(ollama_routes);
    let client = Arc::new(OllamaClient::new(&base, false).unwrap());
    let backend = served(client);
    let err = backend
        .dispatch(
            &ModelId("gemma3:12b".to_string()),
            &InferenceInput::text("hi"),
            &params(64),
            &warrant_for("some-other-model", 64),
        )
        .unwrap_err();
    assert!(matches!(err, InferenceError::WarrantDeniesModel { .. }));
}

#[test]
fn dispatch_refuses_over_ceiling() {
    let base = spawn_mock(ollama_routes);
    let client = Arc::new(OllamaClient::new(&base, false).unwrap());
    let backend = served(client);
    let err = backend
        .dispatch(
            &ModelId("gemma3:12b".to_string()),
            &InferenceInput::text("hi"),
            &params(100),
            &warrant_for("gemma3:12b", 10),
        )
        .unwrap_err();
    assert!(matches!(
        err,
        InferenceError::ScopeViolation {
            field: "max_output_tokens",
            ..
        }
    ));
}

#[test]
fn dispatch_refuses_unserved_model() {
    let base = spawn_mock(ollama_routes);
    let client = Arc::new(OllamaClient::new(&base, false).unwrap());
    let backend = served(client);
    let err = backend
        .dispatch(
            &ModelId("qwen2.5:3b".to_string()),
            &InferenceInput::text("hi"),
            &params(64),
            &warrant_for("qwen2.5:3b", 64),
        )
        .unwrap_err();
    assert!(matches!(err, InferenceError::ModelNotFound { .. }));
}

#[test]
fn dispatch_multimodal_on_non_vision_tag_fails_closed() {
    // PR-B2: a Multimodal dispatch against a tag that does NOT declare vision is
    // refused BEFORE egress (honest-degrade — answering without the image is a lie).
    // `served()` builds via `OllamaBackend::new` (no `/api/show` vision discovery), so
    // gemma3:12b is non-vision here.
    let base = spawn_mock(ollama_routes);
    let client = Arc::new(OllamaClient::new(&base, false).unwrap());
    let backend = served(client);
    let err = backend
        .dispatch(
            &ModelId("gemma3:12b".to_string()),
            &InferenceInput::Multimodal {
                text: "hi".to_string(),
                content_refs: Default::default(),
            },
            &params(64),
            &warrant_for("gemma3:12b", 64),
        )
        .unwrap_err();
    assert!(matches!(err, InferenceError::Unsupported { .. }));
}

#[test]
fn warm_and_evict_route_to_the_owning_engine() {
    let base = spawn_mock(ollama_routes);
    let client = Arc::new(OllamaClient::new(&base, false).unwrap());
    let backend = served(client);
    backend.warm(&ModelId("gemma3:12b".to_string())).unwrap();
    // gemma is resident per /api/ps ⇒ evict reports it was resident.
    assert!(backend.evict(&ModelId("gemma3:12b".to_string())).unwrap());
    // An unserved model is fail-closed.
    let err = backend
        .warm(&ModelId("qwen2.5:3b".to_string()))
        .unwrap_err();
    assert!(matches!(err, InferenceError::ModelNotFound { .. }));
}

#[test]
fn embed_returns_the_vector() {
    let base = spawn_mock(ollama_routes);
    let client = Arc::new(OllamaClient::new(&base, false).unwrap());
    use kx_inference::{EmbeddingBackend, EmbeddingPooling};
    let backend = served(client);
    let out = backend
        .dispatch_embedding(
            &ModelId("gemma3:12b".to_string()),
            "embed me",
            EmbeddingPooling::Mean,
            &warrant_for("gemma3:12b", 64),
        )
        .unwrap();
    assert_eq!(out.vector, vec![0.25, 0.5, 0.75]);
    assert_eq!(out.dim, 3);
}

#[test]
fn connection_refused_is_unreachable() {
    // Bind a listener to grab a free port, then drop it so the port is closed.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let client = OllamaClient::new(&format!("http://127.0.0.1:{port}"), false).unwrap();
    let err = client.version().unwrap_err();
    assert!(
        err.is_absent(),
        "a closed port must classify as absent/unreachable: {err:?}"
    );
    assert!(matches!(err, OllamaError::Unreachable(_)));
}

// --- PR-B2 vision (multimodal) ------------------------------------------------

use base64::Engine as _;
use kx_content::ContentRef;
use kx_inference::ContentFetcher;

/// The 8-byte PNG signature + a little filler — enough for `sniff_image_format`.
const PNG_BYTES: &[u8] = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x01, 0x02];

/// A minimal in-test content fetcher (the backend only needs `ContentFetcher`).
struct MapStore(std::collections::HashMap<ContentRef, Vec<u8>>);
impl ContentFetcher for MapStore {
    fn fetch(&self, r: &ContentRef) -> Option<Vec<u8>> {
        self.0.get(r).cloned()
    }
}

/// A warrant whose `mem_bytes` ceiling matches the vision recipe's 16 MiB (so the
/// image-byte gate passes for a small fixture and the sniff/encode path is reached).
fn vision_warrant(model: &str, max_out: u32) -> WarrantSpec {
    let mut w = warrant_for(model, max_out);
    w.resource_ceiling.mem_bytes = 16 << 20;
    w
}

/// A vision-aware route handler whose `/api/show` declares the vision capability for
/// gemma3:12b, and which RECORDS every `/api/generate` raw body into `sink` so a test
/// can assert the `images` array + the marker-stripped prompt.
fn vision_routes(
    sink: Arc<Mutex<Vec<String>>>,
) -> impl Fn(&str, &str, &str) -> (u16, String) + Send + Sync + 'static {
    move |method: &str, path: &str, body: &str| match (method, path) {
        ("GET", "/api/version") => (200, r#"{"version":"0.1.42"}"#.to_string()),
        ("GET", "/api/tags") => (200, r#"{"models":[{"name":"gemma3:12b"}]}"#.to_string()),
        ("POST", "/api/show") => (
            200,
            r#"{"capabilities":["completion","vision"],"model_info":{"gemma3.context_length":131072}}"#
                .to_string(),
        ),
        ("POST", "/api/generate") => {
            sink.lock().unwrap().push(body.to_string());
            (
                200,
                r#"{"response":"a cat","done":true,"eval_count":3}"#.to_string(),
            )
        }
        _ => (404, r#"{"error":"not found"}"#.to_string()),
    }
}

/// Build a vision-capable backend (discover populates the vision set from `/api/show`)
/// with `store` bound, against the recording `vision_routes` mock.
fn served_vision(store: Arc<dyn ContentFetcher>) -> (OllamaBackend, Arc<Mutex<Vec<String>>>) {
    let sink = Arc::new(Mutex::new(Vec::new()));
    let base = spawn_mock(vision_routes(sink.clone()));
    let client = Arc::new(OllamaClient::new(&base, false).unwrap());
    let backend = OllamaBackend::discover(client, None)
        .unwrap()
        .with_content_store(store);
    (backend, sink)
}

#[test]
fn discover_marks_vision_capable_tag() {
    let store = Arc::new(MapStore(std::collections::HashMap::new())) as Arc<dyn ContentFetcher>;
    let (backend, _sink) = served_vision(store);
    assert!(
        backend.is_vision(&ModelId("gemma3:12b".to_string())),
        "discovery must mark a tag whose /api/show declares the vision capability"
    );
}

#[test]
fn dispatch_multimodal_sends_base64_images_and_strips_marker() {
    use kx_inference::MEDIA_MARKER;
    let png_ref = ContentRef::of(PNG_BYTES);
    let mut map = std::collections::HashMap::new();
    map.insert(png_ref, PNG_BYTES.to_vec());
    let store = Arc::new(MapStore(map)) as Arc<dyn ContentFetcher>;
    let (backend, sink) = served_vision(store);

    let out = backend
        .dispatch(
            &ModelId("gemma3:12b".to_string()),
            &InferenceInput::Multimodal {
                text: format!("{MEDIA_MARKER}what is in this image?"),
                content_refs: std::iter::once(png_ref).collect(),
            },
            &params(64),
            &vision_warrant("gemma3:12b", 64),
        )
        .unwrap();
    assert_eq!(out.bytes, b"a cat");

    let bodies = sink.lock().unwrap();
    let body = bodies.last().expect("a /api/generate body was recorded");
    let json: serde_json::Value = serde_json::from_str(body).unwrap();
    // The image rides the `images` array as base64 of the raw bytes.
    let expected_b64 = base64::engine::general_purpose::STANDARD.encode(PNG_BYTES);
    assert_eq!(json["images"][0].as_str().unwrap(), expected_b64);
    // The media marker is stripped from the prompt (Ollama splices the image itself).
    let prompt = json["prompt"].as_str().unwrap();
    assert!(
        !prompt.contains(MEDIA_MARKER),
        "the <__media__> marker must not reach the Ollama prompt: {prompt}"
    );
    assert!(prompt.contains("what is in this image?"));
    // Raw image bytes must never enter the text.
    assert!(!prompt.contains("\u{89}PNG"));
}

#[test]
fn dispatch_multimodal_no_store_bound_fails_closed() {
    // A vision tag but no content store ⇒ Unsupported (cannot fetch the ref).
    let sink = Arc::new(Mutex::new(Vec::new()));
    let base = spawn_mock(vision_routes(sink));
    let client = Arc::new(OllamaClient::new(&base, false).unwrap());
    let backend = OllamaBackend::discover(client, None).unwrap(); // no with_content_store
    let err = backend
        .dispatch(
            &ModelId("gemma3:12b".to_string()),
            &InferenceInput::Multimodal {
                text: "hi".to_string(),
                content_refs: std::iter::once(ContentRef::of(PNG_BYTES)).collect(),
            },
            &params(64),
            &warrant_for("gemma3:12b", 64),
        )
        .unwrap_err();
    assert!(matches!(err, InferenceError::Unsupported { .. }));
}

#[test]
fn dispatch_multimodal_missing_ref_is_content_store_miss() {
    let store = Arc::new(MapStore(std::collections::HashMap::new())) as Arc<dyn ContentFetcher>;
    let (backend, _sink) = served_vision(store);
    let err = backend
        .dispatch(
            &ModelId("gemma3:12b".to_string()),
            &InferenceInput::Multimodal {
                text: "hi".to_string(),
                content_refs: std::iter::once(ContentRef::of(PNG_BYTES)).collect(),
            },
            &params(64),
            &warrant_for("gemma3:12b", 64),
        )
        .unwrap_err();
    assert!(matches!(err, InferenceError::ContentStoreMiss { .. }));
}

#[test]
fn dispatch_multimodal_oversize_is_scope_violation() {
    let big = vec![0u8; 32]; // > the tiny ceiling we set below; bytes are a valid PNG head
    let mut big_png = PNG_BYTES.to_vec();
    big_png.extend_from_slice(&big);
    let r = ContentRef::of(&big_png);
    let mut map = std::collections::HashMap::new();
    map.insert(r, big_png.clone());
    let store = Arc::new(MapStore(map)) as Arc<dyn ContentFetcher>;
    let (backend, _sink) = served_vision(store);
    let mut warrant = warrant_for("gemma3:12b", 64);
    warrant.resource_ceiling.mem_bytes = 4; // below the payload length
    let err = backend
        .dispatch(
            &ModelId("gemma3:12b".to_string()),
            &InferenceInput::Multimodal {
                text: "hi".to_string(),
                content_refs: std::iter::once(r).collect(),
            },
            &params(64),
            &warrant,
        )
        .unwrap_err();
    assert!(matches!(
        err,
        InferenceError::ScopeViolation {
            field: "image_bytes",
            ..
        }
    ));
}

#[test]
fn dispatch_multimodal_non_image_is_unsupported() {
    let not_image = b"this is plain text, not an image".to_vec();
    let r = ContentRef::of(&not_image);
    let mut map = std::collections::HashMap::new();
    map.insert(r, not_image);
    let store = Arc::new(MapStore(map)) as Arc<dyn ContentFetcher>;
    let (backend, _sink) = served_vision(store);
    let err = backend
        .dispatch(
            &ModelId("gemma3:12b".to_string()),
            &InferenceInput::Multimodal {
                text: "hi".to_string(),
                content_refs: std::iter::once(r).collect(),
            },
            &params(64),
            &vision_warrant("gemma3:12b", 64),
        )
        .unwrap_err();
    assert!(matches!(err, InferenceError::Unsupported { .. }));
}

#[test]
fn non_loopback_is_refused_without_optin() {
    let err = OllamaClient::new("http://10.0.0.5:11434", false).unwrap_err();
    assert!(matches!(err, OllamaError::Refused(_)));
    // Opt-in allows it (no dial happens at construction).
    assert!(OllamaClient::new("http://10.0.0.5:11434", true).is_ok());
}

/// Distinct counter to prove the mock is hit at most once per logical request
/// (no accidental retry storm from the client).
#[test]
fn single_request_per_call() {
    let count = Arc::new(AtomicUsize::new(0));
    let c = count.clone();
    let base = spawn_mock(move |m, p, b| {
        c.fetch_add(1, Ordering::SeqCst);
        ollama_routes(m, p, b)
    });
    let client = OllamaClient::new(&base, false).unwrap();
    client.version().unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 1);
}
