//! RAG rerank-quality scorer (RC4c-2c) — did the listwise rerank ACTUALLY improve the
//! ranking?
//!
//! The task declares the pre-rerank BASE index of the most-relevant candidate
//! (`rerank_best_index`, e.g. an on-topic passage placed LAST); the scorer checks the
//! committed permutation moved it into the top-`rerank_top_k`. A `failed_closed` (or
//! absent) rerank scores **0** — the point of this gate is to assert the rerank fired AND
//! reordered correctly. Without it the `T-RERANK-WORKER-ROUTE` fail-closed class
//! re-breaks silently (it passed every unit test + 22 CI jobs precisely because it fails
//! closed). Tasks that declare no `rerank_best_index` are N/A and excluded from the
//! aggregate. Deterministic, LLM-free, integer per-mille (SN-8).

use crate::scorers::{ScoreOutput, PER_MILLE};

use super::ScoreInput;

pub(super) fn score(input: &ScoreInput) -> ScoreOutput {
    let Some(best_base) = input.expect.rerank_best_index else {
        return ScoreOutput::not_applicable("rerank_quality", "task declares no rerank_best_index");
    };
    // `rerank_top_k` defaults to 0 when omitted in JSON; treat that as "must be first".
    let k = input.expect.rerank_top_k.max(1);

    let Some(rr) = &input.transcript.rerank else {
        return ScoreOutput::gate("rerank_quality", 0, "no rerank round on the run");
    };
    // A rerank that never settled a valid permutation is a quality failure — this is the
    // fail-closed regression guard (base order is not "reranked").
    if rr.outcome != "reranked" {
        return ScoreOutput::gate(
            "rerank_quality",
            0,
            format!("rerank did not settle reranked (outcome={})", rr.outcome),
        );
    }
    // The permutation's index is the NEW rank; the value is the source (base) index. The
    // best candidate's new rank is the position that now holds `best_base`.
    match rr.permutation.iter().position(|&src| src == best_base) {
        Some(rank) if u32::try_from(rank).unwrap_or(u32::MAX) < k => ScoreOutput::gate(
            "rerank_quality",
            PER_MILLE,
            format!("best base #{best_base} reranked to rank {rank} (< top-{k})"),
        ),
        Some(rank) => ScoreOutput::gate(
            "rerank_quality",
            0,
            format!("best base #{best_base} at rank {rank} (>= top-{k})"),
        ),
        None => ScoreOutput::gate(
            "rerank_quality",
            0,
            format!("best base #{best_base} absent from the permutation"),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::suite::{Expectation, ExpectedTerminal};
    use crate::transcript::{Branch, RerankInfo, Transcript, TurnRecord};

    fn rerank_run(outcome: &str, permutation: &[u32]) -> Transcript {
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
            final_answer: Some("ok".into()),
            retrieved_docs: vec![],
            rerank: Some(RerankInfo {
                candidate_count: u32::try_from(permutation.len()).unwrap_or(0),
                permutation: permutation.to_vec(),
                outcome: outcome.into(),
            }),
            max_turns: 8,
            max_tool_calls: 20,
        }
    }

    fn expect_best(best_index: Option<u32>, top_k: u32) -> Expectation {
        Expectation {
            terminal: ExpectedTerminal::Answer,
            answer_must_contain: vec![],
            expected_tools: vec![],
            grounded_in: vec![],
            rerank_best_index: best_index,
            rerank_top_k: top_k,
            memory_must_recall: vec![],
            consolidation_must_capture: vec![],
            ideal_turns: 2,
            ideal_tool_calls: 1,
        }
    }

    fn run(t: &Transcript, e: &Expectation) -> ScoreOutput {
        score(&ScoreInput {
            transcript: t,
            expect: e,
        })
    }

    #[test]
    fn on_topic_last_reranked_to_top_scores_full() {
        // 4 candidates, the best is at BASE index 3 (last); the rerank moves it to rank 0.
        let t = rerank_run("reranked", &[3, 0, 1, 2]);
        let e = expect_best(Some(3), 1);
        assert_eq!(run(&t, &e).gate_per_mille(), Some(PER_MILLE));
    }

    #[test]
    fn best_outside_top_k_scores_zero() {
        // best base #3 lands at new rank 3 — not in the top-1.
        let t = rerank_run("reranked", &[0, 1, 2, 3]);
        let e = expect_best(Some(3), 1);
        assert_eq!(run(&t, &e).gate_per_mille(), Some(0));
    }

    #[test]
    fn top_k_window_is_honored() {
        // best base #3 at new rank 1 — inside the top-2 window.
        let t = rerank_run("reranked", &[0, 3, 1, 2]);
        let e = expect_best(Some(3), 2);
        assert_eq!(run(&t, &e).gate_per_mille(), Some(PER_MILLE));
    }

    #[test]
    fn failed_closed_scores_zero() {
        // THE regression guard: a rerank that fell back to base order is a quality FAIL.
        let t = rerank_run("failed_closed", &[]);
        let e = expect_best(Some(3), 1);
        assert_eq!(run(&t, &e).gate_per_mille(), Some(0));
    }

    #[test]
    fn absent_rerank_scores_zero() {
        let mut t = rerank_run("reranked", &[3, 0, 1, 2]);
        t.rerank = None;
        let e = expect_best(Some(3), 1);
        assert_eq!(run(&t, &e).gate_per_mille(), Some(0));
    }

    #[test]
    fn no_best_index_is_na() {
        let t = rerank_run("reranked", &[3, 0, 1, 2]);
        let e = expect_best(None, 1);
        let s = run(&t, &e);
        assert!(!s.applicable);
        assert_eq!(s.gate_per_mille(), None);
    }

    #[test]
    fn top_k_zero_defaults_to_first() {
        // an omitted rerank_top_k (0) means "must be first".
        let t = rerank_run("reranked", &[0, 3, 1, 2]);
        let e = expect_best(Some(3), 0);
        assert_eq!(run(&t, &e).gate_per_mille(), Some(0));
    }
}
