//! The harness's result types: [`ChaosOutcome`] (what a run observed) and
//! [`ChaosFailure`] (a gate violation or infrastructure fault, carrying the exact
//! seed + plan + reason so a failing sweep prints its own reproduction).

use std::fmt;

use crate::plan::ChaosPlan;

/// The observable result of one seed's run — the numbers the gate asserts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChaosOutcome {
    /// Committed (non-repudiated) Motes in the coordinator's projection at convergence.
    pub committed_count: usize,
    /// Net world effects applied (distinct tool-boundary idempotency keys). The
    /// exactly-once witness.
    pub net_effects: usize,
    /// Total broker dispatch calls (a re-fire after a death counts here, not in
    /// `net_effects`).
    pub dispatch_calls: usize,
    /// Whether the run ended in the P3.6c *safe-stuck* terminal: a fired-but-unstaged
    /// world effect the recovery oracle refused to re-dispatch (uncommitted, not re-fired).
    pub safely_stuck: bool,
    /// For the repudiation scenario: how many downstream Motes the cascade marked.
    pub cascade_size: Option<usize>,
    /// For the topology scenario: how many shaper children were derived + committed.
    pub materialized_children: usize,
}

/// A gate failure: an exactly-once / no-orphan / cascade invariant was violated, or the
/// coordinator returned an unexpected error. Carries everything needed to reproduce:
/// `run_seed(seed)` replays the identical `plan`.
#[derive(Debug, Clone)]
pub struct ChaosFailure {
    /// The seed that reproduces this exactly.
    pub seed: u64,
    /// The plan derived from `seed`.
    pub plan: ChaosPlan,
    /// Why the run failed (invariant text or `infra: …`).
    pub reason: String,
    /// The outcome observed before the failure, when one was reached.
    pub outcome: Option<ChaosOutcome>,
}

impl fmt::Display for ChaosFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "P3 chaos gate FAILED at seed {}\n  reproduce: kx_chaos::run_seed({})\n  plan: {:?}\n  reason: {}\n  outcome: {:?}",
            self.seed, self.seed, self.plan, self.reason, self.outcome
        )
    }
}

impl std::error::Error for ChaosFailure {}
