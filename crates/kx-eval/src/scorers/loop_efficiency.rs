//! Loop-efficiency scorer — how economically did the run reach its terminal?
//!
//! `efficiency = (ideal_turns + ideal_tool_calls) / (turns_used + tool_calls_used)`,
//! capped at 1000 per-mille (a run that beats the ideal is "perfect", never >100%). A
//! model that loops, re-tries, or fires redundant tools spends more turns/calls and
//! scores lower — the direct measure of agentic economy the RC tunes against.

use crate::scorers::{ScoreOutput, PER_MILLE};

use super::ScoreInput;

pub(super) fn score(input: &ScoreInput) -> ScoreOutput {
    let ideal = u64::from(input.expect.ideal_turns) + u64::from(input.expect.ideal_tool_calls);
    let used =
        u64::from(input.transcript.turns_used()) + u64::from(input.transcript.tool_calls_used());

    let per_mille = if used == 0 {
        // An empty run is perfect only if nothing was needed.
        if ideal == 0 {
            PER_MILLE
        } else {
            0
        }
    } else {
        u32::try_from((ideal * u64::from(PER_MILLE) / used).min(u64::from(PER_MILLE)))
            .unwrap_or(PER_MILLE)
    };
    let detail = format!(
        "ideal {ideal} steps vs used {used} (turns {} + tools {})",
        input.transcript.turns_used(),
        input.transcript.tool_calls_used()
    );
    ScoreOutput::gate("loop_efficiency", per_mille, detail)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::suite::{Expectation, ExpectedTerminal};
    use crate::transcript::{Branch, Transcript, TurnRecord};

    fn run(n_tool_turns: u32, answered: bool) -> Transcript {
        let mut turns = vec![];
        for i in 0..n_tool_turns {
            turns.push(TurnRecord {
                turn: i,
                branch: Branch::Tool,
                tool_id: "kv/get".into(),
                tool_version: "1".into(),
                call_index: 0,
                rejection_reason: String::new(),
            });
        }
        if answered {
            turns.push(TurnRecord {
                turn: n_tool_turns,
                branch: Branch::Answer,
                tool_id: String::new(),
                tool_version: String::new(),
                call_index: 0,
                rejection_reason: String::new(),
            });
        }
        Transcript {
            task_id: "t".into(),
            turns,
            final_answer: answered.then(|| "ok".to_string()),
            retrieved_docs: vec![],
            rerank: None,
            max_turns: 8,
            max_tool_calls: 20,
        }
    }

    fn expect(ideal_turns: u32, ideal_tool_calls: u32) -> Expectation {
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
            ideal_turns,
            ideal_tool_calls,
        }
    }

    #[test]
    fn ideal_run_is_perfect() {
        // 1 tool turn + 1 answer turn = 2 turns, 1 tool call ⇒ used 3; ideal 2 turns + 1 call = 3.
        let t = run(1, true);
        let e = expect(2, 1);
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
    fn wasteful_run_scores_lower() {
        // 3 tool turns + 1 answer = 4 turns, 3 tool calls ⇒ used 7; ideal 3 ⇒ 3*1000/7 = 428.
        let t = run(3, true);
        let e = expect(2, 1);
        assert_eq!(
            score(&ScoreInput {
                transcript: &t,
                expect: &e
            })
            .gate_per_mille(),
            Some(428)
        );
    }

    #[test]
    fn beating_ideal_is_capped() {
        let t = run(0, true); // 1 turn, 0 calls ⇒ used 1
        let e = expect(5, 5); // ideal 10 ⇒ would be 10000, capped at 1000
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
