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
use std::sync::Arc;
use std::time::Duration;

use kx_warrant::SecretScope;

use crate::credential::CredentialRef;
use crate::errors::TransportError;
use crate::secret_store::{EnvSecretStore, SecretStore};

/// Fallback wall-clock budget when the warrant supplies none (`0`): 30 s. Keeps a
/// hung or chatty server from blocking a dispatch indefinitely while not failing a
/// legitimately slow tool that simply has no explicit ceiling. Shared with the
/// HTTP transport so both transports honour the same default budget.
pub(crate) const DEFAULT_WALL_CLOCK_MS: u64 = 30_000;

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
    /// `idempotency_key` is the run-scoped cross-boundary dedup token (D38 §1 /
    /// M1.2 `run_scoped_token`). A transport that maps to a protocol with a
    /// first-class dedup header (the HTTP `Idempotency-Key`) SHOULD send it, so a
    /// crash-recovery re-dispatch makes the *remote* effect exactly-once. A
    /// transport with no such header (stdio) ignores it — recovery dedup there is
    /// the content-addressed staging key, not a wire header.
    ///
    /// # Errors
    ///
    /// [`TransportError`] on spawn/connection failure, I/O failure, or timeout.
    fn round_trip(
        &self,
        request: &[u8],
        max_response_bytes: usize,
        wall_clock_ms: u64,
        idempotency_key: Option<&[u8; 32]>,
    ) -> Result<Vec<u8>, TransportError>;

    /// The [`SecretScope`] this transport will actually resolve at dispatch — the
    /// union of its configured credentials' [`SecretRef`](kx_warrant::SecretRef)s
    /// (D110.3). [`crate::McpCapability`] surfaces this as its
    /// `required_secret_scope`, which the broker gates `⊆ warrant.secret_scope`.
    /// Default: [`SecretScope::None`] (no credentials configured).
    fn declared_secret_scope(&self) -> SecretScope {
        SecretScope::None
    }
}

/// Build a [`SecretScope`] from an iterator of credential refs: `None` when empty,
/// else an `AllowList` of their [`SecretRef`](kx_warrant::SecretRef)s.
pub(crate) fn scope_of_credentials<'a>(
    creds: impl Iterator<Item = &'a CredentialRef>,
) -> SecretScope {
    let set: std::collections::BTreeSet<kx_warrant::SecretRef> =
        creds.map(|c| c.secret_ref().clone()).collect();
    if set.is_empty() {
        SecretScope::None
    } else {
        SecretScope::AllowList(set)
    }
}

/// A subprocess MCP transport: newline-delimited JSON-RPC over the child's
/// stdin/stdout.
///
/// M5.2a is **single-shot**: write one `tools/call` request, read one response.
/// The `initialize`/`initialized` handshake a stateful MCP server expects is a
/// documented forward seam (M5.2b); the bundled test server is handshake-free.
pub struct StdioTransport {
    program: OsString,
    args: Vec<OsString>,
    envs: Vec<(OsString, OsString)>,
    credentials: Vec<CredentialRef>,
    /// Resolves a `CredentialRef`'s `SecretRef` → value at spawn (D110.2).
    /// Defaults to [`EnvSecretStore`]; swap a cloud vault via
    /// [`StdioTransport::with_secret_store`].
    secret_store: Arc<dyn SecretStore>,
}

impl std::fmt::Debug for StdioTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Elide the secret store (not `Debug`-meaningful); credential identities
        // are already redaction-safe.
        f.debug_struct("StdioTransport")
            .field("program", &self.program)
            .field("args", &self.args)
            .field("credentials", &self.credentials)
            .finish_non_exhaustive()
    }
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
            secret_store: Arc::new(EnvSecretStore),
        }
    }

    /// Swap the [`SecretStore`] used to resolve credential secrets (D110.2).
    /// Defaults to [`EnvSecretStore`].
    #[must_use]
    pub fn with_secret_store(mut self, store: Arc<dyn SecretStore>) -> Self {
        self.secret_store = store;
        self
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
    fn declared_secret_scope(&self) -> SecretScope {
        scope_of_credentials(self.credentials.iter())
    }

    fn round_trip(
        &self,
        request: &[u8],
        max_response_bytes: usize,
        wall_clock_ms: u64,
        idempotency_key: Option<&[u8; 32]>,
    ) -> Result<Vec<u8>, TransportError> {
        // stdio has no wire header for a dedup key; recovery dedup is the
        // content-addressed staging key (the broker's idempotency token), so the
        // key is intentionally unused here.
        let _ = idempotency_key;
        let mut cmd = Command::new(&self.program);
        cmd.args(&self.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        for (key, value) in &self.envs {
            cmd.env(key, value);
        }
        // Out-of-band secret injection (D81): resolve through the SecretStore into
        // the child env; the secret never transits an EffectRequest / handle / journal.
        for credential in &self.credentials {
            credential.inject_into(&*self.secret_store, &mut cmd);
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
