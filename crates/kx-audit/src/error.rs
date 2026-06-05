//! [`AuditError`] — construction-time errors only.

/// An error constructing an audit sink (opening/creating the log file).
///
/// Record-time write errors are deliberately NOT modelled here: they are
/// swallowed (best-effort), logged via `tracing`, and counted via
/// [`crate::AuditSink::dropped`] — never surfaced to the run loop. The only
/// failure a caller handles is at construction, where it can fail fast on a
/// misconfigured path rather than mid-run.
#[derive(Debug, thiserror::Error)]
pub enum AuditError {
    /// The audit log file could not be opened or created.
    #[error("audit log open failed: {0}")]
    Open(#[from] std::io::Error),
}
