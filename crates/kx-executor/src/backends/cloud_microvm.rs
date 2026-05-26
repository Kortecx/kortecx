//! `CloudMicroVmExecutor` — cloud-side microVM backend. **Refusal on every
//! platform in OSS per D28.** Concrete impl lives behind the cloud feature
//! flag.

use kx_mote::Mote;
use kx_warrant::{ExecutorClass, WarrantSpec};

use crate::executor_trait::{MoteExecutionResult, MoteExecutor, MoteExecutorError, Rootfs};

/// Cloud-side microVM backend (cloud feature only).
#[derive(Debug, Default, Clone, Copy)]
pub struct CloudMicroVmExecutor {
    _private: (),
}

impl CloudMicroVmExecutor {
    /// Construct a new `CloudMicroVmExecutor`. No side effects.
    #[must_use]
    pub const fn new() -> Self {
        Self { _private: () }
    }
}

impl MoteExecutor for CloudMicroVmExecutor {
    fn run(
        &self,
        _mote: &Mote,
        _warrant: &WarrantSpec,
        _env: Option<Rootfs>,
    ) -> Result<MoteExecutionResult, MoteExecutorError> {
        Err(MoteExecutorError::BackendUnsupported {
            class: ExecutorClass::CloudMicroVm,
            reason: "cloud-only; not available in OSS v0.1".into(),
        })
    }

    fn supports(&self, _executor_class: ExecutorClass) -> bool {
        false
    }
}
