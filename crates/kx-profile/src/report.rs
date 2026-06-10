//! The structured profiling report (schema-1 JSON).
//!
//! A [`Report`] is the durable, comparable record Golden Rule 10 mandates:
//! `git_sha` + a fully-populated [`Environment`] + a flat list of
//! [`Metric`]s. The [`Environment`] is a **required, non-`Option`** field, so a
//! report literally cannot serialize without its environment label
//! (make-illegal-states-unrepresentable, Rule 5.2) — there is no code path that
//! emits an unlabelled number.

use serde::{Deserialize, Serialize};

use crate::env::Environment;

/// The report schema version. Bump on any breaking change to the JSON shape so
/// the private trend record can discriminate old captures.
pub const SCHEMA_VERSION: u32 = 1;

/// Whether a metric is a CI regression *gate* (a ratio that holds across
/// substrates) or a measurement-only *spike* (an absolute latency/throughput
/// captured for the trend but never a flaky CI assertion). Golden Rule 10(d).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricKind {
    /// A ratio-expressible regression gate (e.g. fold linearity).
    Gate,
    /// A measurement-only spike (absolute latency / throughput).
    Spike,
}

/// One captured measurement: a stable id, a numeric value, its unit, and its
/// gate-vs-spike classification.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Metric {
    /// A stable identifier (e.g. `warmup_to_serving_p50`).
    pub id: String,
    /// The measured value.
    pub value: f64,
    /// The unit (e.g. `ms`, `commits_per_s`, `us_per_entry`).
    pub unit: String,
    /// Gate (ratio regression guard) vs spike (measurement-only).
    pub kind: MetricKind,
}

impl Metric {
    /// Construct a measurement-only spike metric.
    #[must_use]
    pub fn spike(id: impl Into<String>, value: f64, unit: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            value,
            unit: unit.into(),
            kind: MetricKind::Spike,
        }
    }
}

/// The full profiling report. Serializes to the schema-1 JSON the private
/// `docs/benchmarks/` trend record consumes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Report {
    /// The report schema version ([`SCHEMA_VERSION`]).
    pub schema: u32,
    /// The commit the runtime was profiled at (`git rev-parse HEAD`).
    pub git_sha: String,
    /// The machine + toolchain the numbers were captured on. REQUIRED — a
    /// report cannot exist without it (Golden Rule 10).
    pub env: Environment,
    /// The captured metrics.
    pub metrics: Vec<Metric>,
}

impl Report {
    /// Assemble a schema-1 report from a captured environment + metrics.
    #[must_use]
    pub fn new(git_sha: String, env: Environment, metrics: Vec<Metric>) -> Self {
        Self {
            schema: SCHEMA_VERSION,
            git_sha,
            env,
            metrics,
        }
    }

    /// Render the report as pretty JSON.
    ///
    /// # Errors
    /// Returns [`serde_json::Error`] only if a metric value is a non-finite
    /// float (`serde_json` rejects `NaN`/`Inf`) — the spikes never produce one,
    /// but the error is surfaced rather than panicked.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::env::Environment;

    fn sample_env() -> Environment {
        Environment {
            host: "test-host".into(),
            os: "macos".into(),
            arch: "aarch64".into(),
            cpu: "Apple M-series".into(),
            cores: 10,
            toolchain: "rustc 1.x".into(),
            features: vec!["default".into()],
        }
    }

    #[test]
    fn report_serializes_schema_1_with_env() {
        let r = Report::new(
            "deadbeef".into(),
            sample_env(),
            vec![Metric::spike("warmup_to_serving_p50", 12.5, "ms")],
        );
        let v: serde_json::Value = serde_json::from_str(&r.to_json().unwrap()).unwrap();
        assert_eq!(v["schema"], 1);
        assert_eq!(v["git_sha"], "deadbeef");
        // The env block is present and fully populated.
        assert_eq!(v["env"]["host"], "test-host");
        assert_eq!(v["env"]["cores"], 10);
        assert_eq!(v["metrics"][0]["kind"], "spike");
        assert_eq!(v["metrics"][0]["unit"], "ms");
    }

    #[test]
    fn report_roundtrips() {
        let r = Report::new("sha".into(), sample_env(), vec![]);
        let back: Report = serde_json::from_str(&r.to_json().unwrap()).unwrap();
        assert_eq!(r, back);
    }
}
