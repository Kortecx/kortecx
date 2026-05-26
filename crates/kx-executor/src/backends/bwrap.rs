//! `BwrapExecutor` — bubblewrap-based sandbox executor for Linux. **PR 9a
//! skeleton**: structure + `supports()` are real; `run()` returns
//! `BackendUnsupported`. The real bubblewrap argv builder + `execvp` spawn
//! lands in the PR 9a-hardening follow-up.

use kx_mote::Mote;
use kx_warrant::{ExecutorClass, WarrantSpec};

use crate::executor_trait::{MoteExecutionResult, MoteExecutor, MoteExecutorError, Rootfs};

/// Bubblewrap-based sandbox executor (Linux default per D41).
///
/// Per D31, this is the daemonless ms-spawn least-privilege OSS default on
/// Linux. The constructor is conditional on `target_os = "linux"`; on other
/// targets `BwrapExecutor::new()` returns the placeholder shape but
/// `supports()` returns `false` and `run()` returns `BackendUnsupported`.
#[derive(Debug, Default, Clone, Copy)]
pub struct BwrapExecutor {
    // PR 9a-hardening will add fields here: a pinned `bwrap` binary path,
    // a tempdir handle for rootfs extraction, cgroup-v2 controller paths, etc.
    _private: (),
}

impl BwrapExecutor {
    /// Construct a new `BwrapExecutor`. PR 9a's constructor has no side
    /// effects; the PR 9a-hardening follow-up will probe for the `bwrap`
    /// binary at construction time and refuse if not present.
    #[must_use]
    pub const fn new() -> Self {
        Self { _private: () }
    }
}

impl MoteExecutor for BwrapExecutor {
    fn run(
        &self,
        _mote: &Mote,
        _warrant: &WarrantSpec,
        _env: Option<Rootfs>,
    ) -> Result<MoteExecutionResult, MoteExecutorError> {
        Err(MoteExecutorError::BackendUnsupported {
            class: ExecutorClass::Bwrap,
            reason: "skeleton — real bwrap spawn lands in the PR 9a-hardening follow-up".into(),
        })
    }

    fn supports(&self, executor_class: ExecutorClass) -> bool {
        // The Bwrap variant is the Linux default; on non-Linux targets the
        // backend exists for trait-object uniformity but reports unsupported.
        cfg!(target_os = "linux") && executor_class == ExecutorClass::Bwrap
    }
}
