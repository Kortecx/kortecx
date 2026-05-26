//! Platform-conditional `default_executor()` test (SN-7 — cross-platform).
//! Asserts the factory picks the right backend per `target_os`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use kx_executor::{default_executor, executor_for_class};
use kx_warrant::ExecutorClass;

#[test]
fn default_executor_supports_the_platform_default() {
    let executor = default_executor();
    #[cfg(target_os = "linux")]
    {
        assert!(executor.supports(ExecutorClass::Bwrap));
        assert!(!executor.supports(ExecutorClass::MacOsSandbox));
    }
    #[cfg(target_os = "macos")]
    {
        assert!(executor.supports(ExecutorClass::MacOsSandbox));
        assert!(!executor.supports(ExecutorClass::Bwrap));
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        // On unknown targets the factory returns the cloud-microvm refusal
        // backend; it supports no class.
        assert!(!executor.supports(ExecutorClass::Bwrap));
        assert!(!executor.supports(ExecutorClass::MacOsSandbox));
        assert!(!executor.supports(ExecutorClass::OciDaemon));
        assert!(!executor.supports(ExecutorClass::CloudMicroVm));
    }
}

#[test]
fn executor_for_class_returns_correct_backend_shape() {
    // Each class returns a backend; the concrete impl is class-specific.
    // The `supports` answer depends on the current `target_os`.
    let bwrap = executor_for_class(ExecutorClass::Bwrap);
    assert_eq!(
        bwrap.supports(ExecutorClass::Bwrap),
        cfg!(target_os = "linux")
    );

    let mac = executor_for_class(ExecutorClass::MacOsSandbox);
    assert_eq!(
        mac.supports(ExecutorClass::MacOsSandbox),
        cfg!(target_os = "macos")
    );

    let oci = executor_for_class(ExecutorClass::OciDaemon);
    // OciDaemonExecutor stub claims no support — refusal at run().
    assert!(!oci.supports(ExecutorClass::OciDaemon));

    let cloud = executor_for_class(ExecutorClass::CloudMicroVm);
    assert!(!cloud.supports(ExecutorClass::CloudMicroVm));
}
