//! Model-time-share scorer — of the wall clock a task spent, how much was the model
//! actually generating, rather than the runtime around it?
//!
//! `share = model_ms / total_ms`, per-mille, capped at 1000. Higher is better: a run
//! that spends its time in the model is a run whose cost is the model's, and every
//! millisecond the runtime adds — scheduling, folding, committing, polling, tool
//! round-trips — pushes the score down. This is the only speed number in the suite that
//! is *gated*; absolute latencies ride alongside as Spikes.
//!
//! **Why a ratio and not milliseconds.** An absolute-millisecond gate reads differently
//! on a slower host with no code change at all, so it cannot tell a runtime regression
//! from a busier machine — it is a signal that moves when the thing it claims to measure
//! did not. The ratio moves both terms together: a slower GPU raises `model_ms` and
//! `total_ms` by the same amount, so the score drifts *up*, toward 1000. That asymmetry
//! is deliberate and worth stating plainly: this gate cannot false-FAIL on a slow host,
//! only false-PASS, and the report's environment label is what makes the latter visible.
//!
//! **Absent is not zero.** [`Transcript::timing`] is `None` when the host could not
//! measure the split (no telemetry sidecar, or a scripted fixture that authors none).
//! The score is then N/A and excluded from the aggregate, so no gate is emitted at all —
//! rather than a 0 that would read as "the runtime consumed the entire run". Once a
//! baseline records the gate, a later run that cannot measure it is missing a baseline
//! gate, which `compare_to_baseline` reads as 0 and fails closed. Losing the measurement
//! is therefore loud, which is the point.

use crate::scorers::{ScoreOutput, PER_MILLE};

use super::ScoreInput;

pub(super) fn score(input: &ScoreInput) -> ScoreOutput {
    let Some(t) = input.transcript.timing else {
        return ScoreOutput::not_applicable(
            "model_time_share",
            "no host timing for this run (telemetry sidecar absent, or a scripted fixture)",
        );
    };
    if t.total_ms == 0 {
        // A task that took no measurable time cannot be divided into shares. This is a
        // measurement gap, not a perfect or a terrible run — N/A, like a missing sidecar.
        return ScoreOutput::not_applicable(
            "model_time_share",
            "total wall clock measured as 0 ms — nothing to take a share of",
        );
    }
    let per_mille = u32::try_from(
        (u128::from(t.model_ms) * u128::from(PER_MILLE) / u128::from(t.total_ms))
            .min(u128::from(PER_MILLE)),
    )
    .unwrap_or(PER_MILLE);
    let overhead_ms = t.total_ms.saturating_sub(t.model_ms);
    let detail = format!(
        "model {}ms of {}ms total ({overhead_ms}ms not in the model)",
        t.model_ms, t.total_ms
    );
    ScoreOutput::gate("model_time_share", per_mille, detail)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::suite::{Expectation, ExpectedTerminal};
    use crate::transcript::{Branch, Transcript, TranscriptTiming, TurnRecord};

    fn transcript(timing: Option<TranscriptTiming>) -> Transcript {
        Transcript {
            task_id: "t".into(),
            turns: vec![TurnRecord {
                turn: 0,
                branch: Branch::Answer,
                tool_id: String::new(),
                tool_version: String::new(),
                call_index: 0,
                rejection_reason: String::new(),
                raw: Vec::new(),
            }],
            final_answer: Some("ok".into()),
            retrieved_docs: vec![],
            rerank: None,
            max_turns: 8,
            max_tool_calls: 6,
            timing,
        }
    }

    fn expect() -> Expectation {
        Expectation {
            terminal: ExpectedTerminal::Answer,
            answer_must_contain: vec![],
            expected_tools: vec![],
            grounded_in: vec![],
            rerank_best_index: None,
            rerank_top_k: 0,
            memory_must_recall: vec![],
            consolidation_must_capture: vec![],
            skill_wish_tools: vec![],
            answer_must_not_contain: vec![],
            forbidden_tools: vec![],
            max_turns: None,
            max_tool_calls: None,
            ideal_turns: 1,
            ideal_tool_calls: 0,
        }
    }

    fn score_of(timing: Option<TranscriptTiming>) -> ScoreOutput {
        let t = transcript(timing);
        let e = expect();
        score(&ScoreInput {
            transcript: &t,
            expect: &e,
        })
    }

    #[test]
    fn a_run_that_is_all_model_scores_full() {
        let s = score_of(Some(TranscriptTiming {
            total_ms: 1000,
            model_ms: 1000,
            output_tokens: None,
        }));
        assert_eq!(s.gate_per_mille(), Some(PER_MILLE));
    }

    #[test]
    fn runtime_overhead_pushes_the_share_down() {
        // The SAME model work, on a run the runtime took twice as long to get through.
        let fast = score_of(Some(TranscriptTiming {
            total_ms: 1000,
            model_ms: 800,
            output_tokens: None,
        }));
        let slow = score_of(Some(TranscriptTiming {
            total_ms: 2000,
            model_ms: 800,
            output_tokens: None,
        }));
        assert_eq!(fast.gate_per_mille(), Some(800));
        assert_eq!(slow.gate_per_mille(), Some(400));
        assert!(
            slow.gate_per_mille() < fast.gate_per_mille(),
            "added runtime overhead must lower the score"
        );
    }

    #[test]
    fn a_slower_host_cannot_false_fail_the_gate() {
        // The property the gate is chosen for: scale BOTH terms by the same host factor
        // (a slower machine does exactly this) and the score does not fall. An
        // absolute-millisecond gate would have failed here while nothing regressed.
        let baseline = score_of(Some(TranscriptTiming {
            total_ms: 1000,
            model_ms: 700,
            output_tokens: None,
        }));
        let slower_host = score_of(Some(TranscriptTiming {
            total_ms: 1000 + 500,
            model_ms: 700 + 500,
            output_tokens: None,
        }));
        assert!(
            slower_host.gate_per_mille() >= baseline.gate_per_mille(),
            "a uniformly slower host must not read as a regression"
        );
    }

    #[test]
    fn no_timing_is_not_applicable_rather_than_zero() {
        let s = score_of(None);
        assert!(!s.applicable, "a missing measurement is N/A");
        assert_eq!(s.gate_per_mille(), None, "and contributes no gate value");
    }

    #[test]
    fn a_zero_total_is_not_applicable_rather_than_zero() {
        let s = score_of(Some(TranscriptTiming {
            total_ms: 0,
            model_ms: 0,
            output_tokens: None,
        }));
        assert!(!s.applicable);
    }

    #[test]
    fn model_time_exceeding_the_total_is_capped_not_overflowed() {
        // Concurrency (or a clock skew between the sidecar and the harness) can report
        // more model time than wall clock. Cap rather than emit a >1000 gate.
        let s = score_of(Some(TranscriptTiming {
            total_ms: 100,
            model_ms: 400,
            output_tokens: None,
        }));
        assert_eq!(s.gate_per_mille(), Some(PER_MILLE));
    }
}
