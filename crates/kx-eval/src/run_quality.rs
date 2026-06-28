//! Expectation-free per-run quality analysis.
//!
//! The golden-suite scorers grade a run against a known [`crate::Expectation`] (the
//! `just eval` gate). A *live* run submitted by a user has no oracle, so the per-run
//! readout the `ScoreRun` RPC + the UI Monitoring "Quality" lens surface is built only
//! from signals that need no expectation: did it reach an answer, how many turns /
//! tool-calls it spent, how much of its budget it burned, and how many proposals were
//! rejected. Pure + total over a [`Transcript`].

use serde::{Deserialize, Serialize};

use crate::scorers::PER_MILLE;
use crate::transcript::{Branch, Transcript};

/// An expectation-free quality summary of one run's trajectory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunQuality {
    /// The run's terminal branch.
    pub terminal: Branch,
    /// Whether the run reached a prose answer (the success terminal).
    pub reached_answer: bool,
    /// The number of turns the run used (ToolBatch-aware).
    pub turns_used: u32,
    /// The number of tool calls the run made.
    pub tool_calls_used: u32,
    /// The run's admitted turn cap.
    pub max_turns: u32,
    /// The run's admitted tool-call cap.
    pub max_tool_calls: u32,
    /// The number of rejected (re-prompted) proposals — a friction signal.
    pub rejections: u32,
    /// Turn-budget utilisation in per-mille (`turns_used / max_turns`).
    pub turn_budget_used_per_mille: u32,
    /// Tool-budget utilisation in per-mille (`tool_calls_used / max_tool_calls`).
    pub tool_budget_used_per_mille: u32,
}

/// Summarise a run's trajectory without any expectation.
#[must_use]
pub fn analyze_run(transcript: &Transcript) -> RunQuality {
    let terminal = transcript.terminal_branch();
    let turns_used = transcript.turns_used();
    let tool_calls_used = transcript.tool_calls_used();
    let rejections = u32::try_from(
        transcript
            .turns
            .iter()
            .filter(|t| t.branch == Branch::Rejected)
            .count(),
    )
    .unwrap_or(u32::MAX);
    RunQuality {
        terminal,
        reached_answer: terminal == Branch::Answer,
        turns_used,
        tool_calls_used,
        max_turns: transcript.max_turns,
        max_tool_calls: transcript.max_tool_calls,
        rejections,
        turn_budget_used_per_mille: utilisation(turns_used, transcript.max_turns),
        tool_budget_used_per_mille: utilisation(tool_calls_used, transcript.max_tool_calls),
    }
}

/// `used / max` in per-mille, capped at 1000; `0` when `max == 0` (integer-only).
fn utilisation(used: u32, max: u32) -> u32 {
    if max == 0 {
        return 0;
    }
    let v = (u64::from(used) * u64::from(PER_MILLE) / u64::from(max)).min(u64::from(PER_MILLE));
    u32::try_from(v).unwrap_or(PER_MILLE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcript::TurnRecord;

    fn turn(turn: u32, branch: Branch) -> TurnRecord {
        TurnRecord {
            turn,
            branch,
            tool_id: if branch == Branch::Tool {
                "kv/get".into()
            } else {
                String::new()
            },
            tool_version: if branch == Branch::Tool {
                "1".into()
            } else {
                String::new()
            },
            call_index: 0,
            rejection_reason: String::new(),
        }
    }

    #[test]
    fn answer_run_summary() {
        let t = Transcript {
            task_id: "t".into(),
            turns: vec![turn(0, Branch::Tool), turn(1, Branch::Answer)],
            final_answer: Some("ok".into()),
            retrieved_docs: vec![],
            max_turns: 8,
            max_tool_calls: 20,
        };
        let q = analyze_run(&t);
        assert!(q.reached_answer);
        assert_eq!(q.turns_used, 2);
        assert_eq!(q.tool_calls_used, 1);
        assert_eq!(q.rejections, 0);
        assert_eq!(q.turn_budget_used_per_mille, 250); // 2/8
        assert_eq!(q.tool_budget_used_per_mille, 50); // 1/20
    }

    #[test]
    fn dead_letter_with_rejections() {
        let t = Transcript {
            task_id: "t".into(),
            turns: vec![
                turn(0, Branch::Rejected),
                turn(1, Branch::Rejected),
                turn(2, Branch::DeadLettered),
            ],
            final_answer: None,
            retrieved_docs: vec![],
            max_turns: 8,
            max_tool_calls: 20,
        };
        let q = analyze_run(&t);
        assert!(!q.reached_answer);
        assert_eq!(q.terminal, Branch::DeadLettered);
        assert_eq!(q.rejections, 2);
        assert_eq!(q.tool_calls_used, 0);
    }
}
