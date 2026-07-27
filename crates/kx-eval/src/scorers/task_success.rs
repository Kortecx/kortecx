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
                let degrouped = degroup_digits(&answer);
                let missing: Vec<&str> = input
                    .expect
                    .answer_must_contain
                    .iter()
                    .filter(|needle| !contains_oracle(&answer, &degrouped, needle))
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

/// Whether `needle` appears in the answer, tolerating THOUSANDS SEPARATORS for a
/// purely-numeric oracle.
///
/// A numeric oracle that fails on `1,000` is measuring number formatting, not the
/// capability under test — observed live: a run computed the right value and wrote it as
/// "a final number of 1,000", and the `"1000"` substring missed. Every numeric task in the
/// suite is one comma away from the same false negative, and a false NEGATIVE on a
/// capability benchmark is as misleading as a false positive.
///
/// Deliberately narrow: the relaxation applies only when the needle is ALL DIGITS, and only
/// separators *between digits* are removed, so it cannot merge two distinct numbers or
/// rescue a non-numeric answer. Plain substring semantics are otherwise unchanged, prefix
/// collisions included (`"50"` still matches `"350"` — pick oracle values accordingly).
fn contains_oracle(answer: &str, degrouped: &str, needle: &str) -> bool {
    if answer.contains(needle) {
        return true;
    }
    !needle.is_empty()
        && needle.chars().all(|c| c.is_ascii_digit())
        && degrouped.contains(needle)
}

/// `answer` with `,` / `_` / thin spaces dropped when they sit BETWEEN two ASCII digits —
/// i.e. exactly where a thousands separator can appear. A separator anywhere else (a list,
/// prose punctuation) is preserved, so this cannot glue unrelated numbers together.
fn degroup_digits(answer: &str) -> String {
    let chars: Vec<char> = answer.chars().collect();
    let mut out = String::with_capacity(answer.len());
    for (i, &c) in chars.iter().enumerate() {
        let is_sep = matches!(c, ',' | '_' | '\u{202f}' | '\u{2009}');
        let between_digits = i > 0
            && chars[i - 1].is_ascii_digit()
            && chars.get(i + 1).is_some_and(char::is_ascii_digit);
        if is_sep && between_digits {
            continue;
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::suite::Expectation;
    use crate::transcript::{Transcript, TurnRecord};

    /// A numeric oracle must survive standard number formatting — and must NOT start
    /// matching things it should not.
    #[test]
    fn a_numeric_oracle_tolerates_thousands_separators_only() {
        let hit = |answer: &str, needle: &str| {
            contains_oracle(answer, &degroup_digits(answer), needle)
        };
        // The live failure this exists for.
        assert!(hit("a final number of 1,000.", "1000"));
        assert!(hit("total 5_422_054 units", "5422054"));
        assert!(hit("plain 48246 here", "48246"));
        // A separator that is NOT between digits is preserved, so distinct numbers in a
        // list can never be glued into a spurious match.
        assert!(!hit("the parts are 10, 00 and 7", "1000"));
        // The relaxation is numeric-only: a non-digit oracle keeps exact substring
        // semantics, so a token like QUILL-MERIDIAN-58 cannot be matched loosely.
        assert!(!hit("ZEPHYR,77 was seen", "ZEPHYR77"));
        // And it never rescues an answer that simply lacks the value.
        assert!(!hit("I could not determine the total.", "5422054"));
    }

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
            memory_must_recall: vec![],
            consolidation_must_capture: vec![],
            skill_wish_tools: vec![],
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
            memory_must_recall: vec![],
            consolidation_must_capture: vec![],
            skill_wish_tools: vec![],
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
