//! The eval report + the baseline gate.
//!
//! [`aggregate`] folds the per-task scores into suite-level integer Gate values plus an
//! env-labelled trend record. [`compare_to_baseline`] is the regression ratchet: it
//! fails closed on corpus drift (a changed `suite_digest` must be re-baselined
//! deliberately) and reports any Gate that fell below the committed baseline (minus a
//! per-mille tolerance). The decision is pure integer arithmetic — no float on the gate
//! path (SN-8). The report mirrors `kx-profile`'s Gate/Spike model (GR10) but is a
//! standalone type so the harness stays a dependency-light leaf.

use serde::{Deserialize, Serialize};

use crate::error::EvalError;
use crate::scorers::{ScoreOutput, ScoreValue, TRANSCRIPT_SCORER_IDS};

/// The report schema version (bump on any breaking JSON-shape change).
pub const SCHEMA_VERSION: u32 = 1;

/// The unit string recorded for a Gate metric in the trend report.
pub const GATE_UNIT: &str = "per_mille";

/// A measurement-only Spike (e.g. a Tier-B latency) — recorded for the trend, never
/// gated. Mirrors `kx_profile::Metric` of kind Spike.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpikeMetric {
    /// The metric id.
    pub id: String,
    /// The measured value.
    pub value: f64,
    /// The unit (e.g. `"ms"`).
    pub unit: String,
}

/// One task's scores (every per-transcript scorer's output for that task).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskScore {
    /// The task id.
    pub task_id: String,
    /// Every scorer's output for this task.
    pub scores: Vec<ScoreOutput>,
}

/// One aggregate Gate metric — a stable id and an integer per-mille value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateValue {
    /// The metric id (e.g. `"task_success"`).
    pub id: String,
    /// The aggregate score, `0..=1000`.
    pub per_mille: u32,
}

/// The committed yardstick — the suite's Gate values at a known corpus digest. Lives at
/// `corpus/golden-v1/baseline.json` (committed, NOT in the gitignored `docs/benchmarks/`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Baseline {
    /// The suite the baseline was captured on.
    pub suite_id: String,
    /// The corpus content digest (hex) the baseline is valid for.
    pub suite_digest: String,
    /// The Gate values, in a stable id order.
    pub gates: Vec<GateValue>,
}

/// One metric that regressed below the baseline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Regression {
    /// The metric id.
    pub metric_id: String,
    /// The baseline per-mille.
    pub baseline_per_mille: u32,
    /// The current per-mille (lower).
    pub current_per_mille: u32,
}

/// The outcome of comparing a run to its baseline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaselineComparison {
    /// Every metric that fell below baseline (minus tolerance).
    pub regressions: Vec<Regression>,
    /// `true` iff there were no regressions.
    pub ok: bool,
}

/// A full eval report: the corpus identity + env label, the aggregate Gate values (the
/// gating surface), the Tier-B Spikes (advisory), and the per-task breakdown.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvalReport {
    /// The report schema version ([`SCHEMA_VERSION`]).
    pub schema: u32,
    /// The suite id.
    pub suite_id: String,
    /// The corpus content digest (hex).
    pub suite_digest: String,
    /// The commit the eval ran at (`git rev-parse HEAD`, or `"unknown"`).
    pub git_sha: String,
    /// A short environment label (e.g. `"macos/aarch64 (8 cores)"`) — the GR10 "a number
    /// with no environment label is not a record" discipline, kept lightweight.
    pub env_label: String,
    /// The aggregate Gate values (the regression-gated surface).
    pub gates: Vec<GateValue>,
    /// Measurement-only Spikes (Tier-B latency etc.; advisory, never gated).
    pub spikes: Vec<SpikeMetric>,
    /// The per-task score breakdown.
    pub per_task: Vec<TaskScore>,
}

impl EvalReport {
    /// Extract the committed-baseline view (suite id + digest + Gate values).
    #[must_use]
    pub fn to_baseline(&self) -> Baseline {
        Baseline {
            suite_id: self.suite_id.clone(),
            suite_digest: self.suite_digest.clone(),
            gates: self.gates.clone(),
        }
    }

    /// Render the report as pretty JSON.
    ///
    /// # Errors
    /// Propagates a `serde_json` error only if a Spike metric value is non-finite
    /// (the scorers never produce one).
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}

/// The integer mean of a set of per-mille values (floor), or `None` for an empty set.
fn mean_per_mille(values: &[u32]) -> Option<u32> {
    if values.is_empty() {
        return None;
    }
    let sum: u64 = values.iter().map(|v| u64::from(*v)).sum();
    u32::try_from(sum / values.len() as u64).ok()
}

/// Fold per-task scores + the corpus-level format-coverage score + any Tier-B spikes
/// into an [`EvalReport`]. The aggregate Gate value for each transcript metric is the
/// integer mean over the tasks where it applied.
#[must_use]
pub fn aggregate(
    suite_id: String,
    suite_digest: String,
    per_task: Vec<TaskScore>,
    format_coverage: &ScoreOutput,
    spikes: &[ScoreOutput],
    env_label: String,
    git_sha: String,
) -> EvalReport {
    let mut gates: Vec<GateValue> = Vec::new();

    // Each per-transcript metric: integer mean of the applicable per-task scores.
    for id in TRANSCRIPT_SCORER_IDS {
        let values: Vec<u32> = per_task
            .iter()
            .flat_map(|t| &t.scores)
            .filter(|s| s.metric_id == id)
            .filter_map(ScoreOutput::gate_per_mille)
            .collect();
        if let Some(m) = mean_per_mille(&values) {
            gates.push(GateValue {
                id: id.to_string(),
                per_mille: m,
            });
        }
    }

    // The corpus-level format-coverage gate.
    if let Some(per_mille) = format_coverage.gate_per_mille() {
        gates.push(GateValue {
            id: format_coverage.metric_id.clone(),
            per_mille,
        });
    }

    // The trend record's Spikes (Tier-B latency etc.) — kept verbatim, never gated.
    let spike_metrics: Vec<SpikeMetric> = spikes
        .iter()
        .filter_map(|s| match &s.value {
            ScoreValue::Spike { value, unit } => Some(SpikeMetric {
                id: s.metric_id.clone(),
                value: *value,
                unit: unit.clone(),
            }),
            ScoreValue::Gate { .. } => None,
        })
        .collect();

    EvalReport {
        schema: SCHEMA_VERSION,
        suite_id,
        suite_digest,
        git_sha,
        env_label,
        gates,
        spikes: spike_metrics,
        per_task,
    }
}

/// Compare a run to its baseline. Fails closed on corpus drift; otherwise reports every
/// Gate that fell below `baseline - tolerance_per_mille`.
///
/// # Errors
/// Returns [`EvalError::CorpusDrift`] when the report and baseline were captured on
/// different corpora (their `suite_digest` differs) — the operator must re-baseline.
pub fn compare_to_baseline(
    report: &EvalReport,
    baseline: &Baseline,
    tolerance_per_mille: u32,
) -> Result<BaselineComparison, EvalError> {
    if report.suite_digest != baseline.suite_digest {
        return Err(EvalError::CorpusDrift {
            baseline: baseline.suite_digest.clone(),
            current: report.suite_digest.clone(),
        });
    }
    let mut regressions = Vec::new();
    for base in &baseline.gates {
        let current = report
            .gates
            .iter()
            .find(|g| g.id == base.id)
            .map_or(0, |g| g.per_mille);
        // Regression iff current + tolerance < baseline (integer comparison).
        if current.saturating_add(tolerance_per_mille) < base.per_mille {
            regressions.push(Regression {
                metric_id: base.id.clone(),
                baseline_per_mille: base.per_mille,
                current_per_mille: current,
            });
        }
    }
    Ok(BaselineComparison {
        ok: regressions.is_empty(),
        regressions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scorers::{ScoreOutput, PER_MILLE};

    fn task(id: &str, success: u32, f1: u32) -> TaskScore {
        TaskScore {
            task_id: id.into(),
            scores: vec![
                ScoreOutput::gate("task_success", success, ""),
                ScoreOutput::gate("tool_call_f1", f1, ""),
                ScoreOutput::not_applicable("groundedness", ""),
                ScoreOutput::gate("loop_efficiency", PER_MILLE, ""),
            ],
        }
    }

    fn report() -> EvalReport {
        aggregate(
            "golden-v1".into(),
            "deadbeef".into(),
            vec![task("a", 1000, 1000), task("b", 0, 500)],
            &ScoreOutput::gate("format_coverage", 800, ""),
            &[],
            "test-env".into(),
            "sha".into(),
        )
    }

    #[test]
    fn aggregate_is_integer_mean() {
        let r = report();
        let success = r.gates.iter().find(|g| g.id == "task_success").unwrap();
        assert_eq!(success.per_mille, 500); // (1000 + 0) / 2
        let f1 = r.gates.iter().find(|g| g.id == "tool_call_f1").unwrap();
        assert_eq!(f1.per_mille, 750); // (1000 + 500) / 2
                                       // groundedness was N/A for every task ⇒ no gate emitted.
        assert!(r.gates.iter().all(|g| g.id != "groundedness"));
        // format_coverage carried through.
        assert_eq!(
            r.gates
                .iter()
                .find(|g| g.id == "format_coverage")
                .unwrap()
                .per_mille,
            800
        );
    }

    #[test]
    fn no_regression_against_self() {
        let r = report();
        let cmp = compare_to_baseline(&r, &r.to_baseline(), 0).unwrap();
        assert!(cmp.ok);
    }

    #[test]
    fn regression_detected() {
        let r = report();
        let mut base = r.to_baseline();
        // raise the baseline so the current run is now "below" it.
        for g in &mut base.gates {
            g.per_mille = PER_MILLE;
        }
        let cmp = compare_to_baseline(&r, &base, 0).unwrap();
        assert!(!cmp.ok);
        assert!(cmp
            .regressions
            .iter()
            .any(|x| x.metric_id == "task_success"));
    }

    #[test]
    fn corpus_drift_fails_closed() {
        let r = report();
        let mut base = r.to_baseline();
        base.suite_digest = "different".into();
        assert!(matches!(
            compare_to_baseline(&r, &base, 0),
            Err(EvalError::CorpusDrift { .. })
        ));
    }
}
