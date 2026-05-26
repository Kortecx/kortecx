//! `MacOsSandboxExecutor` — macOS sandbox-exec / Seatbelt sibling of
//! `BwrapExecutor`. **PR 9a-hardening-2** wires the real
//! fork + `sandbox_init` + `execvp` path through `crate::spawn::spawn_body`.
//!
//! The executor either has a configured `body_path` (the binary the spawned
//! child will execvp into) or returns `BackendUnsupported`. Production
//! consumers configure `body_path` from the workflow's `logic_ref` resolved
//! at the runtime layer (P1.13+); integration tests construct
//! `MacOsSandboxExecutor::with_body(path)` directly against the
//! workspace's `kx-executor-pure-body` example binary.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use kx_content::ContentRef;
use kx_mote::Mote;
use kx_warrant::{ExecutorClass, WarrantSpec};

use crate::executor_trait::{MoteExecutionResult, MoteExecutor, MoteExecutorError, Rootfs};

/// macOS sandbox-exec / Seatbelt-based sandbox executor (macOS default per
/// D41).
#[derive(Debug, Default, Clone)]
pub struct MacOsSandboxExecutor {
    /// Absolute path to the body binary the spawned child will execvp into.
    /// When `None`, `run()` returns `BackendUnsupported` (the PR 9a
    /// skeleton shape preserved for back-compat with `default_executor()`).
    body_path: Option<PathBuf>,
    /// Absolute path to a file the body will read as its input (passed as
    /// `argv[1]`). When `None`, the integration test wires this per-Mote
    /// via `with_input_file`; production code derives it from the Mote's
    /// committed parents.
    input_path: Option<PathBuf>,
}

impl MacOsSandboxExecutor {
    /// Construct a new `MacOsSandboxExecutor` with no configured body
    /// (preserves the PR 9a `BackendUnsupported`-on-run shape).
    #[must_use]
    pub const fn new() -> Self {
        Self {
            body_path: None,
            input_path: None,
        }
    }

    /// Construct a `MacOsSandboxExecutor` with a configured body binary.
    /// The spawned child execvps into `body_path` after fork +
    /// `sandbox_init`.
    #[must_use]
    pub fn with_body(body_path: PathBuf) -> Self {
        Self {
            body_path: Some(body_path),
            input_path: None,
        }
    }

    /// Set the input file path passed as the body's `argv[1]`. Production
    /// code derives this from the Mote's committed parents; integration
    /// tests use this directly.
    #[must_use]
    pub fn with_input_file(mut self, input_path: PathBuf) -> Self {
        self.input_path = Some(input_path);
        self
    }
}

impl MoteExecutor for MacOsSandboxExecutor {
    fn run(
        &self,
        mote: &Mote,
        warrant: &WarrantSpec,
        env: Option<Rootfs>,
    ) -> Result<MoteExecutionResult, MoteExecutorError> {
        #[cfg(target_os = "macos")]
        {
            self.run_macos(mote, warrant, env.as_ref())
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (mote, warrant, env);
            Err(MoteExecutorError::BackendUnsupported {
                class: ExecutorClass::MacOsSandbox,
                reason: "MacOsSandbox backend only runs on target_os = \"macos\"".into(),
            })
        }
    }

    fn supports(&self, executor_class: ExecutorClass) -> bool {
        cfg!(target_os = "macos") && executor_class == ExecutorClass::MacOsSandbox
    }
}

#[cfg(target_os = "macos")]
impl MacOsSandboxExecutor {
    /// Real macOS spawn path. Builds the SBPL profile from the warrant,
    /// forks, loads the profile in the child via `sandbox_init`, execvps
    /// the body, reads stdout (the body's result_ref as 64 hex chars),
    /// waitpids, returns the result.
    fn run_macos(
        &self,
        _mote: &Mote,
        warrant: &WarrantSpec,
        _env: Option<&Rootfs>,
    ) -> Result<MoteExecutionResult, MoteExecutorError> {
        let body_path = self
            .body_path
            .as_ref()
            .ok_or(MoteExecutorError::BackendUnsupported {
                class: ExecutorClass::MacOsSandbox,
                reason:
                    "no body_path configured — construct via MacOsSandboxExecutor::with_body(path)"
                        .into(),
            })?;
        let input_path = self
            .input_path
            .as_ref()
            .ok_or(MoteExecutorError::Internal {
                reason: "no input_path configured — call .with_input_file(path) on the executor"
                    .into(),
            })?;

        // 1. Build the SBPL profile.
        let profile = profile_from_warrant(warrant);
        let profile_bytes = profile.as_bytes().to_vec();

        // 2. Body argv (argv[0] = body name, argv[1] = input file path).
        let body_path_str = body_path.to_string_lossy().into_owned();
        let input_path_str = input_path.to_string_lossy().into_owned();
        let argv = vec![body_path_str.clone(), input_path_str];

        // 3. Spawn — fork + pre-exec(sandbox_init) + execvp.
        let started_at_epoch_ms = now_epoch_ms();
        let outcome = crate::spawn::spawn_body(
            &body_path_str,
            &argv,
            Box::new(move || crate::spawn::load_profile(&profile_bytes)),
        )?;
        let finished_at_epoch_ms = now_epoch_ms();

        // 4. Parse the body's stdout: 64 hex chars → 32-byte ContentRef.
        if outcome.exit_code != 0 {
            return Err(MoteExecutorError::BodyExited {
                code: outcome.exit_code,
            });
        }
        let result_ref =
            parse_hex_ref(&outcome.stdout).map_err(|e| MoteExecutorError::Internal {
                reason: format!("body stdout parse: {e}"),
            })?;

        Ok(MoteExecutionResult {
            result_ref,
            started_at_epoch_ms,
            finished_at_epoch_ms,
        })
    }
}

fn now_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| {
            #[allow(clippy::cast_possible_truncation)]
            let ms = d.as_millis() as u64;
            ms
        })
        .unwrap_or(0)
}

/// Parse 64 lowercase-hex bytes (the body's stdout shape) into a
/// `ContentRef`. Trailing whitespace is tolerated (some bodies emit a
/// trailing newline despite the contract).
fn parse_hex_ref(bytes: &[u8]) -> Result<ContentRef, String> {
    // Skip trailing whitespace.
    let mut end = bytes.len();
    while end > 0 && bytes[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    let trimmed = &bytes[..end];
    if trimmed.len() != 64 {
        return Err(format!(
            "expected 64 hex chars, got {} bytes: {:?}",
            trimmed.len(),
            String::from_utf8_lossy(trimmed)
        ));
    }
    let mut out = [0u8; 32];
    for (i, chunk) in trimmed.chunks_exact(2).enumerate() {
        let hi = hex_nibble(chunk[0])?;
        let lo = hex_nibble(chunk[1])?;
        out[i] = (hi << 4) | lo;
    }
    Ok(ContentRef::from_bytes(out))
}

fn hex_nibble(b: u8) -> Result<u8, String> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'A'..=b'F' => Ok(b - b'A' + 10),
        _ => Err(format!("non-hex byte {b}")),
    }
}

/// Compiled SBPL bytes ready to feed to `sandbox-exec`'s policy loader. Per
/// D46, this is a newtype over `Vec<u8>` so accidental string concatenation
/// is forbidden; the inner bytes are SBPL S-expression source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SbplProfile(Vec<u8>);

impl SbplProfile {
    /// The byte representation ready for `sandbox_init`-equivalent.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// The length in bytes of the SBPL source.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the profile is empty (always `false` for `profile_from_warrant`
    /// output since the deny-default template is non-empty).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// The compile-time-embedded deny-default template (D46 §5).
///
/// Imports Apple's baseline `system.sb` so Rust-runtime startup work
/// (stack-guard-page mmap, mach-lookup of `com.apple.dyld`, etc.) can run
/// before the per-axis allowlists narrow the permitted surface. Without
/// `system.sb` the body crashes at thread startup with SIGABRT inside
/// `mmap(PROT_NONE)` for the stack guard page.
///
/// `system.sb` lives at `/System/Library/Sandbox/Profiles/system.sb` on
/// macOS 10.5+; `sandbox_init` resolves the relative reference. If a future
/// macOS removes the import (Apple has been deprecating `sandbox-exec`
/// piecemeal), `sandbox_init` returns an error and the executor surfaces
/// `MoteExecutorError::SandboxLoadFailed` — the correct corpus-aligned
/// failure mode.
const DENY_DEFAULT_TEMPLATE: &[u8] = b"(version 1)\n(import \"system.sb\")\n(deny default)\n";

/// Pure / total / deterministic mapping from a `WarrantSpec` to an
/// `SbplProfile` per D46.
///
/// Pure / total / deterministic: same `WarrantSpec` in → byte-identical
/// `SbplProfile` out. No I/O; no clocks; no `Result` (every WarrantSpec is
/// representable — under-declared axes produce the strictest possible rule
/// via the deny-default template).
///
/// **Per-axis mapping (D46 §6):**
/// - `fs_scope.mounts` — per entry: `ReadOnly` →
///   `(allow file-read* (subpath "<path>"))`; `ReadWrite` →
///   `(allow file-read* ...)` + `(allow file-write* ...)`; `ExecOnly` →
///   `(allow file-read-metadata ...)` + `(allow process-exec ...)`. Paths
///   are emitted via the BTreeMap's canonical iteration order so the
///   resulting bytes are deterministic.
/// - `net_scope` — `None` → no rules (deny-default holds);
///   `EgressAllowlist(hosts)` → per-host
///   `(allow network-outbound (remote ip "<host>:*"))` emitted in
///   BTreeSet iteration order.
/// - `syscall_profile_ref` — opaque per-platform. The reference is NOT
///   resolved here (the content store call lives in `MacOsSandboxExecutor::
///   run`); the profile carries a `;; syscall-profile-ref: <hex>` comment
///   line for audit-trail visibility. The PR 9a-hardening follow-up to
///   THIS PR adds resolution + body-byte appending.
/// - `resource_ceiling` — NOT in the SBPL profile (D46 §6.4: SBPL = access
///   control; `LocalResourceManager` via `setrlimit` = resource control).
/// - `executor_class` — tag-only; no profile-affecting semantic.
///
/// # Examples
///
/// ```
/// use kx_executor::backends::macos_sandbox::profile_from_warrant;
/// use kx_warrant::{ExecutorClass, FsScope, MoteClass, NetScope, ModelRoute,
///     ResourceCeiling, WarrantSpec};
/// use kx_content::ContentRef;
/// use kx_mote::ModelId;
/// use std::collections::BTreeSet;
///
/// let warrant = WarrantSpec {
///     mote_class: MoteClass::Pure,
///     nd_class: MoteClass::Pure,
///     fs_scope: FsScope::empty(),
///     net_scope: NetScope::None,
///     syscall_profile_ref: ContentRef::from_bytes([0; 32]),
///     tool_grants: BTreeSet::new(),
///     model_route: ModelRoute {
///         model_id: ModelId("local".into()),
///         max_input_tokens: 0,
///         max_output_tokens: 0,
///         max_calls: 0,
///     },
///     resource_ceiling: ResourceCeiling {
///         cpu_milli: 0, mem_bytes: 0, wall_clock_ms: 0, fd_count: 0,
///         disk_bytes: 0,
///     },
///     environment_ref: None,
///     executor_class: ExecutorClass::MacOsSandbox,
/// };
///
/// // Pure / total / deterministic: same input → byte-identical output.
/// let p1 = profile_from_warrant(&warrant);
/// let p2 = profile_from_warrant(&warrant);
/// assert_eq!(p1.as_bytes(), p2.as_bytes());
/// ```
#[must_use]
pub fn profile_from_warrant(spec: &WarrantSpec) -> SbplProfile {
    let mut buf: Vec<u8> = Vec::with_capacity(DENY_DEFAULT_TEMPLATE.len() + 256);
    buf.extend_from_slice(DENY_DEFAULT_TEMPLATE);

    // fs_scope — BTreeMap iteration is by sorted key, so the resulting bytes
    // are deterministic across runs / machines.
    for (path, mode) in &spec.fs_scope.mounts {
        let path_str = path.to_string_lossy();
        let path_escaped = sbpl_escape(&path_str);
        match mode {
            kx_warrant::FsMode::ReadOnly => {
                buf.extend_from_slice(b"(allow file-read* (subpath \"");
                buf.extend_from_slice(path_escaped.as_bytes());
                buf.extend_from_slice(b"\"))\n");
            }
            kx_warrant::FsMode::ReadWrite => {
                buf.extend_from_slice(b"(allow file-read* (subpath \"");
                buf.extend_from_slice(path_escaped.as_bytes());
                buf.extend_from_slice(b"\"))\n");
                buf.extend_from_slice(b"(allow file-write* (subpath \"");
                buf.extend_from_slice(path_escaped.as_bytes());
                buf.extend_from_slice(b"\"))\n");
            }
            kx_warrant::FsMode::ExecOnly => {
                buf.extend_from_slice(b"(allow file-read-metadata (subpath \"");
                buf.extend_from_slice(path_escaped.as_bytes());
                buf.extend_from_slice(b"\"))\n");
                buf.extend_from_slice(b"(allow process-exec (subpath \"");
                buf.extend_from_slice(path_escaped.as_bytes());
                buf.extend_from_slice(b"\"))\n");
            }
        }
    }

    // net_scope — BTreeSet iteration is by sorted value.
    match &spec.net_scope {
        kx_warrant::NetScope::None => {
            // Deny-default holds; no rules emitted.
        }
        kx_warrant::NetScope::EgressAllowlist(hosts) => {
            for host in hosts {
                let host_escaped = sbpl_escape(&host.0);
                buf.extend_from_slice(b"(allow network-outbound (remote ip \"");
                buf.extend_from_slice(host_escaped.as_bytes());
                buf.extend_from_slice(b":*\"))\n");
            }
        }
    }

    // syscall_profile_ref — emitted as an audit-trail comment. Resolution +
    // body-byte appending ships in the PR 9a-hardening follow-up sweep
    // (out of scope for this PR).
    buf.extend_from_slice(b";; syscall-profile-ref: ");
    for byte in &spec.syscall_profile_ref.0 {
        let nibble_hi = NIBBLES[(byte >> 4) as usize];
        let nibble_lo = NIBBLES[(byte & 0x0F) as usize];
        buf.push(nibble_hi);
        buf.push(nibble_lo);
    }
    buf.push(b'\n');

    SbplProfile(buf)
}

/// Lowercase hex nibbles for the syscall_profile_ref comment line.
const NIBBLES: &[u8; 16] = b"0123456789abcdef";

/// Escape a path or host string for SBPL S-expression embedding.
/// SBPL uses Lisp-style strings; the only mandatory escapes are `\"` and
/// `\\`. Per the deny-default safety property (D46 §5): under-declared
/// axes default-deny, so an aggressively-escaped string that produces a
/// malformed rule is still safer than under-escaping that produces an
/// over-permissive rule.
fn sbpl_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            // Control characters become spaces (defensive; sandbox-exec
            // rejects them anyway).
            c if c.is_control() => out.push(' '),
            c => out.push(c),
        }
    }
    out
}
