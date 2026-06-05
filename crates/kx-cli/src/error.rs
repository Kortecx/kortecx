//! [`CliError`] — the CLI's typed failure surface, with an explicit exit-code
//! mapping so a script or agent never mistakes a failure for success:
//!
//! | code | meaning                                                            |
//! |------|--------------------------------------------------------------------|
//! | `0`  | success (handled by the verb, not here)                            |
//! | `2`  | usage / configuration error (bad flags, bad hex, client-side bad JSON) |
//! | `3`  | `--wait` timed out — the run is still in progress and resumable     |
//! | `1`  | everything else (RPC error, connect failure, failed Mote, IO)      |

use std::process::ExitCode;

use crate::hex::HexError;

/// A CLI failure. Rendered to stderr as `kx: {error}`; the [`exit_code`] decides
/// the process status.
///
/// [`exit_code`]: CliError::exit_code
#[derive(Debug, thiserror::Error)]
pub enum CliError {
    /// Bad arguments: unknown flag, missing required value, bad hex length,
    /// mutually-exclusive flags, or client-side-invalid `--args` JSON. Exit `2`.
    #[error("{0}")]
    Usage(String),
    /// A forwarded engine/gateway configuration error (`run`/`serve`). Exit `2`.
    #[error("{0}")]
    Config(String),
    /// The gateway endpoint could not be dialed. Exit `1`.
    #[error("could not connect to {endpoint}: {detail}")]
    Connect {
        /// The endpoint URL that failed to dial.
        endpoint: String,
        /// The transport-level detail.
        detail: String,
    },
    /// The gateway returned a gRPC error status. Exit `1`.
    #[error("{code:?}: {message}")]
    Rpc {
        /// The gRPC status code (e.g. `Unauthenticated`, `PermissionDenied`).
        code: tonic::Code,
        /// The status message.
        message: String,
    },
    /// The forwarded [`kx_runtime`] engine returned an error. Exit `1`.
    #[error("{0}")]
    Runtime(String),
    /// A local IO error (reading a token / manifest file, writing `--out`). Exit `1`.
    #[error("io: {0}")]
    Io(String),
    /// `--wait` reached its timeout while the run was still in progress. The
    /// verb has already printed the resumable handle. Exit `3`.
    #[error(
        "run still in progress (timed out waiting); resume with `kx projection` / `kx events`"
    )]
    WaitTimeout,
    /// `--wait` observed the terminal Mote reach a `Failed` state. Exit `1`.
    #[error("the run's terminal Mote failed")]
    Failed,
}

impl CliError {
    /// Build an [`Rpc`](CliError::Rpc) error from a tonic status. Takes the
    /// status by value so it composes as `.map_err(CliError::from_status)`.
    #[must_use]
    #[allow(clippy::needless_pass_by_value)]
    pub fn from_status(status: tonic::Status) -> Self {
        CliError::Rpc {
            code: status.code(),
            message: status.message().to_string(),
        }
    }

    /// The process exit code for this error (see the module table).
    #[must_use]
    pub fn exit_code(&self) -> ExitCode {
        match self {
            CliError::Usage(_) | CliError::Config(_) => ExitCode::from(2),
            CliError::WaitTimeout => ExitCode::from(3),
            _ => ExitCode::FAILURE,
        }
    }

    /// `true` for the errors whose remedy is the usage text (so `run` prints it).
    #[must_use]
    pub fn is_usage(&self) -> bool {
        matches!(self, CliError::Usage(_) | CliError::Config(_))
    }
}

impl From<HexError> for CliError {
    fn from(e: HexError) -> Self {
        CliError::Usage(e.to_string())
    }
}
