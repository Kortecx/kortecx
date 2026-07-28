//! The eval report + the baseline gate.
//!
//! [`aggregate`] folds the per-task scores into suite-level integer Gate values plus an
//! env-labelled trend record. [`compare_to_baseline`] is the regression ratchet: it
//! fails closed on corpus drift (a changed `suite_digest` must be re-baselined
//! deliberately) and reports any Gate that fell below the committed baseline (minus a
//! per-mille tolerance). The decision is pure integer arithmetic — no float on the gate
//! path. The report mirrors `kx-profile`'s Gate/Spike model but is a
//! standalone type so the harness stays a dependency-light leaf.

use serde::{Deserialize, Serialize};

use crate::error::EvalError;
use crate::scorers::{ScoreOutput, ScoreValue, TRANSCRIPT_SCORER_IDS};

/// The report schema version (bump on any breaking JSON-shape change).
pub const SCHEMA_VERSION: u32 = 1;

/// The unit string recorded for a Gate metric in the trend report.
pub const GATE_UNIT: &str = "per_mille";

/// A measurement-only Spike (e.g. a Tier-B latency) — recorded, never gated. Mirrors
/// `kx_profile::Metric` of kind Spike. Spikes travel with the gates into the committed
/// baseline so the docs check can hold a published absolute to a committed source, but
/// the comparison surface stays [`Baseline::gates`] alone — [`compare_to_baseline`]
/// never reads a spike.
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
    /// The capability FAMILY the task belongs to (`GoldenTask::family`), carried so
    /// [`aggregate`] can emit a per-family gate beside each suite-wide one. Defaults to
    /// empty when reading an older trend record (which predates per-family gates).
    #[serde(default)]
    pub family: String,
    /// Every scorer's output for this task.
    pub scores: Vec<ScoreOutput>,
}

/// The separator between a metric id and a family in a per-family gate id
/// (`task_success@swarm`). Chosen because no scorer id contains it, so a family gate can
/// never collide with a suite-wide one.
pub const FAMILY_GATE_SEP: char = '@';

/// One aggregate Gate metric — a stable id and an integer per-mille value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateValue {
    /// The metric id (e.g. `"task_success"`).
    pub id: String,
    /// The aggregate score, `0..=1000`.
    pub per_mille: u32,
}

/// What produced a set of numbers — the label without which a score is not a record.
///
/// A real-model score is meaningless without the model that produced it, and until this
/// existed the committed baselines carried none: [`EvalReport::to_baseline`] dropped both
/// `env_label` and `git_sha`, so the only labelled artifact was a gitignored trend file
/// no reader ever sees. Two engines running two DIFFERENT models were published under one
/// heading, and nothing could catch it.
///
/// Deliberately NOT part of the comparison: running on another machine is not a
/// regression, and folding the host into the ratchet would make every number
/// unreproducible by construction. It is here to be *read* — by a person, and by the
/// docs check that holds the README's stated model and hardware to it.
///
/// This crate is a pure leaf: it defines the shape and never captures it. The caller that
/// already knows the engine and the served model fills it in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaselineEnv {
    /// The inference engine (`"ollama"` / `"llamacpp"`).
    pub engine: String,
    /// The served model id, exactly as the serve reported it — including any
    /// quantisation suffix. The two engines do not run the same build, and this is the
    /// field that makes that impossible to paper over.
    pub model: String,
    /// The host OS (`std::env::consts::OS`).
    pub os: String,
    /// The host architecture (`std::env::consts::ARCH`).
    pub arch: String,
    /// Logical cores.
    pub cores: u32,
    /// How many tasks the suite held when this was captured — the denominator behind
    /// every per-family score.
    pub task_count: u32,
    /// Capture wall clock, seconds since the Unix epoch. Stored as an integer rather
    /// than a formatted date so this crate needs no clock and no date dependency.
    pub captured_unix_s: u64,
    /// The commit the capture ran at.
    pub git_sha: String,
}

/// The committed yardstick — the suite's Gate values at a known corpus digest. Lives at
/// `corpus/golden-v1/baseline.json` (committed, NOT in the gitignored `docs/benchmarks/`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Baseline {
    /// The suite the baseline was captured on.
    pub suite_id: String,
    /// The corpus content digest (hex) the baseline is valid for.
    pub suite_digest: String,
    /// What produced these numbers. `None` on a baseline captured before the label
    /// existed — read-compatible, so an old committed file still loads and still gates.
    #[serde(default)]
    pub env: Option<BaselineEnv>,
    /// The Gate values, in a stable id order.
    pub gates: Vec<GateValue>,
    /// The capture run's Spikes, carried verbatim under the [`BaselineEnv`] rule: here
    /// to be *read* — by a person, and by the docs check that holds a published
    /// absolute (tokens, latency) to the committed capture — never compared.
    /// [`compare_to_baseline`] iterates `gates` alone; a slower host moves these
    /// numbers and that is not a regression. Empty on a baseline captured before
    /// spikes travelled (read-compatible).
    #[serde(default)]
    pub spikes: Vec<SpikeMetric>,
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
    /// A short environment label (e.g. `"macos/aarch64 (8 cores)"`) — the "a number
    /// with no environment label is not a record" discipline, kept lightweight.
    pub env_label: String,
    /// The structured form of that label, which is what survives into a committed
    /// baseline. `None` for a report whose producer did not supply one (the
    /// deterministic golden tier needs no host label — it has no host in its answer).
    #[serde(default)]
    pub env: Option<BaselineEnv>,
    /// The aggregate Gate values (the regression-gated surface).
    pub gates: Vec<GateValue>,
    /// Measurement-only Spikes (Tier-B latency etc.; advisory, never gated).
    pub spikes: Vec<SpikeMetric>,
    /// The per-task score breakdown.
    pub per_task: Vec<TaskScore>,
}

impl EvalReport {
    /// Extract the committed-baseline view: suite id + digest + env label + Gate values
    /// + the capture's Spikes (readable, never compared — see [`Baseline::spikes`]).
    #[must_use]
    pub fn to_baseline(&self) -> Baseline {
        Baseline {
            suite_id: self.suite_id.clone(),
            suite_digest: self.suite_digest.clone(),
            env: self.env.clone(),
            gates: self.gates.clone(),
            spikes: self.spikes.clone(),
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
///
/// Beside each suite-wide gate, a **per-family** gate (`task_success@swarm`) is emitted
/// for every `(metric, family)` pair that applied. A suite covering several substrate
/// families needs both: the suite-wide mean answers "did quality move", the per-family
/// gates answer "where" — without them one family's regression is diluted by the others'
/// task count, and a family whose tasks expect no tools (scoring a vacuous full
/// `tool_call_f1`) silently lifts the suite-wide mean. Family gates are APPENDED after
/// the suite-wide ones, so a baseline captured before they existed keeps its gate order
/// and [`compare_to_baseline`] (which iterates the BASELINE's gates) is unaffected.
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

    // Per-family gates, appended in a deterministic (family, metric) order. A task with
    // no family (an older trend record) contributes only to the suite-wide gates.
    let families: std::collections::BTreeSet<&str> = per_task
        .iter()
        .map(|t| t.family.as_str())
        .filter(|f| !f.is_empty())
        .collect();
    for family in families {
        for id in TRANSCRIPT_SCORER_IDS {
            let values: Vec<u32> = per_task
                .iter()
                .filter(|t| t.family == family)
                .flat_map(|t| &t.scores)
                .filter(|s| s.metric_id == id)
                .filter_map(ScoreOutput::gate_per_mille)
                .collect();
            if let Some(m) = mean_per_mille(&values) {
                gates.push(GateValue {
                    id: format!("{id}{FAMILY_GATE_SEP}{family}"),
                    per_mille: m,
                });
            }
        }
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
        // The structured label is attached by the caller that knows the engine and the
        // served model; `aggregate` is given neither and must not invent them.
        env: None,
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
        task_in("core", id, success, f1)
    }

    fn task_in(family: &str, id: &str, success: u32, f1: u32) -> TaskScore {
        TaskScore {
            task_id: id.into(),
            family: family.into(),
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

    /// The point of a per-family gate: one family's collapse is VISIBLE even when the
    /// suite-wide mean, diluted by the other families' task count, barely moves.
    #[test]
    fn a_family_gate_isolates_what_the_suite_wide_mean_dilutes() {
        let r = aggregate(
            "bench-v1".into(),
            "deadbeef".into(),
            vec![
                task_in("tool", "t1", 1000, 1000),
                task_in("tool", "t2", 1000, 1000),
                task_in("tool", "t3", 1000, 1000),
                task_in("swarm", "s1", 0, 1000), // the one family that collapsed
            ],
            &ScoreOutput::not_applicable("format_coverage", "N/A"),
            &[],
            "test-env".into(),
            "sha".into(),
        );
        let gate = |id: &str| r.gates.iter().find(|g| g.id == id).map(|g| g.per_mille);
        // Suite-wide: 3 perfect + 1 zero ⇒ 750. Still a comfortable-looking number.
        assert_eq!(gate("task_success"), Some(750));
        // Per-family: the collapse is unambiguous, and the healthy family is untouched.
        assert_eq!(gate("task_success@swarm"), Some(0));
        assert_eq!(gate("task_success@tool"), Some(1000));
    }

    /// A baseline captured BEFORE per-family gates existed must keep passing: the
    /// comparison iterates the baseline's gates, so new report gates are additive.
    #[test]
    fn a_pre_family_baseline_still_compares_clean() {
        let r = report();
        let legacy = Baseline {
            suite_id: r.suite_id.clone(),
            suite_digest: r.suite_digest.clone(),
            env: None,
            // Only the suite-wide gates, as an older capture would have written them.
            gates: r
                .gates
                .iter()
                .filter(|g| !g.id.contains(FAMILY_GATE_SEP))
                .cloned()
                .collect(),
            spikes: vec![],
        };
        assert!(legacy.gates.iter().any(|g| g.id == "task_success"));
        assert!(compare_to_baseline(&r, &legacy, 0).unwrap().ok);
    }

    /// The label must survive into the committed file, because the committed file is the
    /// only artifact a reader ever sees — the labelled trend record is gitignored. This
    /// is the regression guard for the bug that `to_baseline` used to have: it built a
    /// baseline that dropped every trace of what produced it.
    #[test]
    fn a_baseline_carries_the_label_of_what_produced_it() {
        let mut r = report();
        r.env = Some(BaselineEnv {
            engine: "ollama".into(),
            model: "gemma3:12b".into(),
            os: "macos".into(),
            arch: "aarch64".into(),
            cores: 8,
            task_count: 16,
            captured_unix_s: 1_753_500_000,
            git_sha: "abc123".into(),
        });
        let round_tripped: Baseline =
            serde_json::from_str(&serde_json::to_string(&r.to_baseline()).unwrap()).unwrap();
        let env = round_tripped
            .env
            .expect("the label reaches the committed file");
        assert_eq!(env.model, "gemma3:12b");
        assert_eq!(
            env.task_count, 16,
            "the denominator behind every family score"
        );
    }

    /// An already-committed baseline predates the label. It must still deserialize and
    /// still gate — a measurement-contract addition that made every existing baseline
    /// unreadable would force a re-capture on machines that cannot run the model.
    #[test]
    fn a_baseline_written_before_the_label_still_loads_and_gates() {
        let legacy: Baseline = serde_json::from_str(
            r#"{"suite_id":"bench-v1","suite_digest":"d","gates":[{"id":"task_success","per_mille":1000}]}"#,
        )
        .expect("a label-less baseline still deserializes");
        assert!(legacy.env.is_none());
        let mut r = report();
        r.suite_digest = "d".into();
        r.gates = vec![GateValue {
            id: "task_success".into(),
            per_mille: 1000,
        }];
        assert!(compare_to_baseline(&r, &legacy, 0).unwrap().ok);
    }

    /// LOSING a measurement is a regression, not a pass. A metric that scores N/A on
    /// every task emits no gate at all, so a report can silently stop carrying one the
    /// baseline records — a serve whose telemetry sidecar went missing stops producing
    /// `model_time_share`, and "no number" must never read as "no change". The
    /// comparison reads an absent gate as 0, which is what makes that loud.
    #[test]
    fn a_gate_the_report_stopped_emitting_is_a_regression() {
        let r = report();
        let mut baseline = r.to_baseline();
        baseline.gates.push(GateValue {
            id: "model_time_share".to_string(),
            per_mille: 700,
        });
        assert!(
            !r.gates.iter().any(|g| g.id == "model_time_share"),
            "the report genuinely does not carry the gate — otherwise this proves nothing"
        );
        let cmp = compare_to_baseline(&r, &baseline, 200).unwrap();
        assert!(!cmp.ok, "a vanished gate must fail closed");
        let reg = cmp
            .regressions
            .iter()
            .find(|x| x.metric_id == "model_time_share")
            .expect("the vanished gate is reported by name");
        assert_eq!(
            reg.current_per_mille, 0,
            "absent reads as 0, not as baseline"
        );
    }

    /// Spikes are readable, never compared: a baseline whose every spike differs from
    /// the report's still compares clean. This is what makes committing them into the
    /// baseline honest — a slower host moves them and that is not a regression.
    #[test]
    fn spikes_in_a_baseline_are_never_compared() {
        let mut r = report();
        r.spikes = vec![SpikeMetric {
            id: "task_latency_ms_p50".into(),
            value: 1000.0,
            unit: "ms".into(),
        }];
        let mut base = r.to_baseline();
        assert!(!base.spikes.is_empty(), "the capture carried its spikes");
        base.spikes[0].value = 999_999.0; // a wildly different host
        base.spikes.push(SpikeMetric {
            id: "a_spike_the_report_never_produced".into(),
            value: 1.0,
            unit: "ms".into(),
        });
        assert!(compare_to_baseline(&r, &base, 0).unwrap().ok);
    }

    /// A baseline committed before spikes travelled has no `spikes` key at all. It must
    /// still deserialize and still gate.
    #[test]
    fn a_baseline_written_before_spikes_still_loads_and_gates() {
        let legacy: Baseline = serde_json::from_str(
            r#"{"suite_id":"bench-v1","suite_digest":"d","gates":[{"id":"task_success","per_mille":500}]}"#,
        )
        .expect("a spike-less baseline still deserializes");
        assert!(legacy.spikes.is_empty());
        let mut r = report();
        r.suite_digest = "d".into();
        assert!(compare_to_baseline(&r, &legacy, 0).unwrap().ok);
    }

    /// The pass^k machinery sentinel: `pass_k4@trials` is captured at 1000 by any
    /// successful capture, so a later gated run whose report lacks the phase entirely
    /// reads it as 0 — a hard regression BY NAME, even when every per-task pass^k value
    /// was captured at 0 and would compare 0-vs-0 silently. This pins the sentinel
    /// against future tolerance or comparison changes.
    #[test]
    fn a_skipped_pass_k_phase_is_named_by_the_trials_sentinel() {
        let r = report(); // carries no pass_k4 gates at all — the "phase skipped" shape
        let mut base = r.to_baseline();
        base.gates.push(GateValue {
            id: "pass_k4@trials".into(),
            per_mille: 1000,
        });
        base.gates.push(GateValue {
            id: "pass_k4@http-authed-lookup".into(),
            per_mille: 0, // a flagship captured at 0 cannot catch the skip…
        });
        let cmp = compare_to_baseline(&r, &base, 200).unwrap();
        assert!(!cmp.ok, "the sentinel must fail the skipped phase closed");
        let named: Vec<&str> = cmp.regressions.iter().map(|x| x.metric_id.as_str()).collect();
        assert!(named.contains(&"pass_k4@trials"), "…the sentinel names it");
        assert!(
            !named.contains(&"pass_k4@http-authed-lookup"),
            "the 0-captured flagship stays silent — which is why the sentinel exists"
        );
    }

    /// A task with no family (an older trend record round-tripped through serde)
    /// contributes to the suite-wide gates and to no family gate.
    #[test]
    fn an_unfamilied_task_emits_no_family_gate() {
        let mut t = task("a", 1000, 1000);
        t.family = String::new();
        let r = aggregate(
            "bench-v1".into(),
            "deadbeef".into(),
            vec![t],
            &ScoreOutput::not_applicable("format_coverage", "N/A"),
            &[],
            "test-env".into(),
            "sha".into(),
        );
        assert!(r.gates.iter().any(|g| g.id == "task_success"));
        assert!(r.gates.iter().all(|g| !g.id.contains(FAMILY_GATE_SEP)));
    }
}
