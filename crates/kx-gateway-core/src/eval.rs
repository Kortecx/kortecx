//! RC1 (D172) — the per-run quality fold backing the `ScoreRun` RPC.
//!
//! Maps a run's off-DAG `ReactRound` trajectory (via the existing `list_react_turns`
//! read-fold) into a [`kx_eval::Transcript`], runs [`kx_eval::analyze_run`], and returns
//! the expectation-free [`proto::RunScore`]. This is the proto↔Transcript boundary, kept
//! HERE so `kx-eval` stays a proto-free pure leaf. Read-only, off-digest, operator-global
//! — the same posture as `ListReactTurns`. The `ScoreRun` readout needs only the
//! trajectory branches, so no committed content is fetched.

use kx_eval::{analyze_run, Branch, Transcript, TurnRecord};
use kx_journal::INSTANCE_ID_LEN;
use kx_proto::proto;

use crate::error::GatewayError;
use crate::reader::JournalReader;

/// Fold a run's ReactRound trajectory into an expectation-free [`proto::RunScore`].
///
/// # Errors
/// [`GatewayError::InvalidArgument`] if `instance_id` is not 16 bytes; propagates any
/// read error from the underlying `list_react_turns` fold.
pub(crate) fn score_run(
    reader: &dyn JournalReader,
    instance_id: &[u8],
) -> Result<proto::RunScore, GatewayError> {
    if instance_id.len() != INSTANCE_ID_LEN {
        return Err(GatewayError::InvalidArgument(
            "score_run instance_id must be 16 bytes",
        ));
    }
    // Reuse the existing read-fold (newest-first); re-order oldest-first for the
    // transcript so terminal_branch / turn counting are correct.
    let listing = crate::react::list_react_turns(reader, None, Some(instance_id), None)?;
    let mut rows = listing.turns;
    rows.sort_by(|a, b| a.seq.cmp(&b.seq).then(a.call_index.cmp(&b.call_index)));

    let (max_turns, max_tool_calls) = rows
        .first()
        .map_or((0, 0), |r| (r.max_turns, r.max_tool_calls));
    let turns = rows
        .iter()
        .map(|r| TurnRecord {
            turn: r.turn,
            branch: branch_from_wire(&r.branch),
            tool_id: r.tool_id.clone(),
            tool_version: r.tool_version.clone(),
            call_index: r.call_index,
            rejection_reason: r.rejection_reason.clone(),
        })
        .collect();

    let transcript = Transcript {
        task_id: hex(instance_id),
        turns,
        final_answer: None, // expectation-free: ScoreRun needs no answer content.
        retrieved_docs: Vec::new(),
        max_turns,
        max_tool_calls,
    };
    let q = analyze_run(&transcript);
    Ok(proto::RunScore {
        instance_id: instance_id.to_vec(),
        terminal: branch_to_wire(q.terminal).to_string(),
        reached_answer: q.reached_answer,
        turns_used: q.turns_used,
        tool_calls_used: q.tool_calls_used,
        max_turns: q.max_turns,
        max_tool_calls: q.max_tool_calls,
        rejections: q.rejections,
        turn_budget_used_per_mille: q.turn_budget_used_per_mille,
        tool_budget_used_per_mille: q.tool_budget_used_per_mille,
    })
}

/// The wire branch string emitted by `list_react_turns` → the eval [`Branch`].
fn branch_from_wire(s: &str) -> Branch {
    match s {
        "answer" => Branch::Answer,
        "tool" => Branch::Tool,
        "rejected" => Branch::Rejected,
        "dead_lettered" => Branch::DeadLettered,
        _ => Branch::Pending,
    }
}

/// The eval [`Branch`] → the wire terminal string (mirrors the ListReactTurns vocabulary).
fn branch_to_wire(b: Branch) -> &'static str {
    match b {
        Branch::Answer => "answer",
        Branch::Tool => "tool",
        Branch::Rejected => "rejected",
        Branch::DeadLettered => "dead_lettered",
        Branch::Pending => "pending",
    }
}

/// Lowercase hex of the run id (a display label for the transcript; unused by scoring).
fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

#[cfg(test)]
mod tests {
    use kx_journal::{InMemoryJournal, Journal, JournalEntry, ReactBranch};
    use kx_mote::MoteId;

    use crate::reader::ReadOnly;

    use super::*;

    fn react_fact(turn: u32, instance: u8, branch: ReactBranch) -> JournalEntry {
        JournalEntry::ReactRound {
            turn,
            turn_mote_id: MoteId::from_bytes([instance; 32]),
            instance_id: [instance; INSTANCE_ID_LEN],
            base_prompt_ref: kx_content::ContentRef::from_bytes([0u8; 32]),
            warrant_ref: kx_content::ContentRef::from_bytes([0u8; 32]),
            model_id: "m".to_string(),
            branch,
            max_turns: 8,
            max_tool_calls: 8,
            step_salt: None,
            is_agentic_launch: false,
            context_items_ref: None,
            image_ref: None,
            require_approval: false,
            seq: 0,
        }
    }

    #[test]
    fn score_run_folds_an_answer_run_with_a_rejection() {
        let j = InMemoryJournal::new();
        j.append(react_fact(
            0,
            0xa1,
            ReactBranch::Rejected {
                reason: "args did not match the tool schema".to_string(),
            },
        ))
        .unwrap();
        j.append(react_fact(1, 0xa1, ReactBranch::Answer)).unwrap();
        let r = ReadOnly::new(j);

        let score = score_run(&r, &[0xa1; INSTANCE_ID_LEN]).unwrap();
        assert!(score.reached_answer);
        assert_eq!(score.terminal, "answer");
        assert_eq!(score.rejections, 1);
        assert_eq!(score.max_turns, 8);
        assert_eq!(score.max_tool_calls, 8);
    }

    #[test]
    fn score_run_reports_dead_letter() {
        let j = InMemoryJournal::new();
        j.append(react_fact(0, 0xb2, ReactBranch::DeadLettered))
            .unwrap();
        let r = ReadOnly::new(j);
        let score = score_run(&r, &[0xb2; INSTANCE_ID_LEN]).unwrap();
        assert!(!score.reached_answer);
        assert_eq!(score.terminal, "dead_lettered");
    }

    #[test]
    fn score_run_refuses_a_bad_instance_id() {
        let r = ReadOnly::new(InMemoryJournal::new());
        let err = score_run(&r, &[1, 2, 3]).unwrap_err();
        assert!(matches!(err, GatewayError::InvalidArgument(_)));
    }
}
