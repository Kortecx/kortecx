//! Retrieval-side context recall — did retrieval SURFACE the declared evidence,
//! whether or not the answer used it?
//!
//! The judge-free shape of RAGAS `NonLLMContextRecall`: the share of `grounded_in`
//! tokens present in at least one retrieved doc. Deliberately one-sided — it reads the
//! retrieval channel alone, so together with `groundedness` (which demands each token
//! in the answer AND a doc) the pair separates "retrieval never found it" from "the
//! model ignored what retrieval found". Tasks with no `grounded_in` are N/A.

use crate::scorers::{ScoreOutput, PER_MILLE};

use super::ScoreInput;

pub(super) fn score(input: &ScoreInput) -> ScoreOutput {
    let needles = &input.expect.grounded_in;
    if needles.is_empty() {
        return ScoreOutput::not_applicable("context_recall", "task declares no grounded tokens");
    }

    let docs = &input.transcript.retrieved_docs;
    let recalled = needles
        .iter()
        .filter(|tok| docs.iter().any(|d| d.contains(tok.as_str())))
        .count();

    let per_mille = u32::try_from(recalled * PER_MILLE as usize / needles.len()).unwrap_or(0);
    let detail = format!(
        "{recalled} of {} tokens present in a retrieved doc",
        needles.len()
    );
    ScoreOutput::gate("context_recall", per_mille, detail)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::suite::{Expectation, ExpectedTerminal};
    use crate::transcript::Transcript;

    fn run(answer: &str, docs: &[&str]) -> Transcript {
        Transcript {
            task_id: "t".into(),
            turns: vec![],
            final_answer: Some(answer.to_string()),
            retrieved_docs: docs.iter().map(|d| (*d).to_string()).collect(),
            rerank: None,
            max_turns: 8,
            max_tool_calls: 20,
            timing: None,
        }
    }

    fn expect_grounded(tokens: &[&str]) -> Expectation {
        Expectation {
            terminal: ExpectedTerminal::Answer,
            answer_must_contain: vec![],
            expected_tools: vec![],
            grounded_in: tokens.iter().map(|s| (*s).to_string()).collect(),
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

    fn recall(t: &Transcript, e: &Expectation) -> Option<u32> {
        score(&ScoreInput {
            transcript: t,
            expect: e,
        })
        .gate_per_mille()
    }

    #[test]
    fn retrieved_evidence_counts_even_when_the_answer_ignores_it() {
        // Retrieval surfaced the token; the answer never used it. context_recall reads
        // 1000 where groundedness reads 0 — the separation the pair exists for.
        let t = run("no idea", &["The callsign is ZEPHYR-77."]);
        let e = expect_grounded(&["ZEPHYR-77"]);
        assert_eq!(recall(&t, &e), Some(PER_MILLE));
        let g = super::super::groundedness::score(&ScoreInput {
            transcript: &t,
            expect: &e,
        });
        assert_eq!(g.gate_per_mille(), Some(0));
    }

    #[test]
    fn missing_evidence_reads_zero() {
        let t = run("ZEPHYR-77", &["nothing relevant"]);
        let e = expect_grounded(&["ZEPHYR-77"]);
        assert_eq!(recall(&t, &e), Some(0));
    }

    #[test]
    fn partial_recall_is_graded() {
        let t = run("x", &["doc mentions Paris only"]);
        let e = expect_grounded(&["Paris", "France"]);
        assert_eq!(recall(&t, &e), Some(500));
    }

    #[test]
    fn no_grounded_tokens_is_na() {
        let t = run("anything", &["a doc"]);
        let e = expect_grounded(&[]);
        let s = score(&ScoreInput {
            transcript: &t,
            expect: &e,
        });
        assert!(!s.applicable);
    }

    #[test]
    fn empty_retrieval_reads_zero_not_na() {
        // A RAG task whose retrieval returned nothing FAILED at recall — that is a
        // measurement, not an inapplicable metric.
        let t = run("ZEPHYR-77", &[]);
        let e = expect_grounded(&["ZEPHYR-77"]);
        assert_eq!(recall(&t, &e), Some(0));
    }
}
