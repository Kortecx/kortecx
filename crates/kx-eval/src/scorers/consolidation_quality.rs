//! Consolidation-quality scorer (RC5b) — did the agent DISTILL its episodic memories
//! into ONE durable fact, and ground its answer on it?
//!
//! The task declares the facts a consolidation must capture (`consolidation_must_capture`).
//! Unlike [`super::memory_quality`] (which checks each fact appears in ANY recalled
//! memory), this requires ALL facts to be collapsed into a SINGLE recalled entry (the
//! distilled semantic summary) AND grounded in the final answer. A consolidation that
//! produced/recalled nothing — the fail-closed class (memory disabled, a mis-routed
//! promptless turn, the model ignored the tool) — scores **0**: that is the point of the
//! gate (the `T-RERANK-WORKER-ROUTE` lesson applied to consolidation). Tasks that declare
//! no `consolidation_must_capture` are N/A. Deterministic, LLM-free, integer per-mille (SN-8).

use crate::scorers::{ScoreOutput, PER_MILLE};

use super::ScoreInput;

pub(super) fn score(input: &ScoreInput) -> ScoreOutput {
    let required = &input.expect.consolidation_must_capture;
    if required.is_empty() {
        return ScoreOutput::not_applicable(
            "consolidation_quality",
            "task declares no consolidation_must_capture",
        );
    }
    let answer = input.transcript.answer_text().unwrap_or_default();
    let recalled = &input.transcript.retrieved_docs;

    // Distilled into ONE entry: some recalled doc contains EVERY required fact.
    let captured_in_one = recalled
        .iter()
        .any(|doc| required.iter().all(|f| doc.contains(f.as_str())));
    // Grounded: the answer reflects every required fact.
    let grounded = required.iter().all(|f| answer.contains(f.as_str()));

    let ok = captured_in_one && grounded;
    ScoreOutput::gate(
        "consolidation_quality",
        if ok { PER_MILLE } else { 0 },
        format!(
            "{} of {} facts distilled into one entry AND grounded",
            if ok { required.len() } else { 0 },
            required.len()
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::suite::{Expectation, ExpectedTerminal};
    use crate::transcript::{Branch, Transcript, TurnRecord};

    fn consolidate_run(answer: &str, recalled: &[&str]) -> Transcript {
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

    fn expect_capture(facts: &[&str]) -> Expectation {
        Expectation {
            terminal: ExpectedTerminal::Answer,
            answer_must_contain: vec![],
            expected_tools: vec![],
            grounded_in: vec![],
            rerank_best_index: None,
            rerank_top_k: 0,
            memory_must_recall: vec![],
            consolidation_must_capture: facts.iter().map(|s| (*s).to_string()).collect(),
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
    fn distilled_into_one_entry_and_grounded_scores_full() {
        // All three facts appear in ONE recalled entry (the distilled summary) AND in the answer.
        let t = consolidate_run(
            "Consolidated: deadline March 3rd, prefers email, waters plants Fridays.",
            &["deadline March 3rd; prefers email; waters plants Fridays"],
        );
        let e = expect_capture(&["March 3rd", "email", "Fridays"]);
        assert_eq!(run(&t, &e).gate_per_mille(), Some(PER_MILLE));
    }

    #[test]
    fn nothing_recalled_scores_zero() {
        // THE fail-closed guard: consolidation produced nothing → 0, even if the answer mentions the facts.
        let t = consolidate_run("deadline March 3rd, email, Fridays", &[]);
        let e = expect_capture(&["March 3rd", "email", "Fridays"]);
        assert_eq!(run(&t, &e).gate_per_mille(), Some(0));
    }

    #[test]
    fn facts_split_across_entries_scores_zero() {
        // Recalled but NOT distilled into one entry (each fact in a separate doc) → 0.
        let t = consolidate_run(
            "deadline March 3rd, email, Fridays",
            &[
                "deadline March 3rd",
                "prefers email",
                "waters plants Fridays",
            ],
        );
        let e = expect_capture(&["March 3rd", "email", "Fridays"]);
        assert_eq!(run(&t, &e).gate_per_mille(), Some(0));
    }

    #[test]
    fn captured_but_not_grounded_scores_zero() {
        // Distilled into one entry, but the answer ignored it → 0.
        let t = consolidate_run(
            "I don't know.",
            &["deadline March 3rd; prefers email; waters plants Fridays"],
        );
        let e = expect_capture(&["March 3rd", "email", "Fridays"]);
        assert_eq!(run(&t, &e).gate_per_mille(), Some(0));
    }

    #[test]
    fn no_required_facts_is_na() {
        let t = consolidate_run("anything", &["x"]);
        let e = expect_capture(&[]);
        let s = run(&t, &e);
        assert!(!s.applicable);
        assert_eq!(s.gate_per_mille(), None);
    }
}
