//! Tool-call correctness scorer — a multiset F1 of the run's actual tool calls against
//! the expected ones, by exact `(id, version)`.
//!
//! Order-tolerant (an F1 over the call multisets) so a model that reorders independent
//! lookups is not penalised, while a wrong/missing/extra call is. Answer-only tasks
//! (no expected calls, no actual calls) score a perfect 1000; a spurious call on such a
//! task drives the score down.

use std::collections::BTreeMap;

use crate::scorers::{ScoreOutput, PER_MILLE};
use crate::suite::ExpectedToolCall;
use crate::transcript::ToolKey;

use super::ScoreInput;

pub(super) fn score(input: &ScoreInput) -> ScoreOutput {
    let actual = input.transcript.actual_tool_calls();
    let expected: Vec<ToolKey> = input
        .expect
        .expected_tools
        .iter()
        .map(ExpectedToolCall::key)
        .collect();

    let total = actual.len() + expected.len();
    if total == 0 {
        return ScoreOutput::gate("tool_call_f1", PER_MILLE, "no tools expected or called");
    }

    // Multiset intersection: consume one actual per matched expected.
    let mut pool: BTreeMap<&ToolKey, usize> = BTreeMap::new();
    for k in &actual {
        *pool.entry(k).or_insert(0) += 1;
    }
    let mut matched = 0usize;
    for k in &expected {
        if let Some(c) = pool.get_mut(k) {
            if *c > 0 {
                *c -= 1;
                matched += 1;
            }
        }
    }

    // F1 = 2·matched / (|actual| + |expected|), in per-mille (integer floor).
    let per_mille = u32::try_from(2 * matched * PER_MILLE as usize / total).unwrap_or(PER_MILLE);
    let detail = format!(
        "matched {matched} of expected {} (actual {})",
        expected.len(),
        actual.len()
    );
    ScoreOutput::gate("tool_call_f1", per_mille, detail)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::suite::{Expectation, ExpectedTerminal, ExpectedToolCall};
    use crate::transcript::{Branch, Transcript, TurnRecord};

    fn tool_turn(turn: u32, id: &str, call_index: u32) -> TurnRecord {
        TurnRecord {
            turn,
            branch: Branch::Tool,
            tool_id: id.into(),
            tool_version: "1".into(),
            call_index,
            rejection_reason: String::new(),
        }
    }

    fn run(turns: Vec<TurnRecord>) -> Transcript {
        Transcript {
            task_id: "t".into(),
            turns,
            final_answer: Some("ok".to_string()),
            retrieved_docs: vec![],
            rerank: None,
            max_turns: 8,
            max_tool_calls: 20,
        }
    }

    fn expect(tools: &[&str]) -> Expectation {
        Expectation {
            terminal: ExpectedTerminal::Answer,
            answer_must_contain: vec![],
            expected_tools: tools
                .iter()
                .map(|id| ExpectedToolCall {
                    tool_id: (*id).to_string(),
                    tool_version: "1".into(),
                })
                .collect(),
            grounded_in: vec![],
            rerank_best_index: None,
            rerank_top_k: 0,
            memory_must_recall: vec![],
            consolidation_must_capture: vec![],
            skill_wish_tools: vec![],
            ideal_turns: 2,
            ideal_tool_calls: 1,
        }
    }

    #[test]
    fn exact_match_is_perfect() {
        let t = run(vec![tool_turn(0, "kv/get", 0)]);
        let e = expect(&["kv/get"]);
        assert_eq!(
            score(&ScoreInput {
                transcript: &t,
                expect: &e
            })
            .gate_per_mille(),
            Some(PER_MILLE)
        );
    }

    #[test]
    fn answer_only_no_tools_is_perfect() {
        let t = run(vec![]);
        let e = expect(&[]);
        assert_eq!(
            score(&ScoreInput {
                transcript: &t,
                expect: &e
            })
            .gate_per_mille(),
            Some(PER_MILLE)
        );
    }

    #[test]
    fn spurious_call_lowers_score() {
        // expected none, called one ⇒ F1 = 2*0/(1+0) = 0.
        let t = run(vec![tool_turn(0, "kv/get", 0)]);
        let e = expect(&[]);
        assert_eq!(
            score(&ScoreInput {
                transcript: &t,
                expect: &e
            })
            .gate_per_mille(),
            Some(0)
        );
    }

    #[test]
    fn partial_match_is_half() {
        // expected {kv,calc}, called {kv} ⇒ F1 = 2*1/(1+2) = 666 per-mille.
        let t = run(vec![tool_turn(0, "kv/get", 0)]);
        let e = expect(&["kv/get", "calc/add"]);
        assert_eq!(
            score(&ScoreInput {
                transcript: &t,
                expect: &e
            })
            .gate_per_mille(),
            Some(666)
        );
    }

    #[test]
    fn order_tolerant_batch() {
        // batch of two in turn 0, reversed order vs expected ⇒ still perfect.
        let t = run(vec![tool_turn(0, "calc/add", 1), tool_turn(0, "kv/get", 0)]);
        let e = expect(&["kv/get", "calc/add"]);
        assert_eq!(
            score(&ScoreInput {
                transcript: &t,
                expect: &e
            })
            .gate_per_mille(),
            Some(PER_MILLE)
        );
    }
}
