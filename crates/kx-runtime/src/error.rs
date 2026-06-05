//! The runtime's error type — honest variants only (SN-4 v2 #4, Golden Rule 1).

use kx_executor::LifecycleError;

/// A failure while driving the runtime.
#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    /// Opening or writing the on-disk SQLite journal failed.
    #[error("journal: {0}")]
    Journal(#[from] kx_journal::JournalError),

    /// Opening or accessing the on-disk content store failed.
    #[error("content store: {0}")]
    Store(#[from] kx_content::StoreError),

    /// Folding a journal entry into the projection failed.
    #[error("projection: {0}")]
    Projection(#[from] kx_projection::ProjectionError),

    /// Submitting a Mote to the scheduler failed.
    #[error("scheduler: {0}")]
    Scheduler(#[from] kx_scheduler::SchedulerError),

    /// An executor lifecycle step (run / commit / recovery) failed.
    #[error("lifecycle: {0}")]
    Lifecycle(#[from] LifecycleError),

    /// The drive loop made no progress yet the workflow is incomplete —
    /// a Mote is stuck (e.g. a WORLD-MUTATING Mote crashed pre-commit with
    /// no `EffectStaged` hint, which the recovery oracle correctly refuses
    /// to re-dispatch). Surfaced rather than looped forever.
    #[error("workflow stalled: {0} Mote(s) incomplete and none are actionable")]
    Stalled(usize),

    /// Canonical-bincode encoding of a `TopologyDecision` / `WarrantSpec`
    /// failed. Effectively unreachable for these fixed serde types, but
    /// surfaced rather than `expect`-ed (workspace deny on `expect_used`).
    #[error("canonical encode: {0}")]
    Encode(String),

    /// CLI / config error (bad argument, missing path).
    #[error("config: {0}")]
    Config(String),

    /// Opening the off-truth-path audit log failed (R4). Surfaced at construction
    /// (fail-fast on a misconfigured `--audit-log` path) — record-time audit write
    /// failures are best-effort and NEVER surface here (they are swallowed +
    /// counted via `kx_audit::AuditSink::dropped`).
    #[error("audit: {0}")]
    Audit(#[from] kx_audit::AuditError),

    /// A verified schema migration ([`crate::migrate_and_verify`]) rewrote the
    /// journal but the result did not preserve the run's product identity — the
    /// up-converted source and the migrated destination fold to different
    /// committed-facts digests. Indicates a migration bug; the destination must
    /// not be trusted.
    #[error(
        "migration verification failed (from schema v{from_version}): \
         source digest {src_digest} != destination digest {dst_digest}"
    )]
    MigrationVerificationFailed {
        /// The source journal's on-disk schema version.
        from_version: u16,
        /// Committed-facts digest of the up-converted source (hex).
        src_digest: String,
        /// Committed-facts digest of the migrated destination (hex).
        dst_digest: String,
    },
}
