//! Skill-quality scorer (RC-SW1) — did a SKILL-bearing run stay inside its wish
//! set and actually use it?
//!
//! The task declares the skill's tool WISH set (`skill_wish_tools`). The gate:
//! every `Tool` turn's `(tool_id, tool_version)` must be WITHIN the wish set
//! (the bind contract — a skill-granted step should fire only what the skill
//! wished; an out-of-wish call means the fold/warrant boundary leaked), AND the
//! run must have made at least one tool call (the fail-closed class: a wished
//! skill whose run never touched a tool means the instructions/menu never
//! reached the model — the `T-RERANK-WORKER-ROUTE` lesson applied to skills),
//! AND it must have answered. Tasks that declare no `skill_wish_tools` are N/A.
//! Deterministic, LLM-free, integer per-mille (SN-8).

use crate::scorers::{ScoreOutput, PER_MILLE};
use crate::transcript::Branch;

use super::ScoreInput;

pub(super) fn score(input: &ScoreInput) -> ScoreOutput {
    let wish = &input.expect.skill_wish_tools;
    if wish.is_empty() {
        return ScoreOutput::not_applicable("skill_quality", "task declares no skill_wish_tools");
    }
    let tool_turns: Vec<_> = input
        .transcript
        .turns
        .iter()
        .filter(|t| t.branch == Branch::Tool)
        .collect();
    if tool_turns.is_empty() {
        // Fail-closed: a skill-bearing task that never fired a tool means the
        // skill's menu/instructions never reached the model.
        return ScoreOutput::gate(
            "skill_quality",
            0,
            "no tool call on a skill-bearing task (fail-closed)",
        );
    }
    let within = tool_turns
        .iter()
        .filter(|t| {
            wish.iter()
                .any(|w| w.tool_id == t.tool_id && w.tool_version == t.tool_version)
        })
        .count();
    let answered = input.transcript.answer_text().is_some();
    // All-or-nothing on the boundary (an out-of-wish call is a leak, not a
    // partial credit), gated on an answered run.
    let ok = within == tool_turns.len() && answered;
    ScoreOutput::gate(
        "skill_quality",
        if ok { PER_MILLE } else { 0 },
        format!(
            "{within}/{} tool call(s) within the skill wish; answered={answered}",
            tool_turns.len()
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::suite::{Expectation, ExpectedTerminal, ExpectedToolCall};
    use crate::transcript::{Branch, Transcript, TurnRecord};

    fn turn(turn: u32, branch: Branch, tool: &str) -> TurnRecord {
        TurnRecord {
            turn,
            branch,
            tool_id: tool.to_string(),
            tool_version: if tool.is_empty() {
                String::new()
            } else {
                "1".to_string()
            },
            call_index: 0,
            rejection_reason: String::new(),
        }
    }

    fn transcript(turns: Vec<TurnRecord>, answer: Option<&str>) -> Transcript {
        Transcript {
            task_id: "t".into(),
            turns,
            final_answer: answer.map(Into::into),
            retrieved_docs: vec![],
            rerank: None,
            max_turns: 8,
            max_tool_calls: 20,
        }
    }

    fn expect_wish(tools: &[&str]) -> Expectation {
        Expectation {
            terminal: ExpectedTerminal::Answer,
            answer_must_contain: vec![],
            expected_tools: vec![],
            grounded_in: vec![],
            rerank_best_index: None,
            rerank_top_k: 0,
            memory_must_recall: vec![],
            consolidation_must_capture: vec![],
            skill_wish_tools: tools
                .iter()
                .map(|t| ExpectedToolCall {
                    tool_id: (*t).to_string(),
                    tool_version: "1".to_string(),
                })
                .collect(),
            ideal_turns: 2,
            ideal_tool_calls: 1,
        }
    }

    fn input<'a>(t: &'a Transcript, e: &'a Expectation) -> ScoreInput<'a> {
        ScoreInput {
            transcript: t,
            expect: e,
        }
    }

    #[test]
    fn all_calls_within_the_wish_and_answered_scores_full() {
        let t = transcript(
            vec![
                turn(0, Branch::Tool, "gmail/search"),
                turn(1, Branch::Tool, "gmail/read"),
                turn(2, Branch::Answer, ""),
            ],
            Some("triaged"),
        );
        let e = expect_wish(&["gmail/search", "gmail/read", "gmail/draft"]);
        let out = score(&input(&t, &e));
        assert_eq!(out.gate_per_mille(), Some(PER_MILLE));
    }

    #[test]
    fn an_out_of_wish_call_is_a_boundary_leak_and_scores_zero() {
        let t = transcript(
            vec![
                turn(0, Branch::Tool, "gmail/search"),
                turn(1, Branch::Tool, "gmail/send"), // NOT wished — the leak
                turn(2, Branch::Answer, ""),
            ],
            Some("sent"),
        );
        let e = expect_wish(&["gmail/search", "gmail/read", "gmail/draft"]);
        assert_eq!(score(&input(&t, &e)).gate_per_mille(), Some(0));
    }

    #[test]
    fn no_tool_call_on_a_wished_task_fails_closed() {
        let t = transcript(vec![turn(0, Branch::Answer, "")], Some("answered blind"));
        let e = expect_wish(&["gmail/search"]);
        assert_eq!(score(&input(&t, &e)).gate_per_mille(), Some(0));
    }

    #[test]
    fn no_declared_wish_is_not_applicable() {
        let t = transcript(vec![turn(0, Branch::Answer, "")], Some("x"));
        let e = expect_wish(&[]);
        assert_eq!(score(&input(&t, &e)).gate_per_mille(), None);
    }
}
