//! Durable-memory quality scorer (RC5a) — did the agent actually RECALL what it
//! learned, and ground its answer on it?
//!
//! The task declares the facts the run must recall (`memory_must_recall`). The scorer
//! checks each required fact appears BOTH in the recalled memories (carried as
//! `retrieved_docs` — a recall observation is structurally a retrieval) AND in the
//! final answer (recalled AND grounded). A recall that silently returns nothing (the
//! fail-closed class — memory disabled, a mis-routed promptless turn, an empty index)
//! scores **0**: that is the point of this gate. Without it a fails-closed recall
//! passes every unit test (they use stubs) precisely because it fails closed — the
//! `T-RERANK-WORKER-ROUTE` lesson applied to memory. Tasks that declare no
//! `memory_must_recall` are N/A and excluded from the aggregate. Deterministic,
//! LLM-free, integer per-mille (SN-8).

use crate::scorers::{ScoreOutput, PER_MILLE};

use super::ScoreInput;

pub(super) fn score(input: &ScoreInput) -> ScoreOutput {
    let required = &input.expect.memory_must_recall;
    if required.is_empty() {
        return ScoreOutput::not_applicable(
            "memory_quality",
            "task declares no memory_must_recall",
        );
    }
    let answer = input.transcript.answer_text().unwrap_or_default();
    let recalled = &input.transcript.retrieved_docs;

    let hit = required
        .iter()
        .filter(|fact| {
            recalled.iter().any(|m| m.contains(fact.as_str())) && answer.contains(fact.as_str())
        })
        .count();

    let per_mille = u32::try_from(hit * PER_MILLE as usize / required.len()).unwrap_or(0);
    ScoreOutput::gate(
        "memory_quality",
        per_mille,
        format!("{hit} of {} facts recalled AND grounded", required.len()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::suite::{Expectation, ExpectedTerminal};
    use crate::transcript::{Branch, Transcript, TurnRecord};

    fn memory_run(answer: &str, recalled: &[&str]) -> Transcript {
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
            final_answer: Some(answer.into()),
            retrieved_docs: recalled.iter().map(|s| (*s).to_string()).collect(),
            rerank: None,
            max_turns: 8,
            max_tool_calls: 20,
        }
    }

    fn expect_recall(facts: &[&str]) -> Expectation {
        Expectation {
            terminal: ExpectedTerminal::Answer,
            answer_must_contain: vec![],
            expected_tools: vec![],
            grounded_in: vec![],
            rerank_best_index: None,
            rerank_top_k: 0,
            memory_must_recall: facts.iter().map(|s| (*s).to_string()).collect(),
            consolidation_must_capture: vec![],
            skill_wish_tools: vec![],
            ideal_turns: 1,
            ideal_tool_calls: 0,
        }
    }

    fn run(t: &Transcript, e: &Expectation) -> ScoreOutput {
        score(&ScoreInput {
            transcript: t,
            expect: e,
        })
    }

    #[test]
    fn recalled_and_grounded_scores_full() {
        // The fact "March 3rd" was recalled (in the memories) AND is in the answer.
        let t = memory_run(
            "Your deadline is March 3rd.",
            &["the deadline is March 3rd"],
        );
        let e = expect_recall(&["March 3rd"]);
        assert_eq!(run(&t, &e).gate_per_mille(), Some(PER_MILLE));
    }

    #[test]
    fn nothing_recalled_scores_zero() {
        // THE fail-closed guard: recall returned nothing (empty memories) → 0, even if the
        // answer happens to mention the fact.
        let t = memory_run("Your deadline is March 3rd.", &[]);
        let e = expect_recall(&["March 3rd"]);
        assert_eq!(run(&t, &e).gate_per_mille(), Some(0));
    }

    #[test]
    fn recalled_but_not_grounded_scores_zero() {
        // Recalled but the answer ignored it → not grounded → 0.
        let t = memory_run("I don't know.", &["the deadline is March 3rd"]);
        let e = expect_recall(&["March 3rd"]);
        assert_eq!(run(&t, &e).gate_per_mille(), Some(0));
    }

    #[test]
    fn no_required_facts_is_na() {
        let t = memory_run("anything", &["x"]);
        let e = expect_recall(&[]);
        let s = run(&t, &e);
        assert!(!s.applicable);
        assert_eq!(s.gate_per_mille(), None);
    }
}
