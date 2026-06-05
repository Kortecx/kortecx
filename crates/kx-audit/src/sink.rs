//! [`AuditSink`] — the pluggable, best-effort audit seam.

use crate::event::AuditEvent;

/// A pluggable destination for [`AuditEvent`]s.
///
/// Implementations are `Send + Sync` and take `&self` (interior mutability) so a
/// single sink can be shared as `Arc<dyn AuditSink>` across the orchestrator and
/// any reader.
///
/// **Best-effort / non-fatal by construction.** [`Self::record`] returns unit:
/// an audit-write failure can NEVER propagate into the run loop (there is no
/// `Result` a maintainer could accidentally `?`-propagate). Implementations
/// swallow write errors, log them via `tracing`, and count them in
/// [`Self::dropped`] so the loss is observable rather than silent. An audit
/// subsystem that can abort the run it audits is a self-inflicted availability
/// bug; this trait shape makes that impossible.
pub trait AuditSink: Send + Sync {
    /// Record one lifecycle event. Infallible from the caller's perspective —
    /// failures are absorbed and counted ([`Self::dropped`]).
    fn record(&self, event: AuditEvent);

    /// Flush any buffered records to the backing store. Default: no-op (e.g. an
    /// in-memory sink has nothing to flush).
    fn flush(&self) {}

    /// Number of events dropped due to backend write failures — best-effort
    /// telemetry an operator (or a test) can assert on. Default: `0`.
    fn dropped(&self) -> u64 {
        0
    }
}
