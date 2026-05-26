//! `default_executor()` — platform-conditional factory per D41. Returns
//! `BwrapExecutor` on Linux, `MacOsSandboxExecutor` on macOS, a refusal
//! factory on other targets.
//!
//! **PR 9a returns the SKELETON backend for the current platform.** The
//! backend's `run` method returns `BackendUnsupported` until the
//! PR 9a-hardening follow-up wires real spawn semantics.

use crate::backends::bwrap::BwrapExecutor;
use crate::backends::cloud_microvm::CloudMicroVmExecutor;
use crate::backends::macos_sandbox::MacOsSandboxExecutor;
use crate::backends::oci_daemon::OciDaemonExecutor;
use crate::executor_trait::MoteExecutor;
use kx_warrant::ExecutorClass;

/// Pick the platform-appropriate default backend.
///
/// On Linux returns `Box<dyn MoteExecutor>` wrapping `BwrapExecutor`; on
/// macOS wrapping `MacOsSandboxExecutor`; on any other target wrapping a
/// `CloudMicroVmExecutor` (which always refuses with `BackendUnsupported` —
/// the safest fail-closed shape for unknown targets).
#[must_use]
pub fn default_executor() -> Box<dyn MoteExecutor> {
    #[cfg(target_os = "linux")]
    {
        Box::new(BwrapExecutor::new())
    }
    #[cfg(target_os = "macos")]
    {
        Box::new(MacOsSandboxExecutor::new())
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        Box::new(CloudMicroVmExecutor::new())
    }
}

/// Pick a backend for an explicit `ExecutorClass` (rather than the platform
/// default). Returns the matching backend; for `MacOsSandbox` on Linux (or
/// `Bwrap` on macOS) returns the requested backend even though it won't
/// successfully `run` on the host — refusal at `run()` is the right shape
/// (the type system permits constructing any backend; the per-call refusal
/// surfaces the platform mismatch).
#[must_use]
pub fn executor_for_class(class: ExecutorClass) -> Box<dyn MoteExecutor> {
    match class {
        ExecutorClass::Bwrap => Box::new(BwrapExecutor::new()),
        ExecutorClass::MacOsSandbox => Box::new(MacOsSandboxExecutor::new()),
        ExecutorClass::OciDaemon => Box::new(OciDaemonExecutor::new()),
        ExecutorClass::CloudMicroVm => Box::new(CloudMicroVmExecutor::new()),
    }
}
