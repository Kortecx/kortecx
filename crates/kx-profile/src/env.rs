//! Environment capture — the load-bearing label on every profiling record.
//!
//! Golden Rule 10: "a number with no environment label is not a record (macOS
//! fsync ≠ Linux fsync — label or discard)." So [`Environment`] is a required,
//! fully-populated struct; capture is fallible and aborts the run rather than
//! emit a partial label.
//!
//! Capture is done at **run time** (via std + a few cheap commands), NOT a
//! `build.rs`, so `kx-profile`'s compiled artifact carries no volatile bytes
//! and the workspace `check-reproducible` (I1.c byte-determinism) gate is
//! unaffected.

use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::error::ProfileError;

/// The machine + toolchain a set of numbers was captured on. Every field is
/// required: a report cannot serialize without a complete label.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Environment {
    /// The host name (`hostname`, or the `HOSTNAME`/`COMPUTERNAME` env).
    pub host: String,
    /// The target OS (`std::env::consts::OS`).
    pub os: String,
    /// The target architecture (`std::env::consts::ARCH`).
    pub arch: String,
    /// A human CPU model string (sysctl/`/proc/cpuinfo`, falling back to arch).
    pub cpu: String,
    /// The number of logical cores (`available_parallelism`).
    pub cores: usize,
    /// The compiler version (`rustc --version`).
    pub toolchain: String,
    /// The cargo features `kx-profile` was built with (the profiled build).
    pub features: Vec<String>,
}

impl Environment {
    /// Capture the current environment.
    ///
    /// # Errors
    /// Returns [`ProfileError::Env`] if `rustc --version` cannot be run (the
    /// toolchain label is mandatory) or the host name cannot be determined.
    pub fn capture() -> Result<Self, ProfileError> {
        let host = host_name()
            .ok_or_else(|| ProfileError::Env("could not determine the host name".to_string()))?;
        let toolchain = run_cmd("rustc", &["--version"]).ok_or_else(|| {
            ProfileError::Env("could not run `rustc --version` for the toolchain label".to_string())
        })?;
        let cores = std::thread::available_parallelism()
            .map(std::num::NonZeroUsize::get)
            .unwrap_or(1);
        Ok(Self {
            host,
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            cpu: cpu_model(),
            cores,
            toolchain,
            features: enabled_features(),
        })
    }
}

/// Capture the commit the runtime is being profiled at.
///
/// # Errors
/// Returns [`ProfileError::Env`] if `git rev-parse HEAD` cannot be run (the
/// report's `git_sha` is mandatory — profiling runs inside the repo).
pub fn capture_git_sha() -> Result<String, ProfileError> {
    run_cmd("git", &["rev-parse", "HEAD"])
        .ok_or_else(|| ProfileError::Env("could not run `git rev-parse HEAD`".to_string()))
}

/// Run a command and return its trimmed stdout, or `None` on any failure.
fn run_cmd(prog: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(prog).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// Best-effort host name: the `hostname` command, then common env vars.
fn host_name() -> Option<String> {
    run_cmd("hostname", &[])
        .or_else(|| std::env::var("HOSTNAME").ok())
        .or_else(|| std::env::var("COMPUTERNAME").ok())
        .filter(|s| !s.is_empty())
}

/// A human CPU model string. Falls back to the architecture (always present),
/// so this never fails — arch is itself a valid CPU descriptor.
fn cpu_model() -> String {
    #[cfg(target_os = "macos")]
    if let Some(brand) = run_cmd("sysctl", &["-n", "machdep.cpu.brand_string"]) {
        return brand;
    }
    #[cfg(target_os = "linux")]
    if let Ok(info) = std::fs::read_to_string("/proc/cpuinfo") {
        if let Some(model) = info.lines().find_map(|l| {
            l.strip_prefix("model name")
                .and_then(|r| r.split(':').nth(1))
        }) {
            let model = model.trim();
            if !model.is_empty() {
                return model.to_string();
            }
        }
    }
    std::env::consts::ARCH.to_string()
}

/// The cargo features `kx-profile` was compiled with. `kx-profile` is FFI-free
/// with no optional features today, so this is `["default"]`; a future
/// `inference`/`hnsw` feature would extend it here so the label tracks the
/// profiled build.
fn enabled_features() -> Vec<String> {
    let features = vec!["default".to_string()];
    features
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_populates_every_field() {
        // In CI + locally, rustc + git are on PATH (we are inside the repo).
        let env = Environment::capture().expect("env capture in the workspace");
        assert!(!env.host.is_empty(), "host present");
        assert!(!env.os.is_empty(), "os present");
        assert!(!env.arch.is_empty(), "arch present");
        assert!(!env.cpu.is_empty(), "cpu present (≥ arch fallback)");
        assert!(env.cores >= 1, "at least one core");
        assert!(env.toolchain.contains("rustc"), "toolchain labelled");
        assert!(!env.features.is_empty(), "features present");
    }

    #[test]
    fn git_sha_is_captured() {
        let sha = capture_git_sha().expect("git sha in the workspace");
        assert_eq!(sha.len(), 40, "a full 40-hex HEAD sha");
    }
}
