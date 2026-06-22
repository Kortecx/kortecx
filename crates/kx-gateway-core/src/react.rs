//! `ListReactTurns` — enumerate the journal's durable `ReactRound` facts,
//! newest-first and paginated (PR-2d-1). A read-only filter over the **off-DAG**
//! ReAct-turn metadata the live chain commits — never a write, never identity,
//! never the projection digest. The operator-facing window into a live ReAct
//! chain: which turns ran, how each settled (answer / tool / dead-letter /
//! pending), and the durable budget the run was admitted under.
//!
//! SCOPE (single-node OSS): **operator-global** — like `ListReplanRounds`, the
//! OSS journal records no party on a `ReactRound`, so this lists every turn on
//! the node (one operator, behind the deny-all/bearer auth interceptor). An
//! optional `instance_id` filter scopes to one run's chain (serve's journal is
//! SHARED across runs — the same salt that keys the coordinator's settle).
//! CLOUD party-scopes this in the gateway-auth layer (D102.1); do NOT fold
//! party scoping in here.
//!
//! Pagination defends the O(journal) fold against an unbounded response: a page
//! is at most [`MAX_PAGE`] turns; turns are sparse (≤ `max_turns` facts per run
//! plus settles), so a single default page covers any realistic chain.

use kx_journal::{JournalEntry, ReactBranch, INSTANCE_ID_LEN};
use kx_proto::proto;

use crate::error::{internal, GatewayError};
use crate::reader::JournalReader;

/// The server cap on a `ListReactTurns` page. Bounds the response (and the
/// per-call allocation) regardless of journal size.
const MAX_PAGE: usize = 500;

/// The page size when the request omits `limit`.
const DEFAULT_PAGE: usize = 200;

/// The closed wire vocabulary for a settled branch (frozen at append; mirrored
/// in the proto doc-comment — a string, not an enum, so a future branch is
/// additive on the wire).
fn branch_wire(branch: &ReactBranch) -> (&'static str, String, String, String) {
    match branch {
        ReactBranch::Answer => ("answer", String::new(), String::new(), String::new()),
        ReactBranch::Tool {
            tool_id,
            tool_version,
        } => ("tool", tool_id.clone(), tool_version.clone(), String::new()),
        // PR-3 (A2): a refused proposal the model re-prompts over — carry the
        // durable reason for operator troubleshooting (display only).
        ReactBranch::Rejected { reason } => {
            ("rejected", String::new(), String::new(), reason.clone())
        }
        ReactBranch::DeadLettered => ("dead_lettered", String::new(), String::new(), String::new()),
        ReactBranch::Pending => ("pending", String::new(), String::new(), String::new()),
    }
}

/// Fold the journal's `ReactRound` facts and return one newest-first page of
/// turn summaries, optionally scoped to one run's `instance_id`. `limit` is
/// clamped to `[1, MAX_PAGE]` (or [`DEFAULT_PAGE`] when absent). A present-but-
/// malformed `instance_id` (wrong length) is refused loudly rather than
/// silently matching nothing.
///
/// # Errors
/// [`GatewayError::Internal`] on a journal read failure;
/// [`GatewayError::InvalidArgument`] on a malformed `instance_id` filter.
pub(crate) fn list_react_turns(
    reader: &dyn JournalReader,
    limit: Option<u32>,
    instance_filter: Option<&[u8]>,
    step_salt_filter: Option<&[u8]>,
) -> Result<proto::ListReactTurnsResponse, GatewayError> {
    let filter: Option<[u8; INSTANCE_ID_LEN]> = match instance_filter {
        None => None,
        Some(raw) => Some(<[u8; INSTANCE_ID_LEN]>::try_from(raw).map_err(|_| {
            GatewayError::InvalidArgument("react instance_id filter must be 16 bytes")
        })?),
    };
    // PR-R1: optional per-chain scope. Absent = every chain under the instance
    // filter (the pre-PR-R1 behaviour). Present: 0 bytes = the legacy run-level
    // (`None`-salt) chain; 32 bytes = exactly that chain (a per-invocation run-level
    // chain or an agentic step). `Option<Option<[u8;32]>>`: outer None = no filter.
    let chain_filter: Option<Option<[u8; 32]>> = match step_salt_filter {
        None => None,
        Some([]) => Some(None),
        Some(raw) => Some(Some(<[u8; 32]>::try_from(raw).map_err(|_| {
            GatewayError::InvalidArgument("react step_salt filter must be 0 or 32 bytes")
        })?)),
    };
    let head = reader.current_seq().map_err(internal)?;
    // Collect ReactRound facts in ascending journal order, then reverse for
    // newest-first (turns are budget-bounded per run, so this never
    // materializes a large vec).
    let mut all: Vec<proto::ReactTurnSummary> = reader
        .read_entries_by_seq(0..head.saturating_add(1))
        .map_err(internal)?
        .filter_map(|entry| match entry {
            JournalEntry::ReactRound {
                turn,
                turn_mote_id,
                instance_id,
                model_id,
                branch,
                max_turns,
                max_tool_calls,
                step_salt,
                seq,
                ..
            } if filter.is_none_or(|f| f == instance_id)
                && chain_filter.is_none_or(|cf| cf == step_salt) =>
            {
                let (branch_str, tool_id, tool_version, rejection_reason) = branch_wire(&branch);
                Some(proto::ReactTurnSummary {
                    turn,
                    turn_mote_id: turn_mote_id.as_bytes().to_vec(),
                    instance_id: instance_id.to_vec(),
                    model_id,
                    branch: branch_str.to_string(),
                    tool_id,
                    tool_version,
                    max_turns,
                    max_tool_calls,
                    seq,
                    rejection_reason,
                    // PR-R1: the chain key (empty for a legacy run-level `None` chain).
                    step_salt: step_salt.map(|s| s.to_vec()).unwrap_or_default(),
                })
            }
            _ => None,
        })
        .collect();
    all.reverse(); // newest-first (descending seq)

    let page = limit.map_or(DEFAULT_PAGE, |l| (l as usize).clamp(1, MAX_PAGE));
    let has_more = all.len() > page;
    all.truncate(page);
    Ok(proto::ListReactTurnsResponse {
        turns: all,
        has_more,
    })
}

#[cfg(test)]
mod tests {
    use kx_journal::{InMemoryJournal, Journal, JournalEntry};
    use kx_mote::MoteId;

    use crate::reader::ReadOnly;

    use super::*;

    fn turn_fact(turn: u32, instance: u8, branch: ReactBranch) -> JournalEntry {
        turn_fact_chain(turn, instance, None, branch)
    }

    #[allow(clippy::cast_possible_truncation)] // test turns are tiny
    fn turn_fact_chain(
        turn: u32,
        instance: u8,
        step_salt: Option<[u8; 32]>,
        branch: ReactBranch,
    ) -> JournalEntry {
        JournalEntry::ReactRound {
            turn,
            turn_mote_id: MoteId::from_bytes([instance.wrapping_add(turn as u8); 32]),
            instance_id: [instance; INSTANCE_ID_LEN],
            base_prompt_ref: kx_content::ContentRef::from_bytes([0u8; 32]),
            warrant_ref: kx_content::ContentRef::from_bytes([0u8; 32]),
            model_id: "m".to_string(),
            branch,
            max_turns: 8,
            max_tool_calls: 8,
            step_salt,
            is_agentic_launch: false,
            seq: 0,
        }
    }

    #[test]
    fn rejected_branch_surfaces_its_reason_on_the_wire() {
        // PR-3 (A2): a Rejected turn is a distinct wire branch carrying the
        // durable reason for operator troubleshooting; other branches carry "".
        let j = InMemoryJournal::new();
        j.append(turn_fact(
            0,
            0xb0,
            ReactBranch::Rejected {
                reason: "the arguments for `mcp-echo/echo@1` do not match its inputSchema"
                    .to_string(),
            },
        ))
        .unwrap();
        j.append(turn_fact(1, 0xb0, ReactBranch::Answer)).unwrap();
        let r = ReadOnly::new(j);

        let resp = list_react_turns(&r, None, None, None).unwrap();
        let rejected = resp
            .turns
            .iter()
            .find(|t| t.branch == "rejected")
            .expect("a rejected turn");
        assert!(
            rejected.rejection_reason.contains("inputSchema"),
            "the rejection reason surfaces on the wire"
        );
        assert!(rejected.tool_id.is_empty(), "no tool id on a rejected turn");
        // A non-rejected branch carries an empty reason (forward-compat default).
        let answer = resp.turns.iter().find(|t| t.branch == "answer").unwrap();
        assert!(answer.rejection_reason.is_empty());
    }

    #[test]
    fn empty_journal_lists_nothing() {
        let r = ReadOnly::new(InMemoryJournal::new());
        let resp = list_react_turns(&r, None, None, None).unwrap();
        assert!(resp.turns.is_empty());
        assert!(!resp.has_more);
    }

    #[test]
    fn lists_newest_first_with_branch_vocabulary() {
        let j = InMemoryJournal::new();
        j.append(turn_fact(0, 0xa0, ReactBranch::Pending)).unwrap();
        j.append(turn_fact(
            0,
            0xa0,
            ReactBranch::Tool {
                tool_id: "mcp-echo".to_string(),
                tool_version: "1".to_string(),
            },
        ))
        .unwrap();
        j.append(turn_fact(1, 0xa0, ReactBranch::Answer)).unwrap();
        let r = ReadOnly::new(j);

        let resp = list_react_turns(&r, None, None, None).unwrap();
        assert_eq!(resp.turns.len(), 3);
        assert!(!resp.has_more);
        // Newest-first: the turn-1 Answer settle (highest seq) leads.
        assert_eq!(resp.turns[0].turn, 1);
        assert_eq!(resp.turns[0].branch, "answer");
        assert!(resp.turns[0].tool_id.is_empty());
        // The Tool settle carries its (granted) tool identity.
        assert_eq!(resp.turns[1].branch, "tool");
        assert_eq!(resp.turns[1].tool_id, "mcp-echo");
        assert_eq!(resp.turns[1].tool_version, "1");
        // The anchor is last, Pending, with the durable caps.
        assert_eq!(resp.turns[2].branch, "pending");
        assert_eq!(resp.turns[2].max_turns, 8);
        // Strictly descending seq.
        assert!(resp.turns[0].seq > resp.turns[1].seq);
        assert!(resp.turns[1].seq > resp.turns[2].seq);
    }

    #[test]
    fn instance_filter_scopes_to_one_run() {
        let j = InMemoryJournal::new();
        j.append(turn_fact(0, 0xa1, ReactBranch::Pending)).unwrap();
        j.append(turn_fact(0, 0xa2, ReactBranch::Pending)).unwrap();
        j.append(turn_fact(1, 0xa1, ReactBranch::Answer)).unwrap();
        let r = ReadOnly::new(j);

        let resp = list_react_turns(&r, None, Some(&[0xa1; INSTANCE_ID_LEN]), None).unwrap();
        assert_eq!(resp.turns.len(), 2, "only run 0xa1's facts");
        assert!(resp
            .turns
            .iter()
            .all(|t| t.instance_id == vec![0xa1; INSTANCE_ID_LEN]));
    }

    #[test]
    fn step_salt_filter_scopes_to_one_chain() {
        // PR-R1: serve's shared journal can carry many chains under one instance_id
        // (one per Invoke). The step_salt filter isolates a single chain.
        let salt_a = [0x11u8; 32];
        let salt_b = [0x22u8; 32];
        let j = InMemoryJournal::new();
        j.append(turn_fact_chain(0, 0xd0, Some(salt_a), ReactBranch::Pending))
            .unwrap();
        j.append(turn_fact_chain(0, 0xd0, Some(salt_b), ReactBranch::Pending))
            .unwrap();
        j.append(turn_fact_chain(1, 0xd0, Some(salt_a), ReactBranch::Answer))
            .unwrap();
        // A legacy run-level chain (None salt) under the same run.
        j.append(turn_fact_chain(0, 0xd0, None, ReactBranch::Pending))
            .unwrap();
        let r = ReadOnly::new(j);

        // Scope to chain A: its anchor + answer only.
        let resp =
            list_react_turns(&r, None, Some(&[0xd0; INSTANCE_ID_LEN]), Some(&salt_a)).unwrap();
        assert_eq!(resp.turns.len(), 2, "only chain A's two facts");
        assert!(resp.turns.iter().all(|t| t.step_salt == salt_a.to_vec()));
        // Empty step_salt scopes to the legacy run-level (None) chain only.
        let run_level =
            list_react_turns(&r, None, Some(&[0xd0; INSTANCE_ID_LEN]), Some(&[])).unwrap();
        assert_eq!(run_level.turns.len(), 1, "only the None-salt chain");
        assert!(run_level.turns[0].step_salt.is_empty());
        // Absent step_salt = every chain under the run (4 facts).
        let all = list_react_turns(&r, None, Some(&[0xd0; INSTANCE_ID_LEN]), None).unwrap();
        assert_eq!(all.turns.len(), 4);
    }

    #[test]
    fn malformed_instance_filter_is_refused_loudly() {
        let r = ReadOnly::new(InMemoryJournal::new());
        let err = list_react_turns(&r, None, Some(&[1, 2, 3]), None).unwrap_err();
        assert!(matches!(err, GatewayError::InvalidArgument(_)));
        // PR-R1: a non-empty, non-32-byte step_salt filter is refused loudly too.
        let r2 = ReadOnly::new(InMemoryJournal::new());
        let err2 = list_react_turns(&r2, None, None, Some(&[9, 9, 9])).unwrap_err();
        assert!(matches!(err2, GatewayError::InvalidArgument(_)));
    }

    #[test]
    fn limit_clamps_and_signals_has_more() {
        let j = InMemoryJournal::new();
        for i in 0..5u32 {
            j.append(turn_fact(i, 0xb0, ReactBranch::Pending)).unwrap();
        }
        let r = ReadOnly::new(j);
        let resp = list_react_turns(&r, Some(2), None, None).unwrap();
        assert_eq!(resp.turns.len(), 2);
        assert!(resp.has_more, "3 turns remain beyond a page of 2");
    }

    #[test]
    fn non_react_facts_are_ignored() {
        let j = InMemoryJournal::new();
        j.append(JournalEntry::RunRegistered {
            instance_id: [1u8; INSTANCE_ID_LEN],
            recipe_fingerprint: [2u8; 32],
            ts: 0,
            seq: 0,
        })
        .unwrap();
        j.append(turn_fact(0, 0xc0, ReactBranch::Pending)).unwrap();
        let r = ReadOnly::new(j);
        let resp = list_react_turns(&r, None, None, None).unwrap();
        assert_eq!(resp.turns.len(), 1, "only ReactRound facts are enumerated");
        assert_eq!(resp.turns[0].turn, 0);
    }
}
