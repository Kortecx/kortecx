//! Task-success scorer — did the run reach the expected terminal, and (for an answer
//! task) does the committed answer contain the oracle substrings?
//!
//! Binary per task (1000 or 0 per-mille); the suite aggregate is the success RATE.

use crate::scorers::{ScoreOutput, PER_MILLE};
use crate::suite::ExpectedTerminal;
use crate::transcript::Branch;

use super::ScoreInput;

pub(super) fn score(input: &ScoreInput) -> ScoreOutput {
    let terminal = input.transcript.terminal_branch();
    let (ok, detail) = match input.expect.terminal {
        ExpectedTerminal::Answer => {
            if terminal == Branch::Answer {
                let answer = input.transcript.answer_text().unwrap_or_default();
                let missing: Vec<&str> = input
                    .expect
                    .answer_must_contain
                    .iter()
                    .filter(|needle| !answer.contains(needle.as_str()))
                    .map(String::as_str)
                    .collect();
                if missing.is_empty() {
                    (true, "answer reached, oracle satisfied".to_string())
                } else {
                    (
                        false,
                        format!("answer missing oracle substrings: {missing:?}"),
                    )
                }
            } else {
                (false, format!("expected Answer, got {terminal:?}"))
            }
        }
        ExpectedTerminal::DeadLetter => {
            if terminal == Branch::DeadLettered {
                (true, "clean dead-letter terminal".to_string())
            } else {
                (false, format!("expected DeadLetter, got {terminal:?}"))
            }
        }
    };
    ScoreOutput::gate("task_success", if ok { PER_MILLE } else { 0 }, detail)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::suite::Expectation;
    use crate::transcript::{Transcript, TurnRecord};

    fn answer_run(answer: &str) -> Transcript {
        Transcript {
            task_id: "t".into(),
            turns: vec![TurnRecord {
                turn: 0,
                branch: Branch::Answer,
                tool_id: String::new(),
                tool_version: String::new(),
                call_index: 0,
                rejection_reason: String::new(),
            }],
            final_answer: Some(answer.to_string()),
            retrieved_docs: vec![],
            rerank: None,
            max_turns: 8,
            max_tool_calls: 20,
        }
    }

    fn expect_answer(must: &[&str]) -> Expectation {
        Expectation {
            terminal: ExpectedTerminal::Answer,
            answer_must_contain: must.iter().map(|s| (*s).to_string()).collect(),
            expected_tools: vec![],
            grounded_in: vec![],
            rerank_best_index: None,
            rerank_top_k: 0,
            ideal_turns: 1,
            ideal_tool_calls: 0,
        }
    }

    #[test]
    fn answer_with_oracle_succeeds() {
        let t = answer_run("the answer is 42");
        let e = expect_answer(&["42"]);
        let s = score(&ScoreInput {
            transcript: &t,
            expect: &e,
        });
        assert_eq!(s.gate_per_mille(), Some(PER_MILLE));
    }

    #[test]
    fn missing_oracle_fails() {
        let t = answer_run("the answer is 7");
        let e = expect_answer(&["42"]);
        let s = score(&ScoreInput {
            transcript: &t,
            expect: &e,
        });
        assert_eq!(s.gate_per_mille(), Some(0));
    }

    #[test]
    fn deadletter_expected_and_reached() {
        let t = Transcript {
            task_id: "t".into(),
            turns: vec![TurnRecord {
                turn: 0,
                branch: Branch::DeadLettered,
                tool_id: String::new(),
                tool_version: String::new(),
                call_index: 0,
                rejection_reason: "budget exhausted".into(),
            }],
            final_answer: None,
            retrieved_docs: vec![],
            rerank: None,
            max_turns: 8,
            max_tool_calls: 20,
        };
        let e = Expectation {
            terminal: ExpectedTerminal::DeadLetter,
            answer_must_contain: vec![],
            expected_tools: vec![],
            grounded_in: vec![],
            rerank_best_index: None,
            rerank_top_k: 0,
            ideal_turns: 8,
            ideal_tool_calls: 20,
        };
        let s = score(&ScoreInput {
            transcript: &t,
            expect: &e,
        });
        assert_eq!(s.gate_per_mille(), Some(PER_MILLE));
    }
}
