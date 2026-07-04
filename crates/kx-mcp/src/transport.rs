//! [`McpTransport`] — the request/response seam, plus the M5.2a [`StdioTransport`].
//!
//! The transport is a trait so the M5.2b `ureq` streamable-HTTP impl drops in
//! behind it without touching [`crate::McpCapability`]. [`StdioTransport`] speaks
//! newline-delimited JSON-RPC to a subprocess MCP server over its stdin/stdout —
//! no network, no TLS. Credentials are injected into the child's environment
//! out-of-band (D81); the response read is **bounded** by the caller's size cap
//! (IMP-16) and **wall-clock-bounded** (a watchdog kills a hung server).

use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use kx_warrant::SecretScope;

use crate::credential::CredentialRef;
use crate::decode::{
    decode_initialize_result, decode_tool_result, decode_tools_list, response_id, RemoteToolDecl,
    MAX_TOOL_RESULT_BYTES_DEFAULT,
};
use crate::errors::TransportError;
use crate::jsonrpc::{frame_initialize, frame_tools_call, frame_tools_list};
use crate::secret_store::{EnvSecretStore, SecretStore};
use crate::session::{McpSession, SessionError};

/// OOM backstop for a single newline-delimited response line a stateful stdio
/// session reads (16 MiB): far above any legitimate `tools/list` / `tools/call`
/// body, so a hostile server cannot force an unbounded allocation. The PRECISE
/// per-call bound is the decoder's `max_response_bytes` (the warrant ceiling);
/// this only caps what the background reader will buffer.
const SESSION_READ_HARD_CAP: u64 = 16 << 20;

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

    /// PR-6b-1: open a stateful [`McpSession`] (`initialize → tools/list →
    /// tools/call` over ONE live connection) — used by the external MCP gateway
    /// for discovery and by [`crate::McpSessionCapability`] for firing.
    ///
    /// Default: NOT supported — a single-shot transport fires via
    /// [`round_trip`](McpTransport::round_trip). [`StdioTransport`] and
    /// [`crate::HttpTransport`] override this with real sessions. Returning an
    /// error here (rather than a silent single-shot adapter) keeps the contract
    /// honest: a caller that asked for a session gets one or a typed refusal.
    fn open_session(&self) -> Result<Box<dyn McpSession>, TransportError> {
        Err(TransportError::Unreachable(
            "this transport does not support stateful MCP sessions".to_string(),
        ))
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

/// Resolve a connector program name to a concrete path just before spawn.
///
/// A curated connection (`kx connections add --provider slack`) stores a BARE
/// program name (`kx-connector-slack`). With no resolution that name is looked up
/// only on the OS `PATH`, so a user who installed `kx` but never put the bundled
/// connector on their `PATH` gets [`TransportError::Unreachable`] — the sidecar is
/// present beside `kx`, just not discoverable. We therefore look for the connector
/// first as a SIBLING of the running executable (the usual install layout: a
/// bundled connector next to `kx`), then fall back to the bare name (OS `PATH`, the
/// prior behaviour). A name that already contains a path separator (absolute or
/// relative — e.g. a test harness's `target/release/...`) is honoured verbatim and
/// never rewritten. This is a spawn-time hint only; the stored `program` keeps the
/// author's intent, and preferring the `kx`-sibling over a bare `PATH` lookup also
/// narrows the PATH-hijack surface for a bare name (GR8 security-positive).
fn resolve_program(program: &OsString) -> OsString {
    resolve_program_with(program, std::env::current_exe().ok().as_deref())
}

/// The pure core of [`resolve_program`], with the running-executable path injected
/// so it is unit-testable without touching the process's real `current_exe`.
fn resolve_program_with(program: &OsString, current_exe: Option<&Path>) -> OsString {
    // A qualified path (contains a separator) is the author's explicit choice — honour it.
    if Path::new(program).components().count() > 1 {
        return program.clone();
    }
    // A bare name: prefer a sibling of the running executable, else fall back to PATH.
    if let Some(dir) = current_exe.and_then(|exe| exe.parent()) {
        for name in sibling_candidates(program) {
            let candidate = dir.join(&name);
            if candidate.is_file() {
                return candidate.into_os_string();
            }
        }
    }
    program.clone()
}

/// The sibling filenames to probe for a bare connector name: the name as given,
/// plus the same name with the platform executable suffix (`.exe` on Windows) when
/// it is not already present. On Unix `EXE_SUFFIX` is empty ⇒ a single candidate.
fn sibling_candidates(program: &OsString) -> Vec<OsString> {
    let mut names = vec![program.clone()];
    let suffix = std::env::consts::EXE_SUFFIX;
    if !suffix.is_empty() && !program.to_string_lossy().ends_with(suffix) {
        let mut with_suffix = program.clone();
        with_suffix.push(suffix);
        names.push(with_suffix);
    }
    names
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
        let mut cmd = Command::new(resolve_program(&self.program));
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

    /// PR-6b-1: open a persistent stdio session — one long-lived subprocess across
    /// the `initialize → tools/list → tools/call` lifecycle (vs `round_trip`'s
    /// fresh process per call). Credentials are injected into the child env at
    /// spawn (D81). A background reader thread bounds each response line (a 16 MiB
    /// OOM backstop) so a hostile server cannot OOM the host.
    fn open_session(&self) -> Result<Box<dyn McpSession>, TransportError> {
        let mut cmd = Command::new(resolve_program(&self.program));
        cmd.args(&self.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        for (key, value) in &self.envs {
            cmd.env(key, value);
        }
        for credential in &self.credentials {
            credential.inject_into(&*self.secret_store, &mut cmd);
        }
        let session = StdioSession::spawn(cmd)?;
        Ok(Box::new(session))
    }
}

/// A persistent stdio MCP session (PR-6b-1): one subprocess, newline-delimited
/// JSON-RPC, across the lifecycle handshake + discovery + calls. A background
/// reader thread reads each response line (size-bounded) into a channel so every
/// request can be wall-clock-bounded via `recv_timeout` (mirrors the single-shot
/// `round_trip` watchdog discipline). Dropping the session kills + reaps the
/// child.
struct StdioSession {
    child: Child,
    stdin: ChildStdin,
    /// Each item is one response line's raw bytes (newline stripped), or a read error.
    lines: Receiver<std::io::Result<Vec<u8>>>,
    reader: Option<JoinHandle<()>>,
    next_id: u64,
    /// Latched on any I/O fault so a poisoned session refuses further requests.
    closed: bool,
}

impl StdioSession {
    /// Spawn `cmd` (already configured with stdio pipes + env/credentials) and
    /// start its background line reader.
    fn spawn(mut cmd: Command) -> Result<Self, TransportError> {
        let mut child = cmd
            .spawn()
            .map_err(|e| TransportError::Unreachable(e.to_string()))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| TransportError::Io("child stdin unavailable".to_string()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| TransportError::Io("child stdout unavailable".to_string()))?;
        let (tx, rx) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            let mut buf_reader = BufReader::new(stdout);
            loop {
                let mut line = Vec::new();
                // Bound each line so an oversize/never-terminating response cannot
                // OOM the host; the per-call decoder applies the precise cap.
                match (&mut buf_reader)
                    .take(SESSION_READ_HARD_CAP)
                    .read_until(b'\n', &mut line)
                {
                    Ok(0) => break, // EOF: the server closed stdout.
                    Ok(_) => {
                        if line.last() == Some(&b'\n') {
                            line.pop();
                        }
                        if tx.send(Ok(line)).is_err() {
                            break; // receiver gone (session dropped)
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Err(e));
                        break;
                    }
                }
            }
        });
        Ok(Self {
            child,
            stdin,
            lines: rx,
            reader: Some(reader),
            next_id: 1,
            closed: false,
        })
    }

    /// Write one framed request + newline, then read response lines under the
    /// wall-clock budget until one CORRELATES to `id` — SKIPPING any unsolicited
    /// JSON-RPC notification (no `id`) or stale response (different `id`) a
    /// spec-compliant server may interleave on stdout (logging / progress). The
    /// budget is a single deadline across all skipped lines. Latches `closed` on
    /// any fault.
    fn request_raw(
        &mut self,
        frame: &[u8],
        id: u64,
        wall_clock_ms: u64,
    ) -> Result<Vec<u8>, TransportError> {
        if self.closed {
            return Err(TransportError::Io("stdio session is closed".to_string()));
        }
        if let Err(e) = self
            .stdin
            .write_all(frame)
            .and_then(|()| self.stdin.write_all(b"\n"))
            .and_then(|()| self.stdin.flush())
        {
            self.closed = true;
            return Err(TransportError::Io(e.to_string()));
        }
        let total = Duration::from_millis(if wall_clock_ms == 0 {
            DEFAULT_WALL_CLOCK_MS
        } else {
            wall_clock_ms
        });
        let deadline = Instant::now() + total;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                self.closed = true;
                return Err(TransportError::Timeout { wall_clock_ms });
            }
            match self.lines.recv_timeout(remaining) {
                Ok(Ok(bytes)) => {
                    // A reply that correlates to our in-flight request wins; a
                    // notification (no id) or a stale/foreign id is skipped (this
                    // client never pipelines, so a foreign id is a server quirk,
                    // not a crossed response) — keep reading until the deadline.
                    if response_id(&bytes) == Some(id) {
                        return Ok(bytes);
                    }
                }
                Ok(Err(e)) => {
                    self.closed = true;
                    return Err(TransportError::Io(e.to_string()));
                }
                Err(RecvTimeoutError::Timeout) => {
                    self.closed = true;
                    return Err(TransportError::Timeout { wall_clock_ms });
                }
                Err(RecvTimeoutError::Disconnected) => {
                    self.closed = true;
                    return Err(TransportError::Io("stdio server closed".to_string()));
                }
            }
        }
    }

    fn next_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        id
    }
}

impl McpSession for StdioSession {
    fn initialize(&mut self, wall_clock_ms: u64) -> Result<String, SessionError> {
        let id = self.next_id();
        let frame = frame_initialize(id).map_err(|e| TransportError::Io(e.to_string()))?;
        let resp = self.request_raw(&frame, id, wall_clock_ms)?;
        // PR-6b-3: a well-formed result confirms the handshake AND carries the
        // server's negotiated protocolVersion (empty when omitted) — returned to
        // the caller (logged at dial), never a hard gate (old/new interop).
        Ok(decode_initialize_result(
            &resp,
            MAX_TOOL_RESULT_BYTES_DEFAULT,
        )?)
    }

    fn list_tools(
        &mut self,
        max_response_bytes: usize,
        wall_clock_ms: u64,
    ) -> Result<Vec<RemoteToolDecl>, SessionError> {
        let id = self.next_id();
        let frame = frame_tools_list(id).map_err(|e| TransportError::Io(e.to_string()))?;
        let resp = self.request_raw(&frame, id, wall_clock_ms)?;
        Ok(decode_tools_list(&resp, max_response_bytes)?)
    }

    fn call(
        &mut self,
        remote_name: &str,
        arguments: &[u8],
        max_response_bytes: usize,
        wall_clock_ms: u64,
        idempotency_key: Option<&[u8; 32]>,
    ) -> Result<Vec<u8>, SessionError> {
        // stdio has no dedup header; recovery dedup is the content-addressed
        // staging key (the broker idempotency token), so the key is unused here.
        let _ = idempotency_key;
        let id = self.next_id();
        let frame = frame_tools_call(id, remote_name, arguments)
            .map_err(|e| TransportError::Io(e.to_string()))?;
        let resp = self.request_raw(&frame, id, wall_clock_ms)?;
        Ok(decode_tool_result(&resp, max_response_bytes)?)
    }
}

impl Drop for StdioSession {
    fn drop(&mut self) {
        // Close stdin (EOF to the server) by dropping it implicitly at struct drop;
        // kill + reap so a lingering server cannot outlive the session.
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

#[cfg(test)]
mod resolve_tests {
    use super::{resolve_program_with, OsString, Path};
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A unique temp directory per test — kx-mcp keeps its dev-deps minimal (no
    /// `tempfile`), and `resolve_program_with` only reads the exe's PARENT, so a
    /// plain std dir under the system temp is enough. Uniqueness is process-id +
    /// a monotonic counter (no randomness needed).
    fn unique_dir(tag: &str) -> std::path::PathBuf {
        static N: AtomicU32 = AtomicU32::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("kx-mcp-resolve-{tag}-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn qualified_paths_are_used_verbatim() {
        // A relative path (contains a separator) is the author's explicit choice —
        // never rewritten, regardless of what sits beside the executable.
        let rel = OsString::from("target/release/kx-connector-slack");
        assert_eq!(
            resolve_program_with(&rel, Some(Path::new("/opt/kx/bin/kx"))),
            rel
        );
        // An absolute path — verbatim (this is exactly what the live witness harness
        // registers, so resolution must never disturb it).
        let abs = OsString::from("/usr/local/libexec/kx-connector-notion");
        assert_eq!(
            resolve_program_with(&abs, Some(Path::new("/opt/kx/bin/kx"))),
            abs
        );
    }

    #[test]
    fn bare_name_resolves_to_a_sibling_of_the_executable() {
        let dir = unique_dir("sibling");
        let connector = dir.join("kx-connector-slack");
        std::fs::write(&connector, b"#!/bin/sh\n").unwrap();
        // The exe file itself need not exist; only its parent directory is read.
        let exe = dir.join("kx");
        let got = resolve_program_with(&OsString::from("kx-connector-slack"), Some(&exe));
        assert_eq!(got, connector.into_os_string());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn bare_name_falls_back_to_the_bare_name_when_no_sibling() {
        let dir = unique_dir("no-sibling"); // empty ⇒ nothing beside the exe
        let exe = dir.join("kx");
        let bare = OsString::from("kx-connector-slack");
        // No sibling ⇒ returned unchanged (the OS PATH resolves it at spawn, the
        // prior behaviour — so a user who DID put it on PATH is unaffected).
        assert_eq!(resolve_program_with(&bare, Some(&exe)), bare);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn no_current_exe_leaves_the_bare_name() {
        let bare = OsString::from("kx-connector-slack");
        assert_eq!(resolve_program_with(&bare, None), bare);
    }

    #[test]
    fn a_same_named_directory_is_not_mistaken_for_the_binary() {
        // The `is_file()` guard rejects a directory that happens to share the name.
        let dir = unique_dir("dir-not-file");
        std::fs::create_dir_all(dir.join("kx-connector-slack")).unwrap();
        let exe = dir.join("kx");
        let bare = OsString::from("kx-connector-slack");
        assert_eq!(resolve_program_with(&bare, Some(&exe)), bare);
        std::fs::remove_dir_all(&dir).ok();
    }
}
