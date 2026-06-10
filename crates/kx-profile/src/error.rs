//! The profiling harness error type.
//!
//! Profiling is a best-effort *measurement* tool, but the **environment label**
//! is load-bearing (Golden Rule 10: a number with no environment is not a
//! record). So environment capture is fallible and surfaces here rather than
//! being silently defaulted — an incomplete env aborts the run.

use thiserror::Error;

/// Errors raised while capturing the environment, running a spike, or writing a
/// report. A profiling run aborts on any of these rather than emit a partial or
/// unlabelled record.
#[derive(Debug, Error)]
pub enum ProfileError {
    /// A required environment field could not be determined (Golden Rule 10:
    /// fail rather than persist an unlabelled number).
    #[error("environment capture failed: {0}")]
    Env(String),

    /// The in-process gateway under measurement failed to start, serve, or
    /// shut down.
    #[error("gateway under measurement failed: {0}")]
    Gateway(String),

    /// A gRPC client call against the gateway under measurement failed.
    #[error("client call failed: {0}")]
    Client(String),

    /// The gateway did not reach the expected terminal state within the budget.
    #[error("timed out waiting for {what} after {elapsed_ms} ms")]
    Timeout {
        /// What the harness was waiting for (e.g. "health SERVING").
        what: String,
        /// How long it waited before giving up, in milliseconds.
        elapsed_ms: u64,
    },

    /// Writing the JSON report to disk failed.
    #[error("writing the report failed: {0}")]
    Report(String),
}
