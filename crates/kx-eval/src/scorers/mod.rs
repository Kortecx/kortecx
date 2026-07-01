//! The scorers — each a **pure, total** function that grades one aspect of a run.
//!
//! Per-transcript scorers ([`score_transcript`]) read a [`Transcript`] + [`Expectation`]
//! and return a [`ScoreValue::Gate`] in per-mille. The cross-format parse scorer
//! ([`score_format_coverage`]) reads the format-case corpus and runs the runtime's real
//! `kx_toolcall::parse_tool_call` over each raw model string — it is the committed
//! measurement of the current (`T-GEMMA-PAREN`) parse coverage, the "before" RC2 raises.
//!
//! A Gate value is an integer per-mille (`0..=1000`); a pass/fail decision is therefore
//! an exact integer comparison, never a float (SN-8).

mod format_coverage;
mod groundedness;
mod loop_efficiency;
mod rerank_quality;
mod task_success;
mod tool_calls;

pub use format_coverage::{score_format_coverage, FormatCase, FormatExpectation};

use serde::{Deserialize, Serialize};

use crate::suite::Expectation;
use crate::transcript::Transcript;

/// One unit of "perfect" — a per-mille Gate score of 1000 is a flawless result.
pub(crate) const PER_MILLE: u32 = 1000;

/// The stable ids of the per-transcript scorers, in the order [`score_transcript`]
/// emits them. Kept public so the report aggregator and tests can enumerate them.
pub const TRANSCRIPT_SCORER_IDS: [&str; 5] = [
    "task_success",
    "tool_call_f1",
    "groundedness",
    "loop_efficiency",
    "rerank_quality",
];

/// The value a scorer produced: an integer Gate (the decision path) or an absolute
/// Spike (recorded for the trend, never gated). Make-illegal-states-unrepresentable —
/// a Gate cannot carry a float, a Spike cannot be compared to a baseline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ScoreValue {
    /// A regression-gated ratio in `0..=1000` per-mille.
    Gate {
        /// The score, `0..=1000`.
        per_mille: u32,
    },
    /// A measurement-only absolute value (e.g. latency ms) — recorded, never gated.
    Spike {
        /// The measured value.
        value: f64,
        /// The unit (e.g. `"ms"`).
        unit: String,
    },
}

/// `Some` when this score is N/A for the task (e.g. groundedness on a non-RAG task) and
/// should be excluded from the aggregate rather than counted as 0.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScoreOutput {
    /// The metric's stable id.
    pub metric_id: String,
    /// The score.
    pub value: ScoreValue,
    /// Whether the metric applies to this task (`false` ⇒ excluded from the aggregate).
    pub applicable: bool,
    /// A short human detail (never raw payloads).
    pub detail: String,
}

impl ScoreOutput {
    /// A per-mille Gate score that applies to the task.
    #[must_use]
    pub fn gate(metric_id: impl Into<String>, per_mille: u32, detail: impl Into<String>) -> Self {
        Self {
            metric_id: metric_id.into(),
            value: ScoreValue::Gate {
                per_mille: per_mille.min(PER_MILLE),
            },
            applicable: true,
            detail: detail.into(),
        }
    }

    /// A Gate score that does NOT apply to this task (excluded from the aggregate).
    #[must_use]
    pub fn not_applicable(metric_id: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            metric_id: metric_id.into(),
            value: ScoreValue::Gate { per_mille: 0 },
            applicable: false,
            detail: detail.into(),
        }
    }

    /// The per-mille value if this is an applicable Gate, else `None`.
    #[must_use]
    pub fn gate_per_mille(&self) -> Option<u32> {
        match (&self.value, self.applicable) {
            (ScoreValue::Gate { per_mille }, true) => Some(*per_mille),
            _ => None,
        }
    }
}

/// The input to a per-transcript scorer.
pub struct ScoreInput<'a> {
    /// The run being scored.
    pub transcript: &'a Transcript,
    /// What the run was expected to achieve.
    pub expect: &'a Expectation,
}

/// Run every per-transcript scorer over one run, in [`TRANSCRIPT_SCORER_IDS`] order.
#[must_use]
pub fn score_transcript(input: &ScoreInput) -> Vec<ScoreOutput> {
    vec![
        task_success::score(input),
        tool_calls::score(input),
        groundedness::score(input),
        loop_efficiency::score(input),
        rerank_quality::score(input),
    ]
}
