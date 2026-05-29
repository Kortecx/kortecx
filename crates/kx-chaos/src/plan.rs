//! The chaos plan: everything a seed deterministically decides, derived once.
//!
//! [`ChaosPlan::from_seed`] is total and pure — it consumes a single
//! [`SplitMix64`](crate::SplitMix64) stream and returns the scenario, fault, and
//! shape for that seed. Nothing downstream draws further randomness, so a seed maps
//! to exactly one driven sequence and one outcome.

use crate::prng::SplitMix64;

/// Which P3 exit-gate guarantee a run exercises.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScenarioKind {
    /// World-mutating exactly-once under worker death (and the P3.6c safe-stuck case).
    ExactlyOnce,
    /// A topology shaper death: exactly-one committed decision, deterministic children.
    TopologyShaper,
    /// Repudiation cascade correctness after a death during lineage assembly.
    RepudiationCascade,
}

/// Where/whether the holder of the fault-target Mote fails. The cluster interprets
/// each variant against the target's `nd_class` + effect pattern (e.g. a death before
/// commit re-dispatches a PURE/staged Mote but leaves an unstaged world effect safely
/// stuck — the P3.6c oracle refusal).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultPoint {
    /// No fault — a single worker drives the target to commit (control arm).
    Clean,
    /// Two live workers both lease and execute the target concurrently (no lease lock);
    /// the journal's dedup-by-key must collapse it to one committed fact.
    RacingDuplicate,
    /// The holder dies after acting but before committing; a live worker reaps and
    /// recovers it (or the oracle refuses re-dispatch and it stays safely stuck).
    DeathBeforeCommit,
}

/// The effect pattern of the world-mutating target in an [`ScenarioKind::ExactlyOnce`]
/// run. `StageThenCommit` records a durable `EffectStaged` hint (re-dispatch admissible
/// after death); the other two dispatch *without* staging, so the P3.6c oracle refuses
/// to re-dispatch a crash-failed one — the safe-stuck path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WmPattern {
    /// Stage the intent before firing — recoverable after death.
    StageThenCommit,
    /// Critic-validated; fires without staging.
    ValidateThenCommit,
    /// Idempotent by design; fires without staging.
    IdempotentByConstruction,
}

/// The fully-resolved decision for one seed. Constructed only via
/// [`ChaosPlan::from_seed`]; every field is a pure function of `seed`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChaosPlan {
    /// The seed this plan was derived from (the reproduction handle).
    pub seed: u64,
    /// Which guarantee this run proves.
    pub scenario: ScenarioKind,
    /// How the fault-target's holder fails.
    pub fault: FaultPoint,
    /// The world-mutating target's effect pattern (relevant to `ExactlyOnce`).
    pub wm_pattern: WmPattern,
    /// How many workers the run registers (2..=4) — enough for a reap + a racer.
    pub worker_count: u8,
    /// A per-seed identity salt, so distinct seeds build visibly-distinct Motes
    /// (aids debugging; each seed already has its own coordinator + journal).
    pub salt: u8,
}

impl ChaosPlan {
    /// Derive the plan for `seed`. Pure and total: equal seeds give equal plans.
    #[must_use]
    pub fn from_seed(seed: u64) -> Self {
        let mut r = SplitMix64::new(seed);
        let scenario = *r
            .choose(&[
                ScenarioKind::ExactlyOnce,
                ScenarioKind::TopologyShaper,
                ScenarioKind::RepudiationCascade,
            ])
            .unwrap_or(&ScenarioKind::ExactlyOnce);
        let fault = *r
            .choose(&[
                FaultPoint::Clean,
                FaultPoint::RacingDuplicate,
                FaultPoint::DeathBeforeCommit,
            ])
            .unwrap_or(&FaultPoint::Clean);
        let wm_pattern = *r
            .choose(&[
                WmPattern::StageThenCommit,
                WmPattern::ValidateThenCommit,
                WmPattern::IdempotentByConstruction,
            ])
            .unwrap_or(&WmPattern::StageThenCommit);
        let worker_count = 2 + u8::try_from(r.below(3)).unwrap_or(0);
        let salt = u8::try_from(r.below(251)).unwrap_or(0);
        Self {
            seed,
            scenario,
            fault,
            wm_pattern,
            worker_count,
            salt,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ChaosPlan;

    #[test]
    fn from_seed_is_deterministic() {
        for seed in 0..5_000u64 {
            assert_eq!(ChaosPlan::from_seed(seed), ChaosPlan::from_seed(seed));
        }
    }

    #[test]
    fn worker_count_in_range() {
        for seed in 0..5_000u64 {
            let p = ChaosPlan::from_seed(seed);
            assert!((2..=4).contains(&p.worker_count), "seed {seed}: {p:?}");
        }
    }

    #[test]
    fn all_scenarios_and_faults_appear() {
        use super::{FaultPoint, ScenarioKind};
        let mut scen = [false; 3];
        let mut fault = [false; 3];
        for seed in 0..512u64 {
            let p = ChaosPlan::from_seed(seed);
            scen[match p.scenario {
                ScenarioKind::ExactlyOnce => 0,
                ScenarioKind::TopologyShaper => 1,
                ScenarioKind::RepudiationCascade => 2,
            }] = true;
            fault[match p.fault {
                FaultPoint::Clean => 0,
                FaultPoint::RacingDuplicate => 1,
                FaultPoint::DeathBeforeCommit => 2,
            }] = true;
        }
        assert!(scen.iter().all(|&b| b), "every scenario is exercised");
        assert!(fault.iter().all(|&b| b), "every fault is exercised");
    }
}
