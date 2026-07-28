//! Tool-call sequence-accuracy scorers — the order-sensitive companions to
//! `tool_call_f1`.
//!
//! NESTFUL-style, our own definitions (never NESTFUL-comparable): the gold sequence is
//! the task's `expected_tools` array read IN ORDER (the corpus authors chains in call
//! order, and that order is normative here), the actual sequence is the run's tool
//! calls in `(seq, call_index)` order.
//!
//! - `tool_seq_fsa` (full sequence accuracy) — binary per task: the actual sequence
//!   equals the gold sequence exactly.
//! - `tool_seq_psa` (partial sequence accuracy) — graded per task:
//!   `|LCS(actual, gold)| · 1000 / max(|actual|, |gold|)`, integer floor. The longest
//!   common subsequence respects order, so a run that made the right SET of calls in a
//!   broken order scores below 1000 here while `tool_call_f1` reads a perfect 1000 —
//!   that separation is the whole point of the column.
//!
//! Both are N/A when the task expects no tools (an empty gold sequence has no order to
//! respect; abstention is scored by `task_success`, never folded in here — the same
//! applicability rule `tool_call_f1` follows for its empty-gold degeneracy).

use crate::scorers::{ScoreOutput, PER_MILLE};
use crate::suite::ExpectedToolCall;
use crate::transcript::ToolKey;

use super::ScoreInput;

pub(super) fn score_fsa(input: &ScoreInput) -> ScoreOutput {
    let Some((actual, expected)) = sequences(input) else {
        return ScoreOutput::not_applicable("tool_seq_fsa", "task expects no tool calls");
    };
    let exact = actual == expected;
    let per_mille = if exact { PER_MILLE } else { 0 };
    let detail = format!(
        "actual sequence of {} {} the expected sequence of {}",
        actual.len(),
        if exact { "equals" } else { "differs from" },
        expected.len()
    );
    ScoreOutput::gate("tool_seq_fsa", per_mille, detail)
}

pub(super) fn score_psa(input: &ScoreInput) -> ScoreOutput {
    let Some((actual, expected)) = sequences(input) else {
        return ScoreOutput::not_applicable("tool_seq_psa", "task expects no tool calls");
    };
    let lcs = lcs_len(&actual, &expected);
    let denom = actual.len().max(expected.len());
    let per_mille = u32::try_from(lcs * PER_MILLE as usize / denom).unwrap_or(0);
    let detail = format!(
        "LCS {lcs} over max(actual {}, expected {})",
        actual.len(),
        expected.len()
    );
    ScoreOutput::gate("tool_seq_psa", per_mille, detail)
}

/// The (actual, gold) call sequences, or `None` when the task expects no tools.
fn sequences(input: &ScoreInput) -> Option<(Vec<ToolKey>, Vec<ToolKey>)> {
    if input.expect.expected_tools.is_empty() {
        return None;
    }
    let expected: Vec<ToolKey> = input
        .expect
        .expected_tools
        .iter()
        .map(ExpectedToolCall::key)
        .collect();
    Some((input.transcript.actual_tool_calls(), expected))
}

/// Longest-common-subsequence length — the standard O(n·m) table, small inputs only
/// (call sequences are bounded by the tool-call budget of 20).
fn lcs_len(a: &[ToolKey], b: &[ToolKey]) -> usize {
    let mut row = vec![0usize; b.len() + 1];
    for x in a {
        let mut diag = 0usize;
        for (j, y) in b.iter().enumerate() {
            let up = row[j + 1];
            row[j + 1] = if x == y { diag + 1 } else { up.max(row[j]) };
            diag = up;
        }
    }
    row[b.len()]
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
            timing: None,
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
            answer_must_not_contain: vec![],
            forbidden_tools: vec![],
            max_turns: None,
            max_tool_calls: None,
            ideal_turns: 2,
            ideal_tool_calls: 1,
        }
    }

    fn fsa(t: &Transcript, e: &Expectation) -> Option<u32> {
        score_fsa(&ScoreInput {
            transcript: t,
            expect: e,
        })
        .gate_per_mille()
    }

    fn psa(t: &Transcript, e: &Expectation) -> Option<u32> {
        score_psa(&ScoreInput {
            transcript: t,
            expect: e,
        })
        .gate_per_mille()
    }

    #[test]
    fn exact_chain_is_perfect_on_both() {
        let t = run(vec![tool_turn(0, "kv/get", 0), tool_turn(1, "calc/add", 0)]);
        let e = expect(&["kv/get", "calc/add"]);
        assert_eq!(fsa(&t, &e), Some(PER_MILLE));
        assert_eq!(psa(&t, &e), Some(PER_MILLE));
    }

    /// THE null this column exists for: the right SET in the wrong ORDER. `tool_call_f1`
    /// reads a perfect 1000 here (order-tolerant multiset, pinned in `tool_calls`);
    /// the sequence column must separate it.
    #[test]
    fn right_set_wrong_order_separates_from_f1() {
        let t = run(vec![tool_turn(0, "calc/add", 0), tool_turn(1, "kv/get", 0)]);
        let e = expect(&["kv/get", "calc/add"]);
        assert_eq!(fsa(&t, &e), Some(0));
        assert_eq!(psa(&t, &e), Some(500)); // LCS 1 of max(2,2)
        let f1 = super::super::tool_calls::score(&ScoreInput {
            transcript: &t,
            expect: &e,
        });
        assert_eq!(f1.gate_per_mille(), Some(PER_MILLE));
    }

    #[test]
    fn broken_chain_scores_partial() {
        // Chain stops after the first call: FSA 0, PSA = 1/2.
        let t = run(vec![tool_turn(0, "kv/get", 0)]);
        let e = expect(&["kv/get", "calc/add"]);
        assert_eq!(fsa(&t, &e), Some(0));
        assert_eq!(psa(&t, &e), Some(500));
    }

    #[test]
    fn extra_interleaved_call_dilutes_psa() {
        // Gold order preserved but a spurious call sits between: FSA 0, PSA = 2/3.
        let t = run(vec![
            tool_turn(0, "kv/get", 0),
            tool_turn(1, "echo/echo", 0),
            tool_turn(2, "calc/add", 0),
        ]);
        let e = expect(&["kv/get", "calc/add"]);
        assert_eq!(fsa(&t, &e), Some(0));
        assert_eq!(psa(&t, &e), Some(666));
    }

    #[test]
    fn no_calls_at_all_scores_zero() {
        let t = run(vec![]);
        let e = expect(&["kv/get"]);
        assert_eq!(fsa(&t, &e), Some(0));
        assert_eq!(psa(&t, &e), Some(0));
    }

    #[test]
    fn empty_gold_is_na_never_folded() {
        // Abstention tasks (empty expected_tools) are N/A here — scored by
        // task_success, never by a sequence metric.
        let t = run(vec![]);
        let e = expect(&[]);
        let f = score_fsa(&ScoreInput {
            transcript: &t,
            expect: &e,
        });
        let p = score_psa(&ScoreInput {
            transcript: &t,
            expect: &e,
        });
        assert!(!f.applicable);
        assert!(!p.applicable);
    }

    #[test]
    fn repeated_tool_ids_respect_multiplicity_and_order() {
        // page, page, get — dropping the second page: LCS 2 of max(2,3).
        let t = run(vec![tool_turn(0, "fleet/page", 0), tool_turn(1, "fleet/get", 0)]);
        let e = expect(&["fleet/page", "fleet/page", "fleet/get"]);
        assert_eq!(fsa(&t, &e), Some(0));
        assert_eq!(psa(&t, &e), Some(666));
    }
}
