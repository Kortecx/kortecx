//! The golden-suite model — the versioned, on-disk corpus of agentic tasks and the
//! [`Expectation`] each is scored against.
//!
//! A [`GoldenTask`] carries both an `instruction` (what the Tier-B live lane sends to a
//! real model) and a scripted [`crate::Transcript`] (the Tier-A deterministic fixture).
//! The same [`Expectation`] scores both tiers, so the deterministic gate and the
//! real-model trend measure the *same* contract.

use serde::{Deserialize, Serialize};

use crate::transcript::{ToolKey, Transcript};

/// The terminal state a task is expected to reach.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpectedTerminal {
    /// The run should end with a prose answer.
    Answer,
    /// The run should cleanly dead-letter (e.g. a deliberate budget-exhaustion task) —
    /// the loud terminal, not a hang.
    DeadLetter,
}

/// One expected tool call, by exact `(id, version)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpectedToolCall {
    /// The expected tool id (`<server>/<remote>`).
    pub tool_id: String,
    /// The expected pinned version.
    pub tool_version: String,
}

impl ExpectedToolCall {
    /// The `(id, version)` key for comparison against an actual call.
    #[must_use]
    pub fn key(&self) -> ToolKey {
        ToolKey {
            id: self.tool_id.clone(),
            version: self.tool_version.clone(),
        }
    }
}

/// What a task's run must achieve, against which the scorers grade.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Expectation {
    /// The terminal the run must reach.
    pub terminal: ExpectedTerminal,
    /// Substrings the final answer must contain (the answer oracle). Empty for a
    /// dead-letter task.
    #[serde(default)]
    pub answer_must_contain: Vec<String>,
    /// The tool calls the run is expected to make, as a multiset (order-tolerant). Empty
    /// for an answer-only task.
    #[serde(default)]
    pub expected_tools: Vec<ExpectedToolCall>,
    /// Tokens the answer must be GROUNDED in — each must appear both in the answer and
    /// in at least one retrieved doc. Empty ⇒ groundedness is N/A for this task.
    #[serde(default)]
    pub grounded_in: Vec<String>,
    /// RC4c-2c: the pre-rerank BASE index of the most-relevant candidate (e.g. an on-topic
    /// passage placed LAST). The rerank scorer checks it landed in the top-`rerank_top_k`
    /// after the reorder. `None` ⇒ the rerank scorer is N/A for this task.
    #[serde(default)]
    pub rerank_best_index: Option<u32>,
    /// The rank window the best candidate must land within (0/omitted ⇒ 1, must be first).
    #[serde(default)]
    pub rerank_top_k: u32,
    /// RC5a: facts the run must RECALL from durable memory (each must appear both in the
    /// recalled memories — carried as `retrieved_docs` — AND in the final answer, i.e.
    /// recalled AND grounded). Empty ⇒ the memory_quality scorer is N/A for this task. The
    /// fail-closed guard: a recall that silently returns nothing scores 0.
    #[serde(default)]
    pub memory_must_recall: Vec<String>,
    /// RC5b: facts a CONSOLIDATION must distill into ONE recalled semantic entry AND
    /// ground in the answer. Distinct from `memory_must_recall` (which checks each fact
    /// in ANY recalled doc): consolidation requires all facts collapsed into a single
    /// entry. Empty ⇒ the consolidation_quality scorer is N/A. The fail-closed guard: a
    /// consolidation that produced/recalled nothing scores 0.
    #[serde(default)]
    pub consolidation_must_capture: Vec<String>,
    /// The SKILL's tool WISH set for a skill-bearing task. The skill_quality
    /// gate: every Tool turn must stay WITHIN this set (an out-of-wish call is a
    /// fold/warrant boundary leak) and the run must actually fire a tool + answer
    /// (fail-closed: a wished skill whose run never touched a tool scores 0). Empty
    /// ⇒ the skill_quality scorer is N/A for this task.
    #[serde(default)]
    pub skill_wish_tools: Vec<ExpectedToolCall>,
    /// The ideal number of turns to solve the task (the loop-efficiency denominator).
    pub ideal_turns: u32,
    /// The ideal number of tool calls to solve the task.
    pub ideal_tool_calls: u32,
}

/// One golden task: an instruction (Tier-B), a scripted transcript (Tier-A), and the
/// [`Expectation`] both are scored against.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoldenTask {
    /// The task id (stable; labels every metric).
    pub id: String,
    /// The capability FAMILY this task exercises (`core` when omitted) — the
    /// `kx-eval --suite <family>` selector (per-family iteration; the
    /// committed baseline stays the aggregate gate).
    #[serde(default = "default_family")]
    pub family: String,
    /// A one-line description of what the task exercises.
    pub description: String,
    /// The instruction the Tier-B live lane sends to a real model.
    pub instruction: String,
    /// What the run must achieve.
    pub expect: Expectation,
    /// The deterministic Tier-A fixture (a scripted run that meets the expectation).
    pub scripted_transcript: Transcript,
}

fn default_family() -> String {
    "core".to_string()
}

/// A named, versioned set of golden tasks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoldenSuite {
    /// The suite id (e.g. `"golden-v1"`).
    pub id: String,
    /// The tasks, in a stable order.
    pub tasks: Vec<GoldenTask>,
}
