//! The [`Transcript`] — the single, engine- and model-agnostic substrate every scorer
//! reads. It is the projection of one run's `ReactRound` facts plus its committed
//! outputs, reduced to exactly what scoring needs.
//!
//! A Tier-A fixture deserializes a `Transcript` straight from the corpus; a Tier-B run
//! has the caller build one from the gateway's `ListReactTurns` rows + the committed
//! answer/retrieval content (the proto mapping lives in the gateway handler, so this
//! crate stays a `kx-proto`-free pure leaf).

use serde::{Deserialize, Serialize};

/// The outcome of one ReAct turn — a faithful mirror of the runtime's `ReactBranch`
/// (`kx-coordinator`), reduced to the variants scoring distinguishes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Branch {
    /// The turn is still in flight (no terminal verdict yet).
    Pending,
    /// The turn produced a prose answer — the success terminal.
    Answer,
    /// The turn proposed a granted tool call (one row per call; a ToolBatch fans into
    /// N `Tool` rows by `call_index`).
    Tool,
    /// The turn's proposal was refused (ungranted/malformed/ambiguous) and the loop
    /// re-prompted with the durable reason. Counts against the budget.
    Rejected,
    /// The chain terminated without an answer (dispatch failure or budget exhaustion).
    DeadLettered,
}

/// One turn (or one call within a ToolBatch turn) of a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnRecord {
    /// The 0-indexed turn number.
    pub turn: u32,
    /// The turn's outcome.
    pub branch: Branch,
    /// The proposed tool's id (`<server>/<remote>` form), or empty when not a tool turn.
    #[serde(default)]
    pub tool_id: String,
    /// The proposed tool's pinned version, or empty.
    #[serde(default)]
    pub tool_version: String,
    /// The call index within a ToolBatch turn (0 for single-call / non-tool turns).
    #[serde(default)]
    pub call_index: u32,
    /// The durable refusal reason when `branch == Rejected` (else empty).
    #[serde(default)]
    pub rejection_reason: String,
}

/// A `(tool_id, tool_version)` identity used to compare an actual call against an
/// expected one. Exact equality only (SN-8 posture — scoring never fuzzy-matches a
/// tool identity).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ToolKey {
    /// The tool id.
    pub id: String,
    /// The tool version.
    pub version: String,
}

/// The reduced record of one agent run — the input to every scorer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transcript {
    /// The task this run answered (for labelling).
    pub task_id: String,
    /// The run's turns, oldest → newest.
    pub turns: Vec<TurnRecord>,
    /// The committed final answer text, present iff a turn reached [`Branch::Answer`].
    /// A live (Tier-B) transcript lossy-decodes the committed answer bytes; a scripted
    /// (Tier-A) fixture authors it directly.
    #[serde(default)]
    pub final_answer: Option<String>,
    /// The ordered retrieved RAG docs (committed content; scores excluded, SN-8). Empty
    /// for a non-RAG run.
    #[serde(default)]
    pub retrieved_docs: Vec<String>,
    /// The run's admitted turn cap (durable at anchor).
    pub max_turns: u32,
    /// The run's admitted tool-call cap (durable at anchor).
    pub max_tool_calls: u32,
}

impl Transcript {
    /// The terminal branch of the run — the last turn's branch, or [`Branch::Pending`]
    /// for an empty transcript.
    #[must_use]
    pub fn terminal_branch(&self) -> Branch {
        self.turns.last().map_or(Branch::Pending, |t| t.branch)
    }

    /// The ordered tool calls the run actually made (each [`Branch::Tool`] row, in
    /// transcript order, including every ToolBatch element).
    #[must_use]
    pub fn actual_tool_calls(&self) -> Vec<ToolKey> {
        self.turns
            .iter()
            .filter(|t| t.branch == Branch::Tool)
            .map(|t| ToolKey {
                id: t.tool_id.clone(),
                version: t.tool_version.clone(),
            })
            .collect()
    }

    /// The number of distinct turns (the `turn` field's cardinality) — a ToolBatch
    /// turn with N calls counts as ONE turn against the turn budget.
    #[must_use]
    pub fn turns_used(&self) -> u32 {
        let mut last: Option<u32> = None;
        let mut count = 0u32;
        for t in &self.turns {
            if last != Some(t.turn) {
                count = count.saturating_add(1);
                last = Some(t.turn);
            }
        }
        count
    }

    /// The number of tool calls the run made (each [`Branch::Tool`] row, ToolBatch-aware).
    #[must_use]
    pub fn tool_calls_used(&self) -> u32 {
        u32::try_from(
            self.turns
                .iter()
                .filter(|t| t.branch == Branch::Tool)
                .count(),
        )
        .unwrap_or(u32::MAX)
    }

    /// The final answer text, or `None` when the run produced no answer.
    #[must_use]
    pub fn answer_text(&self) -> Option<String> {
        self.final_answer.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(turn: u32, branch: Branch, tool: &str) -> TurnRecord {
        TurnRecord {
            turn,
            branch,
            tool_id: tool.into(),
            tool_version: if tool.is_empty() {
                String::new()
            } else {
                "1".into()
            },
            call_index: 0,
            rejection_reason: String::new(),
        }
    }

    #[test]
    fn terminal_branch_is_last() {
        let t = Transcript {
            task_id: "t".into(),
            turns: vec![
                turn(0, Branch::Tool, "mcp-echo/echo"),
                turn(1, Branch::Answer, ""),
            ],
            final_answer: Some("hi".to_string()),
            retrieved_docs: vec![],
            max_turns: 8,
            max_tool_calls: 20,
        };
        assert_eq!(t.terminal_branch(), Branch::Answer);
        assert_eq!(t.turns_used(), 2);
        assert_eq!(t.tool_calls_used(), 1);
        assert_eq!(t.answer_text().as_deref(), Some("hi"));
    }

    #[test]
    fn toolbatch_is_one_turn_many_calls() {
        let mut a = turn(0, Branch::Tool, "kv/get");
        let mut b = turn(0, Branch::Tool, "calc/add");
        a.call_index = 0;
        b.call_index = 1;
        let t = Transcript {
            task_id: "t".into(),
            turns: vec![a, b, turn(1, Branch::Answer, "")],
            final_answer: Some("ok".to_string()),
            retrieved_docs: vec![],
            max_turns: 8,
            max_tool_calls: 20,
        };
        assert_eq!(t.turns_used(), 2); // turn 0 (batch) + turn 1 (answer)
        assert_eq!(t.tool_calls_used(), 2); // two calls in the batch
        assert_eq!(t.actual_tool_calls().len(), 2);
    }

    #[test]
    fn empty_transcript_is_pending() {
        let t = Transcript {
            task_id: "t".into(),
            turns: vec![],
            final_answer: None,
            retrieved_docs: vec![],
            max_turns: 8,
            max_tool_calls: 20,
        };
        assert_eq!(t.terminal_branch(), Branch::Pending);
        assert_eq!(t.turns_used(), 0);
    }
}
