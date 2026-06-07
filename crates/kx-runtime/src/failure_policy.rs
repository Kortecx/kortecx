//! [`FailurePolicy`] — the PR-1 bounded-retry + dead-letter seam (`Option`-gated).
//!
//! When the orchestrator ([`crate::run_with_seams`]) is handed `Some(&FailurePolicy)`,
//! a Mote dispatch error no longer aborts the whole run: a *transient infrastructure*
//! failure is retried (bounded), and a *terminal* failure (or an exhausted transient)
//! is journaled as a `Failed` fact so the drive loop can continue **past** the Mote
//! (dead-letter). With `None` — the canonical demo and every existing caller — a
//! dispatch error propagates exactly as before (`return Err`), so the deterministic
//! truth path (digest `a6b5c679…`) is byte-unchanged. This is the proven
//! `snapshot_sink`/`audit_sink`/`capture_sink` additive-`Option`-seam pattern.
//!
//! The policy is the *failsafe foundation* the model-driven re-plan loop (AL2)
//! builds on: a terminal `Failed` fact is the durable, auditable record a later
//! re-plan round reads — kortecx never blindly re-runs the same failing Mote
//! (the user's "no point re-running it"); it records the failure and, in a later
//! PR, lets the model propose a corrected fresh round.

use std::time::Duration;

use kx_executor::{CommitProtocolError, LifecycleError, MoteExecutorError, ResourceError};
use kx_journal::FailureReason;

/// How a Mote dispatch failure should be handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FailureClass {
    /// A transient *infrastructure* error (resource contention, a storage/journal
    /// hiccup, a broker network blip, a process spawn/timeout) — one that plausibly
    /// succeeds on a retry. Eligible for bounded retry-with-backoff.
    TransientInfra,
    /// A *terminal* logic / permission / validation failure — re-running the
    /// identical Mote cannot succeed. Dead-letter it (write a `Failed` fact) and
    /// let the loop continue; a model re-plan (AL2) is the correct recovery, not a
    /// blind retry.
    TerminalLogic,
}

/// Per-run bounded-retry + dead-letter policy. Passed as `Option<&FailurePolicy>`
/// to [`crate::run_with_seams`]; `None` ⇒ legacy abort-on-failure (digest-invariant).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FailurePolicy {
    /// Maximum dispatch attempts for a *transient* error before dead-lettering.
    /// `1` ⇒ no retry (a single attempt, then dead-letter on a transient failure).
    pub max_attempts: u32,
    /// Fixed backoff slept between transient retries.
    pub backoff: Duration,
}

impl FailurePolicy {
    /// A policy with `max_attempts` transient attempts and a `backoff` pause.
    /// `max_attempts` is floored at `1` (every Mote gets at least one attempt).
    #[must_use]
    pub fn new(max_attempts: u32, backoff: Duration) -> Self {
        Self {
            max_attempts: max_attempts.max(1),
            backoff,
        }
    }

    /// A sensible local-serve default: 3 transient attempts, 100 ms backoff.
    #[must_use]
    pub fn dev_default() -> Self {
        Self::new(3, Duration::from_millis(100))
    }
}

/// Classify a dispatch [`LifecycleError`] as transient-infra (retry) or
/// terminal-logic (dead-letter).
///
/// **Conservative by construction:** only clearly transient *infrastructure*
/// errors are retried; everything else — including any future/unknown nested
/// variant — defaults to terminal, so a deterministic failure is never retried in
/// a storm (fail-safe). The partition mirrors the recovery oracle's
/// pre-commit-crash (transient) vs terminal split: storage / broker / spawn /
/// timeout / resource-contention are transient; refusals, validation verdicts, the
/// R-13 double-fire guard, a non-zero body exit, and unsupported backends are
/// terminal.
pub(crate) fn classify_lifecycle_error(err: &LifecycleError) -> FailureClass {
    use FailureClass::{TerminalLogic, TransientInfra};
    match err {
        // A journal append hiccup is transient storage.
        LifecycleError::JournalAppend(_) => TransientInfra,
        LifecycleError::ResourceAcquire(e) => classify_resource_error(e),
        LifecycleError::ExecutorRun(e) => classify_executor_error(e),
        LifecycleError::CommitProtocol(e) => classify_commit_error(e),
        // A submission refusal (a permission/construction verdict — R-1..R-9 / widen
        // / validator type error) or an opaque internal lifecycle error: both are
        // deterministic, never retried.
        LifecycleError::Refused(_) | LifecycleError::Internal(_) => TerminalLogic,
    }
}

fn classify_resource_error(err: &ResourceError) -> FailureClass {
    use FailureClass::{TerminalLogic, TransientInfra};
    match err {
        // The manager is momentarily saturated — a backoff may free a slot.
        ResourceError::NoCapacity => TransientInfra,
        // The Mote's own ceilings are exceeded, or an internal/unknown-slot bug —
        // deterministic across retries.
        ResourceError::CpuCapExceeded { .. }
        | ResourceError::MemCapExceeded { .. }
        | ResourceError::FdCapExceeded { .. }
        | ResourceError::UnknownSlot(_)
        | ResourceError::Internal(_) => TerminalLogic,
    }
}

fn classify_executor_error(err: &MoteExecutorError) -> FailureClass {
    use FailureClass::{TerminalLogic, TransientInfra};
    match err {
        // Transient OS / sandbox / timeout conditions that may vary across attempts.
        MoteExecutorError::ProcessSpawnFailed { .. }
        | MoteExecutorError::SandboxLoadFailed { .. }
        | MoteExecutorError::RlimitFailed { .. }
        | MoteExecutorError::WallClockTimedOut { .. } => TransientInfra,
        // Deterministic: the backend can't run this class, the rootfs/profile is
        // bad, or the body itself exited non-zero (an agent *logic* failure → a
        // model re-plan, never a blind retry).
        MoteExecutorError::BackendUnsupported { .. }
        | MoteExecutorError::RootfsExtractFailed { .. }
        | MoteExecutorError::ProfileSyntaxError { .. }
        | MoteExecutorError::BodyExited { .. }
        | MoteExecutorError::Internal { .. } => TerminalLogic,
    }
}

fn classify_commit_error(err: &CommitProtocolError) -> FailureClass {
    use FailureClass::{TerminalLogic, TransientInfra};
    match err {
        // Storage / broker / probe transients — the broker's idempotency-key dedup
        // makes a WM re-dispatch exactly-once, so a bounded retry is safe.
        CommitProtocolError::BrokerDispatchFailed { .. }
        | CommitProtocolError::ContentStorePutFailed { .. }
        | CommitProtocolError::JournalAppendCommittedFailed { .. }
        | CommitProtocolError::JournalAppendEffectStagedFailed { .. }
        | CommitProtocolError::ProbeFailed { .. } => TransientInfra,
        // Deterministic protocol / validation verdicts + the R-13 re-dispatch
        // refusal (the double-fire guard — MUST NOT be retried).
        CommitProtocolError::R11ResultRefIncomplete { .. }
        | CommitProtocolError::R12CommittedNotProofOfValidity { .. }
        | CommitProtocolError::R13WmReDispatchRefused { .. }
        | CommitProtocolError::CompensateFailed { .. }
        | CommitProtocolError::Internal { .. } => TerminalLogic,
    }
}

/// The journaled [`FailureReason`] for a dead-lettered Mote of the given class.
///
/// **Both classes map to the dedicated terminal [`FailureReason::DeadLettered`]**
/// (F4). This is load-bearing for correctness, not cosmetic: the projection's
/// `Failed` fold sets `terminal_failure_observed` only when the reason is NOT a
/// pre-commit-crash ([`kx_journal::is_pre_commit_crash`]). The pre-F4 mapping wrote
/// [`FailureReason::TimedOut`] for an exhausted transient — but `TimedOut` IS a
/// pre-commit-crash (a worker that died mid-flight, *re-dispatchable* under an
/// `EffectStaged`). So a budget-exhausted WORLD-MUTATING `StageThenCommit`
/// dead-letter stayed re-dispatchable forever and `run_with_seams` spun the
/// EffectStaged-redispatch path without ever terminating (the F4 hang). Writing the
/// terminal `DeadLettered` makes the Mote terminal `Failed`, so the drive loop skips
/// it and the run completes. Mapping the *terminal-logic* class to the same variant
/// (vs the old [`FailureReason::ExecutorRefused`] reuse — a submission-time verdict,
/// not an engine dead-letter) gives the AL2 re-plan one unambiguous "the engine gave
/// up on this step" signal. No schema bump beyond the additive `DeadLettered`
/// variant; `Failed` entries are not folded into the canonical digest.
pub(crate) fn reason_for(class: FailureClass) -> FailureReason {
    match class {
        FailureClass::TransientInfra | FailureClass::TerminalLogic => FailureReason::DeadLettered,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kx_executor::MoteExecutorError;

    #[test]
    fn body_exit_is_terminal_not_retried() {
        // An agent body that exits non-zero is a deterministic logic failure — the
        // user's "no point re-running it". It must dead-letter, never retry-storm.
        let e = LifecycleError::ExecutorRun(MoteExecutorError::BodyExited { code: 1 });
        assert_eq!(classify_lifecycle_error(&e), FailureClass::TerminalLogic);
        // F4: every engine dead-letter is the dedicated terminal `DeadLettered`
        // reason (NOT pre-commit-crash → sets terminal_failure_observed → the loop
        // never re-dispatches it).
        assert_eq!(
            reason_for(FailureClass::TerminalLogic),
            FailureReason::DeadLettered
        );
    }

    #[test]
    fn resource_no_capacity_is_transient() {
        let e = LifecycleError::ResourceAcquire(ResourceError::NoCapacity);
        assert_eq!(classify_lifecycle_error(&e), FailureClass::TransientInfra);
        // F4: an exhausted-transient dead-letter is terminal `DeadLettered`, NOT the
        // pre-commit-crash `TimedOut` (which under EffectStaged stays redispatchable).
        assert_eq!(
            reason_for(FailureClass::TransientInfra),
            FailureReason::DeadLettered
        );
    }

    #[test]
    fn resource_cap_exceeded_is_terminal() {
        // The Mote asks for more than the ceiling — deterministic, dead-letter.
        let e = LifecycleError::ResourceAcquire(ResourceError::MemCapExceeded {
            requested: 1 << 40,
            cap: 1 << 20,
        });
        assert_eq!(classify_lifecycle_error(&e), FailureClass::TerminalLogic);
    }

    #[test]
    fn journal_append_hiccup_is_transient() {
        let e = LifecycleError::JournalAppend("disk busy".into());
        assert_eq!(classify_lifecycle_error(&e), FailureClass::TransientInfra);
    }

    #[test]
    fn internal_lifecycle_error_is_terminal() {
        let e = LifecycleError::Internal("opaque".into());
        assert_eq!(classify_lifecycle_error(&e), FailureClass::TerminalLogic);
    }

    #[test]
    fn max_attempts_floored_at_one() {
        assert_eq!(FailurePolicy::new(0, Duration::ZERO).max_attempts, 1);
        assert_eq!(FailurePolicy::dev_default().max_attempts, 3);
    }
}
