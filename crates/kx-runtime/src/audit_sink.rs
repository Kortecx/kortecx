//! [`RuntimeAuditSink`] — the R4 off-truth-path audit seam.
//!
//! A thin kx-runtime handle over a pluggable [`kx_audit::AuditSink`], mirroring
//! [`crate::CaptureSink`] / [`crate::SnapshotSink`]: the orchestrator
//! ([`crate::run_with_seams`]) takes `Option<&RuntimeAuditSink>` and emits an
//! [`kx_audit::AuditEvent`] at each lifecycle transition. `None` disables auditing
//! (the byte-identity-without-overhead path); `Some` records the run's lifecycle
//! to the wrapped sink.
//!
//! Like the other two seams it is **OFF THE TRUTH PATH**: audit is never journaled,
//! is never a `MoteId` input, and never gates scheduling / promotion / eviction
//! (the dependency wall in `kx-audit/tests/boundary.rs` is the compiler-enforced
//! tripwire). Turning it on changes only what is *observed*, never the committed
//! facts — so the canonical product digest `a6b5c679…` is byte-unchanged.
//!
//! It owns the run-scoped context (today: nothing beyond the wrapped sink; a
//! future gateway wiring stamps the authenticated `principal` here without
//! re-touching the run loop or the event enum).

use std::path::Path;
use std::sync::Arc;

use kx_audit::{AuditEvent, AuditSink, JsonlAuditSink};

use crate::error::RuntimeError;

/// A cheap, cloneable handle over a pluggable [`AuditSink`]. Clones share one
/// underlying sink (an `Arc`), so the orchestrator and any reader observe the same
/// trail.
#[derive(Clone)]
pub struct RuntimeAuditSink {
    inner: Arc<dyn AuditSink>,
}

impl RuntimeAuditSink {
    /// Wrap an arbitrary [`AuditSink`] (the general constructor — tests pass an
    /// `Arc<InMemoryAuditSink>` and keep a clone to assert on its `events()`).
    #[must_use]
    pub fn from_arc(inner: Arc<dyn AuditSink>) -> Self {
        Self { inner }
    }

    /// A run-scoped JSONL audit log at `path`, truncated fresh for this run (the
    /// `kx run --audit-log` default). Open failure is surfaced here (fail-fast on
    /// a bad path), never mid-run.
    pub fn jsonl(path: impl AsRef<Path>) -> Result<Self, RuntimeError> {
        let sink = JsonlAuditSink::create(path)?;
        Ok(Self::from_arc(Arc::new(sink)))
    }

    /// Record one lifecycle event. Best-effort / non-fatal (delegates to the
    /// wrapped sink, whose `record` cannot fail the run).
    pub fn record(&self, event: AuditEvent) {
        self.inner.record(event);
    }

    /// Flush any buffered records (the orchestrator calls this at run-complete).
    pub fn flush(&self) {
        self.inner.flush();
    }

    /// Number of events dropped due to backend write failures (best-effort telemetry).
    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.inner.dropped()
    }
}

impl std::fmt::Debug for RuntimeAuditSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuntimeAuditSink")
            .field("dropped", &self.inner.dropped())
            .finish_non_exhaustive()
    }
}
