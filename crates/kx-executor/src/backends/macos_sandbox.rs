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
/// **PR 9a placeholder.** Returns the deny-default template only — no
/// per-axis allows. The full per-axis mapping (`fs_scope` →
/// `(allow file-read*/file-write* (subpath …))`; `net_scope` →
/// `(allow network-outbound (remote ip "<host>:*"))`; etc.) ships in the
/// PR 9a-hardening follow-up. The function's signature, purity contract, and
/// return type are stable from PR 9a forward.
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
pub fn profile_from_warrant(_spec: &WarrantSpec) -> SbplProfile {
    // PR 9a returns the deny-default template only. PR 9a-hardening will
    // append per-axis allows derived from `_spec.fs_scope` / `_spec.net_scope`
    // / `_spec.syscall_profile_ref` per D46 §6.
    SbplProfile(DENY_DEFAULT_TEMPLATE.to_vec())
}
