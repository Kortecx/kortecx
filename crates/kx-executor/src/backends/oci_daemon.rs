//! `OciDaemonExecutor` — warrant-declared opt-in container-runtime backend.
//! **Stub on every platform per D31 PENDING EXTERNAL VERIFICATION.** Full
//! impl deferred until GPU-under-bwrap / live-service-environment research
//! decides between Podman/runc/docker on the per-Mote path.

use kx_mote::Mote;
use kx_warrant::{ExecutorClass, WarrantSpec};

use crate::executor_trait::{MoteExecutionResult, MoteExecutor, MoteExecutorError, Rootfs};

/// Warrant-declared opt-in container-runtime backend. PR 9a stub.
#[derive(Debug, Default, Clone, Copy)]
pub struct OciDaemonExecutor {
    _private: (),
}

impl OciDaemonExecutor {
    /// Construct a new `OciDaemonExecutor`. No side effects.
    #[must_use]
    pub const fn new() -> Self {
        Self { _private: () }
    }
}

impl MoteExecutor for OciDaemonExecutor {
    fn run(
        &self,
        _mote: &Mote,
        _warrant: &WarrantSpec,
        _env: Option<Rootfs>,
    ) -> Result<MoteExecutionResult, MoteExecutorError> {
        Err(MoteExecutorError::BackendUnsupported {
            class: ExecutorClass::OciDaemon,
            reason: "stub — full impl deferred per D31 PENDING EXTERNAL VERIFICATION".into(),
        })
    }

    fn supports(&self, _executor_class: ExecutorClass) -> bool {
        // The stub never claims support for any `executor_class`. The default
        // executor factory routes `OciDaemon`-class warrants to this backend,
        // which then returns `BackendUnsupported`.
        false
    }
}
