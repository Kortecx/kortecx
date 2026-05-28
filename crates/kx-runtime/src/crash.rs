//! Deterministic crash injection for the kill-and-replay proof.
//!
//! A [`CrashPoint`] names a precise spot in the run where the process must
//! die — modelling a `kill -9` at exactly the window the exactly-once
//! guarantee has to survive. Injection is a real [`std::process::abort`]
//! (SIGABRT, no unwinding, no destructors, no flush) so the on-disk journal
//! is left in exactly the state a hard kill would leave it.

use std::str::FromStr;

/// Where to inject a hard process death during a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrashPoint {
    /// Abort during a `StageThenCommit` WORLD-MUTATING Mote's dispatch:
    /// after `EffectStaged` is journaled and the broker has staged the
    /// effect, but **before** the `Committed` entry lands. Recovery
    /// re-dispatches (the `EffectStaged` hint permits it) and the
    /// deterministic idempotency key dedups the external effect.
    PreCommitStc,
    /// Abort immediately **after** the `ValidateThenCommit` WORLD-MUTATING
    /// Mote's `Committed` entry lands, before the critic / remainder run.
    /// Recovery RE-READS the committed `result_ref` — the headline novel
    /// claim: a committed world-mutating step is a fact, never re-run.
    PostCommitVtc,
}

impl CrashPoint {
    /// The CLI / env spelling of this crash point.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            CrashPoint::PreCommitStc => "pre-commit-stc",
            CrashPoint::PostCommitVtc => "post-commit-vtc",
        }
    }

    /// Abort the process **now**, modelling a `kill -9`. Never returns.
    ///
    /// Uses `process::abort` (not `panic!` / `exit`) so no unwinding, no
    /// destructors, and no buffered-writer flush runs — the on-disk journal
    /// reflects only what was durably committed before this call, exactly
    /// as a hard kill would leave it.
    pub fn abort_now(self) -> ! {
        // Stderr so a supervising test can confirm the injected death.
        eprintln!("kx-runtime: injected crash at {} — aborting", self.as_str());
        std::process::abort();
    }
}

impl FromStr for CrashPoint {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pre-commit-stc" => Ok(CrashPoint::PreCommitStc),
            "post-commit-vtc" => Ok(CrashPoint::PostCommitVtc),
            other => Err(format!(
                "unknown crash point {other:?} (expected `pre-commit-stc` or `post-commit-vtc`)"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crash_point_roundtrips_through_str() {
        for cp in [CrashPoint::PreCommitStc, CrashPoint::PostCommitVtc] {
            assert_eq!(CrashPoint::from_str(cp.as_str()), Ok(cp));
        }
    }

    #[test]
    fn unknown_crash_point_is_rejected() {
        assert!(CrashPoint::from_str("mid-pure").is_err());
    }
}
