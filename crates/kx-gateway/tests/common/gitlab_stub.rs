//! A loopback stand-in for the GitLab REST API, shared by the deterministic and the live
//! environment-map proofs.
//!
//! It exists so BOTH environment entries are observable from the tool's own result with no
//! network: the base-URL entry decides whether a request arrives here at all, and the token
//! entry decides whether this answers `401` or serves the marker payload. One stub, two
//! independent witnesses.

#![allow(dead_code)]

use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

/// A loopback stand-in for the GitLab REST API.
///
/// Answers `401` unless the bearer token matches exactly, and otherwise returns one
/// schema-complete project whose `path_with_namespace` is `marker` — a string that exists
/// nowhere else, so a result carrying it could only have come through this listener.
pub struct GitLabStub {
    addr: SocketAddr,
    hits: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
}

impl GitLabStub {
    pub fn start(expected_token: &str, marker: &str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (h, s) = (hits.clone(), stop.clone());
        let (expected, marker) = (expected_token.to_string(), marker.to_string());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                if s.load(Ordering::SeqCst) {
                    return;
                }
                let Ok(mut stream) = stream else { continue };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut authorized = false;
                let mut line = String::new();
                // Read the request head; we only need the Authorization header.
                while reader.read_line(&mut line).unwrap_or(0) > 0 {
                    if line == "\r\n" || line == "\n" {
                        break;
                    }
                    if line.to_ascii_lowercase().starts_with("authorization:") {
                        authorized = line
                            .split_once(':')
                            .is_some_and(|(_, v)| v.trim() == format!("Bearer {expected}"));
                    }
                    line.clear();
                }
                h.fetch_add(1, Ordering::SeqCst);
                let response = if authorized {
                    // The shape `GitLabSearchResponseSchema` validates: an ARRAY of
                    // projects, with the count carried by the `X-Total` header.
                    let body = format!(
                        r#"[{{"id":4242,"name":"w3","path_with_namespace":"{marker}","visibility":"private","web_url":"http://127.0.0.1/w3","description":null,"ssh_url_to_repo":"git@127.0.0.1:w3.git","http_url_to_repo":"http://127.0.0.1/w3.git","created_at":"2026-08-06T00:00:00Z","last_activity_at":"2026-08-06T00:00:00Z","default_branch":"main"}}]"#
                    );
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nX-Total: 7\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                } else {
                    let body = r#"{"message":"401 Unauthorized"}"#;
                    format!(
                        "HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                };
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });
        Self { addr, hits, stop }
    }

    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.addr.port())
    }

    pub fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }
}

impl Drop for GitLabStub {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Unblock the accept loop so the thread can observe `stop`.
        let _ = std::net::TcpStream::connect(self.addr);
    }
}
