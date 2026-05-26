//! `MoteExecutor` trait — the per-Mote sandboxing seam (D31 + D41).
//!
//! `BwrapExecutor` (Linux) and `MacOsSandboxExecutor` (macOS) implement the
//! trait with real spawn semantics; `OciDaemonExecutor` is a warrant-declared
//! opt-in stub; `CloudMicroVmExecutor` refuses with `BackendUnsupported`
//! (cloud-side impl lives behind the cloud feature flag).
//!
//! **PR 9a ships SKELETON backends.** All four `run` methods return
//! `MoteExecutorError::BackendUnsupported { reason: "skeleton — real spawn in
//! 9a-hardening" }` to keep PR 9a's reviewer load manageable. The trait surface
//! is real; the integration test exercises the lifecycle via a `TestMoteExecutor`
//! (see `crate::lifecycle`); the real bwrap argv builder + SBPL profile
//! generator + posix_spawn + cgroup v2 file I/O land in the PR 9a-hardening
//! follow-up. PR 9b adds the commit protocol on top.

use kx_content::ContentRef;
use kx_mote::Mote;
use kx_warrant::{ExecutorClass, WarrantSpec};
use thiserror::Error;

/// The per-Mote sandboxing seam. Workspace `default_executor()` factory picks
/// the platform-appropriate impl: `BwrapExecutor` on Linux, `MacOsSandboxExecutor`
/// on macOS. The trait is object-safe + `Send + Sync` so callers may hold
/// `Arc<dyn MoteExecutor>`.
pub trait MoteExecutor: Send + Sync {
    /// Run the Mote body inside the sandbox enforced by `warrant`.
    ///
    /// `env` is the optional rootfs reference (`warrant.environment_ref` resolved
    /// to bytes); `None` means a minimal-base sandbox.
    ///
    /// **PR 9a returns `BackendUnsupported` from every concrete impl.** The real
    /// spawn semantics land in the PR 9a-hardening follow-up. Production
    /// consumers MUST NOT depend on this method succeeding at PR 9a.
    fn run(
        &self,
        mote: &Mote,
        warrant: &WarrantSpec,
        env: Option<Rootfs>,
    ) -> Result<MoteExecutionResult, MoteExecutorError>;

    /// Whether this backend can run Motes whose `warrant.executor_class` matches
    /// the argument. Used by `default_executor()` and refusal predicates.
    fn supports(&self, executor_class: ExecutorClass) -> bool;
}

/// Opaque rootfs reference — `warrant.environment_ref` resolved to bytes by
/// the executor's pre-spawn step. PR 9a treats this as a black-box; the
/// PR 9a-hardening follow-up wires bwrap rootfs extraction + macOS posix_spawn
/// chroot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rootfs {
    /// The content-addressed reference to the rootfs in the content store.
    pub content_ref: ContentRef,
    /// The path-on-disk where the rootfs has been (or will be) extracted.
    /// `None` means "rootfs is not yet materialized; extract before spawn."
    pub materialized_at: Option<std::path::PathBuf>,
}

/// Successful `MoteExecutor::run` return value. The body produced its
/// `result_ref`; the lifecycle layer (`crate::lifecycle`) handles the
/// downstream commit txn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoteExecutionResult {
    /// The body's output `ContentRef`. For PURE Motes (PR 9a scope) this is
    /// the deterministic hash of the body's compute; for WORLD-MUTATING Motes
    /// (PR 9b scope) it is the `BrokerHandle::staged_ref` returned by the
    /// capability broker.
    pub result_ref: ContentRef,
    /// Wall-clock at execution start (epoch milliseconds; opaque to the
    /// commit protocol — auditing only). NOT part of identity.
    pub started_at_epoch_ms: u64,
    /// Wall-clock at execution end. NOT part of identity.
    pub finished_at_epoch_ms: u64,
}

/// Typed error variants from `MoteExecutor::run`. The variant set is the
/// vocabulary the lifecycle + refusal layers reason against.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum MoteExecutorError {
    /// The backend does not support this `executor_class`. PR 9a returns this
    /// from every concrete impl since real spawn semantics ship in the
    /// PR 9a-hardening follow-up.
    #[error("backend {class:?} unsupported: {reason}")]
    BackendUnsupported {
        /// Which executor class was requested.
        class: ExecutorClass,
        /// Operator-facing diagnostic.
        reason: String,
    },

    /// Rootfs extraction (from `warrant.environment_ref`) failed.
    #[error("rootfs extract failed: {reason}")]
    RootfsExtractFailed {
        /// Operator-facing diagnostic.
        reason: String,
    },

    /// `sandbox_init`-equivalent rejected the profile (macOS).
    #[error("sandbox load failed: {reason}")]
    SandboxLoadFailed {
        /// Operator-facing diagnostic.
        reason: String,
    },

    /// Generated SBPL profile failed pre-flight syntax check (regression
    /// guard for D46 template edits).
    #[error("profile syntax error at offset {offset}")]
    ProfileSyntaxError {
        /// Byte offset within the generated profile.
        offset: usize,
    },

    /// `setrlimit` for an axis returned non-zero (macOS resource path).
    #[error("setrlimit {axis} failed: errno {errno}")]
    RlimitFailed {
        /// Which `RLIMIT_*` axis (e.g., `RLIMIT_CPU`, `RLIMIT_AS`).
        axis: String,
        /// `errno` value.
        errno: i32,
    },

    /// `posix_spawn` (macOS) / `execvp` (Linux) returned non-zero.
    #[error("process spawn failed: errno {errno}")]
    ProcessSpawnFailed {
        /// `errno` value.
        errno: i32,
    },

    /// The body process exceeded `warrant.resource_ceiling.wall_clock_ms`
    /// and was killed.
    #[error("wall-clock timeout exceeded {budget_ms} ms")]
    WallClockTimedOut {
        /// The budget that was exceeded.
        budget_ms: u64,
    },

    /// The body process exited with a non-zero code. The lifecycle layer
    /// surfaces this as a `Failed` journal entry; not a runtime panic.
    #[error("body exited with non-zero status {code}")]
    BodyExited {
        /// The exit code.
        code: i32,
    },

    /// Anything else — fail-closed catch-all. Operator-facing diagnostic
    /// surfaces the root cause.
    #[error("executor internal error: {reason}")]
    Internal {
        /// Operator-facing diagnostic.
        reason: String,
    },
}
