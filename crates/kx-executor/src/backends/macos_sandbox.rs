//! `MacOsSandboxExecutor` — macOS sandbox-exec / Seatbelt sibling of
//! `BwrapExecutor`. **PR 9a skeleton**: structure + `supports()` are real;
//! `run()` returns `BackendUnsupported`. The real `posix_spawn` +
//! `sandbox_init`-equivalent + SBPL profile generation lands in the PR 9a-
//! hardening follow-up.
//!
//! Per D46 (`docs/design/macos-sandbox-profile.md` P0.14), the runtime
//! generates an `SbplProfile` from a `WarrantSpec` via the pure
//! `profile_from_warrant` function. PR 9a ships a placeholder
//! `profile_from_warrant` that returns the deny-default template only (no
//! per-axis allows); the per-axis mapping ships in the PR 9a-hardening
//! follow-up.

use kx_mote::Mote;
use kx_warrant::{ExecutorClass, WarrantSpec};

use crate::executor_trait::{MoteExecutionResult, MoteExecutor, MoteExecutorError, Rootfs};

/// macOS sandbox-exec / Seatbelt-based sandbox executor (macOS default per
/// D41). PR 9a skeleton.
#[derive(Debug, Default, Clone, Copy)]
pub struct MacOsSandboxExecutor {
    _private: (),
}

impl MacOsSandboxExecutor {
    /// Construct a new `MacOsSandboxExecutor`. No side effects in PR 9a.
    #[must_use]
    pub const fn new() -> Self {
        Self { _private: () }
    }
}

impl MoteExecutor for MacOsSandboxExecutor {
    fn run(
        &self,
        _mote: &Mote,
        _warrant: &WarrantSpec,
        _env: Option<Rootfs>,
    ) -> Result<MoteExecutionResult, MoteExecutorError> {
        Err(MoteExecutorError::BackendUnsupported {
            class: ExecutorClass::MacOsSandbox,
            reason:
                "skeleton — real posix_spawn + sandbox_init lands in the PR 9a-hardening follow-up"
                    .into(),
        })
    }

    fn supports(&self, executor_class: ExecutorClass) -> bool {
        cfg!(target_os = "macos") && executor_class == ExecutorClass::MacOsSandbox
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

/// The compile-time-embedded deny-default template (D46 §5). PR 9a-hardening
/// will replace this with an `include_str!` of
/// `crates/kx-executor/src/backends/macos_sandbox_template.sb`. The placeholder
/// here is a minimal deny-default SBPL stub that demonstrates the shape.
const DENY_DEFAULT_TEMPLATE: &[u8] = b"(version 1)\n(deny default)\n";

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
