//! [`McpTransport`] — the request/response seam, plus the M5.2a [`StdioTransport`].
//!
//! The transport is a trait so the M5.2b `ureq` streamable-HTTP impl drops in
//! behind it without touching [`crate::McpCapability`]. [`StdioTransport`] speaks
//! newline-delimited JSON-RPC to a subprocess MCP server over its stdin/stdout —
//! no network, no TLS. Credentials are injected into the child's environment
//! out-of-band (D81); the response read is **bounded** by the caller's size cap
//! (IMP-16) and **wall-clock-bounded** (a watchdog kills a hung server).

use std::ffi::OsString;
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

use crate::credential::CredentialRef;
use crate::errors::TransportError;

/// Fallback wall-clock budget when the warrant supplies none (`0`): 30 s. Keeps a
/// hung or chatty server from blocking a dispatch indefinitely while not failing a
/// legitimately slow tool that simply has no explicit ceiling.
const DEFAULT_WALL_CLOCK_MS: u64 = 30_000;

/// The MCP transport seam: one synchronous request/response round-trip.
///
/// Implementations carry no per-call mutable state (a fresh round-trip per
/// `invoke`), so `Send + Sync` is trivially satisfied — required because
/// [`crate::McpCapability`] is held behind a `Send + Sync` `Capability`.
pub trait McpTransport: Send + Sync {
    /// Send `request` (a complete JSON-RPC message, without a trailing newline) and
    /// return the server's raw response bytes.
    ///
    /// The implementation MUST bound the response read to at most
    /// `max_response_bytes + 1` bytes (so the decoder can detect an oversize body
    /// without the transport buffering an unbounded amount) and MUST abandon the
    /// call after `wall_clock_ms` (a `0` budget means "use a sane default").
    ///
    /// # Errors
    ///
    /// [`TransportError`] on spawn/connection failure, I/O failure, or timeout.
    fn round_trip(
        &self,
        request: &[u8],
        max_response_bytes: usize,
        wall_clock_ms: u64,
    ) -> Result<Vec<u8>, TransportError>;
}

/// A subprocess MCP transport: newline-delimited JSON-RPC over the child's
/// stdin/stdout.
///
/// M5.2a is **single-shot**: write one `tools/call` request, read one response.
/// The `initialize`/`initialized` handshake a stateful MCP server expects is a
/// documented forward seam (M5.2b); the bundled test server is handshake-free.
#[derive(Debug, Default)]
pub struct StdioTransport {
    program: OsString,
    args: Vec<OsString>,
    envs: Vec<(OsString, OsString)>,
    credentials: Vec<CredentialRef>,
}

impl StdioTransport {
    /// Build a transport that launches `program` as the MCP server subprocess.
    #[must_use]
    pub fn new(program: impl Into<OsString>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            envs: Vec::new(),
            credentials: Vec::new(),
        }
    }

    /// Append a command-line argument for the server subprocess.
    #[must_use]
    pub fn arg(mut self, arg: impl Into<OsString>) -> Self {
        self.args.push(arg.into());
        self
    }

    /// Set a plain (non-secret) environment variable on the server subprocess.
    #[must_use]
    pub fn env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.envs.push((key.into(), value.into()));
        self
    }

    /// Register a credential to inject into the subprocess environment out-of-band
    /// at dispatch time (D81). The secret value is read transiently when the child
    /// is spawned and is never stored on this struct.
    #[must_use]
    pub fn credential(mut self, credential: CredentialRef) -> Self {
        self.credentials.push(credential);
        self
    }
}

impl McpTransport for StdioTransport {
    fn round_trip(
        &self,
        request: &[u8],
        max_response_bytes: usize,
        wall_clock_ms: u64,
    ) -> Result<Vec<u8>, TransportError> {
        let mut cmd = Command::new(&self.program);
        cmd.args(&self.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        for (key, value) in &self.envs {
            cmd.env(key, value);
        }
        // Out-of-band secret injection (D81): read from the host env into the child
        // env; the secret never transits an EffectRequest / handle / journal.
        for credential in &self.credentials {
            credential.inject_into(&mut cmd);
        }

        let mut child = cmd
            .spawn()
            .map_err(|e| TransportError::Unreachable(e.to_string()))?;

        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| TransportError::Io("child stdin unavailable".to_string()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| TransportError::Io("child stdout unavailable".to_string()))?;

        // Bound the read to cap+1 bytes so an oversize body is detectable by the
        // decoder without the transport buffering it all.
        let read_cap = u64::try_from(max_response_bytes.saturating_add(1)).unwrap_or(u64::MAX);
        let request_owned = request.to_vec();
        let (tx, rx) = mpsc::channel();
        // BOTH the write and the read run on the worker thread, so the wall-clock
        // watchdog below covers the FULL round-trip. (If the write ran on the parent
        // and a server filled the OS pipe buffer without draining stdin, the parent
        // could block in `write_all` BEFORE the watchdog armed — an unkillable hang.)
        let reader = std::thread::spawn(move || {
            let write = stdin
                .write_all(&request_owned)
                .and_then(|()| stdin.write_all(b"\n"))
                .and_then(|()| stdin.flush());
            drop(stdin); // EOF to the server, regardless of write outcome
            if let Err(e) = write {
                let _ = tx.send(Err(e));
                return;
            }
            let mut buf = Vec::new();
            let outcome = stdout.take(read_cap).read_to_end(&mut buf);
            // The receiver may already be gone (timeout) — ignore a closed channel.
            let _ = tx.send(outcome.map(|_| buf));
        });

        let budget = Duration::from_millis(if wall_clock_ms == 0 {
            DEFAULT_WALL_CLOCK_MS
        } else {
            wall_clock_ms
        });

        let result = match rx.recv_timeout(budget) {
            Ok(Ok(bytes)) => Ok(bytes),
            Ok(Err(e)) => Err(TransportError::Io(e.to_string())),
            Err(RecvTimeoutError::Timeout) => Err(TransportError::Timeout { wall_clock_ms }),
            Err(RecvTimeoutError::Disconnected) => {
                Err(TransportError::Io("reader thread disconnected".to_string()))
            }
        };

        // Reap the child unconditionally (kill on the error/timeout paths so a hung
        // server cannot linger); the reader thread then observes EOF and finishes.
        if result.is_err() {
            let _ = child.kill();
        }
        let _ = child.wait();
        let _ = reader.join();

        result
    }
}
