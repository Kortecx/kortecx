//! RAG groundedness scorer — is the answer grounded in the retrieved docs?
//!
//! Deterministic and LLM-free: each token the task declares as `grounded_in` must
//! appear both in the committed answer AND in at least one retrieved doc. (An LLM judge
//! would be non-deterministic and is fail-closed in `kx-critic`; RC4 may add a graded
//! variant as a Spike, but the GATE stays deterministic.) Tasks with no `grounded_in`
//! are N/A and excluded from the aggregate.

use crate::scorers::{ScoreOutput, PER_MILLE};

use super::ScoreInput;

pub(super) fn score(input: &ScoreInput) -> ScoreOutput {
    let needles = &input.expect.grounded_in;
    if needles.is_empty() {
        return ScoreOutput::not_applicable("groundedness", "task declares no grounded tokens");
    }

    let answer = input.transcript.answer_text().unwrap_or_default();
    let docs = &input.transcript.retrieved_docs;

    let grounded = needles
        .iter()
        .filter(|tok| {
            answer.contains(tok.as_str()) && docs.iter().any(|d| d.contains(tok.as_str()))
        })
        .count();

    let per_mille = u32::try_from(grounded * PER_MILLE as usize / needles.len()).unwrap_or(0);
    let detail = format!(
        "{grounded} of {} tokens grounded in answer+docs",
        needles.len()
    );
    ScoreOutput::gate("groundedness", per_mille, detail)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::suite::{Expectation, ExpectedTerminal};
    use crate::transcript::{Branch, Transcript, TurnRecord};

    fn rag_run(answer: &str, docs: &[&str]) -> Transcript {
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
            retrieved_docs: docs.iter().map(|d| (*d).to_string()).collect(),
            rerank: None,
            max_turns: 8,
            max_tool_calls: 20,
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
            ideal_turns: 2,
            ideal_tool_calls: 1,
        }
    }

    #[test]
    fn fully_grounded() {
        let t = rag_run(
            "Paris is the capital of France",
            &["The capital of France is Paris."],
        );
        let e = expect_grounded(&["Paris", "France"]);
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
    fn hallucinated_token_not_grounded() {
        // "Berlin" is in the answer but not in any doc ⇒ ungrounded.
        let t = rag_run("Paris and Berlin", &["The capital of France is Paris."]);
        let e = expect_grounded(&["Paris", "Berlin"]);
        assert_eq!(
            score(&ScoreInput {
                transcript: &t,
                expect: &e
            })
            .gate_per_mille(),
            Some(500)
        );
    }

    #[test]
    fn no_grounded_tokens_is_na() {
        let t = rag_run("anything", &[]);
        let e = expect_grounded(&[]);
        let s = score(&ScoreInput {
            transcript: &t,
            expect: &e,
        });
        assert!(!s.applicable);
        assert_eq!(s.gate_per_mille(), None);
    }
}
