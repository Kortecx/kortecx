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

/// One wire row of a settled branch: `(branch_str, tool_id, tool_version, reason,
/// call_index)`. The branch vocabulary is frozen at append + mirrored in the proto
/// doc-comment — a string, not an enum, so a future branch is additive on the wire.
struct BranchRow {
    branch: &'static str,
    tool_id: String,
    tool_version: String,
    reason: String,
    call_index: u32,
}

/// The wire ROWS for a settled branch. Every branch is ONE row EXCEPT a `ToolBatch`,
/// which FANS into N `"tool"` rows (one per call, call_index 0..N-1) so a client sees
/// the full multi-tool trajectory (T-MULTI-ELEMENT-TOOLCALLS). A single-call `Tool`
/// (and every non-tool branch) is one row with call_index 0 — byte-compatible with a
/// pre-v13 server (the new field defaults to 0).
fn branch_wire(branch: &ReactBranch) -> Vec<BranchRow> {
    let one = |branch, tool_id: String, tool_version: String, reason: String| {
        vec![BranchRow {
            branch,
            tool_id,
            tool_version,
            reason,
            call_index: 0,
        }]
    };
    match branch {
        ReactBranch::Answer => one("answer", String::new(), String::new(), String::new()),
        ReactBranch::Tool {
            tool_id,
            tool_version,
        } => one("tool", tool_id.clone(), tool_version.clone(), String::new()),
        // PR-3 (A2): a refused proposal the model re-prompts over — carry the
        // durable reason for operator troubleshooting (display only).
        ReactBranch::Rejected { reason } => {
            one("rejected", String::new(), String::new(), reason.clone())
        }
        ReactBranch::DeadLettered => {
            one("dead_lettered", String::new(), String::new(), String::new())
        }
        ReactBranch::Pending => one("pending", String::new(), String::new(), String::new()),
        // T-MULTI-ELEMENT-TOOLCALLS: N "tool" rows, call-indexed in emission order.
        ReactBranch::ToolBatch { calls } => calls
            .iter()
            .enumerate()
            .map(|(i, (tool_id, tool_version))| BranchRow {
                branch: "tool",
                tool_id: tool_id.clone(),
                tool_version: tool_version.clone(),
                reason: String::new(),
                call_index: u32::try_from(i).unwrap_or(u32::MAX),
            })
            .collect(),
    }
}

/// Fix C (T-CONNECTOR-AUTOGRANT-LIVE-DEADLETTER): the journal's
/// `ReactBranch::DeadLettered` carries NO durable reason field, so a dead-letter would
/// surface blank on the wire — opaque to an operator (the exact gap behind the silent
/// turn-0 dead-letter). Synthesize a DISPLAY reason from the chain's folded context: a
/// PURE projection read (never a journal write, never an identity/digest input), so it
/// changes no committed bytes. `chain` is every wire row sharing the dead-letter's
/// `(instance_id, step_salt)`. Precedence: (a) the most recent in-chain refusal (the
/// dead-letter usually follows a proposal the model could not correct), else (b/c) a
/// budget-exhaustion summary, else (d) a generic terminal flavor (a tool dispatch
/// failed / no admissible next turn) — never blank, never fabricated.
fn synthesize_dead_letter_reason(
    chain: &[&proto::ReactTurnSummary],
    dead: &proto::ReactTurnSummary,
) -> String {
    if let Some(reason) = chain
        .iter()
        .filter(|t| t.branch == "rejected" && !t.rejection_reason.is_empty())
        .max_by_key(|t| t.turn)
        .map(|t| t.rejection_reason.clone())
    {
        return format!(
            "dead-lettered: the model could not correct a refused proposal under its \
             budget — last refusal: {reason}"
        );
    }
    let tool_calls = chain.iter().filter(|t| t.branch == "tool").count();
    let tool_cap = usize::try_from(dead.max_tool_calls).unwrap_or(usize::MAX);
    if tool_cap > 0 && tool_calls >= tool_cap {
        return format!(
            "dead-lettered: tool-call budget exhausted ({tool_calls}/{tool_cap} tool \
             calls without a final answer)"
        );
    }
    let turns_used = usize::try_from(dead.turn)
        .unwrap_or(usize::MAX)
        .saturating_add(1);
    let turn_cap = usize::try_from(dead.max_turns).unwrap_or(usize::MAX);
    if turn_cap > 0 && turns_used >= turn_cap {
        return format!(
            "dead-lettered: turn budget exhausted ({turns_used}/{turn_cap} turns \
             without a final answer)"
        );
    }
    "dead-lettered: the chain could not progress (a tool dispatch failed or no further \
     turn was admissible)"
        .to_string()
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
        .flat_map(|entry| match entry {
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
                // One row per branch — EXCEPT a ToolBatch, which fans into N "tool"
                // rows sharing this turn's coordinates (T-MULTI-ELEMENT-TOOLCALLS).
                let step_salt_wire = step_salt.map(|s| s.to_vec()).unwrap_or_default();
                branch_wire(&branch)
                    .into_iter()
                    .map(|row| proto::ReactTurnSummary {
                        turn,
                        turn_mote_id: turn_mote_id.as_bytes().to_vec(),
                        instance_id: instance_id.to_vec(),
                        model_id: model_id.clone(),
                        branch: row.branch.to_string(),
                        tool_id: row.tool_id,
                        tool_version: row.tool_version,
                        max_turns,
                        max_tool_calls,
                        seq,
                        rejection_reason: row.reason,
                        // PR-R1: the chain key (empty for a legacy run-level `None` chain).
                        step_salt: step_salt_wire.clone(),
                        // T-MULTI-ELEMENT-TOOLCALLS: 0 for one-row branches; 0..N-1 for a batch.
                        call_index: row.call_index,
                    })
                    .collect::<Vec<_>>()
            }
            _ => Vec::new(),
        })
        .collect();
    // Fix C: fill any dead-letter row's blank reason from the chain's folded context
    // (the `DeadLettered` branch carries none). Done over the FULL fold (before paging)
    // so the chain context is complete even when the dead-letter lands on the page but
    // its earlier turns do not. Digest-neutral — a display-only projection read.
    let synthesized: Vec<(usize, String)> = all
        .iter()
        .enumerate()
        .filter(|(_, t)| t.branch == "dead_lettered" && t.rejection_reason.is_empty())
        .map(|(i, dead)| {
            let chain: Vec<&proto::ReactTurnSummary> = all
                .iter()
                .filter(|t| t.instance_id == dead.instance_id && t.step_salt == dead.step_salt)
                .collect();
            (i, synthesize_dead_letter_reason(&chain, dead))
        })
        .collect();
    for (i, reason) in synthesized {
        all[i].rejection_reason = reason;
    }
    // Newest-first (descending seq); within one turn's fanned ToolBatch (the rows
    // share one seq), ascending call_index so the trajectory reads left-to-right.
    all.sort_by(|a, b| b.seq.cmp(&a.seq).then(a.call_index.cmp(&b.call_index)));

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
            context_items_ref: None,
            image_ref: None,
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

    /// Patch the durable caps onto a `turn_fact` (the helper hardcodes 8/8).
    fn turn_fact_caps(
        turn: u32,
        instance: u8,
        branch: ReactBranch,
        max_turns: u32,
        max_tool_calls: u32,
    ) -> JournalEntry {
        let mut e = turn_fact(turn, instance, branch);
        if let JournalEntry::ReactRound {
            max_turns: mt,
            max_tool_calls: mtc,
            ..
        } = &mut e
        {
            *mt = max_turns;
            *mtc = max_tool_calls;
        }
        e
    }

    #[test]
    fn dead_letter_after_refusal_surfaces_the_last_refusal_reason() {
        // Fix C: the DeadLettered branch carries no reason; the view synthesizes one
        // from the chain — here, the most recent in-chain refusal (the disambiguating
        // reason from Fix A) so an operator sees WHY the chain died.
        let j = InMemoryJournal::new();
        j.append(turn_fact(0, 0xe0, ReactBranch::Pending)).unwrap();
        j.append(turn_fact(
            0,
            0xe0,
            ReactBranch::Rejected {
                reason: "the tool name `echo` is ambiguous — use the full id: \
                         mcp-echo/echo OR refconn/echo"
                    .to_string(),
            },
        ))
        .unwrap();
        j.append(turn_fact(0, 0xe0, ReactBranch::DeadLettered))
            .unwrap();
        let r = ReadOnly::new(j);

        let resp = list_react_turns(&r, None, None, None).unwrap();
        let dead = resp
            .turns
            .iter()
            .find(|t| t.branch == "dead_lettered")
            .expect("a dead-letter row");
        assert!(
            dead.rejection_reason.contains("last refusal:")
                && dead.rejection_reason.contains("ambiguous")
                && dead.rejection_reason.contains("refconn/echo"),
            "the dead-letter surfaces the last refusal: {}",
            dead.rejection_reason
        );
    }

    #[test]
    fn dead_letter_on_a_tool_tail_reports_budget_exhaustion() {
        // A TOOL tail that never answered: no refusal in the chain ⇒ the synthesis
        // reports the spent tool-call budget (1/1 here), never a blank reason.
        let j = InMemoryJournal::new();
        j.append(turn_fact_caps(
            0,
            0xe1,
            ReactBranch::Tool {
                tool_id: "refconn/reverse".to_string(),
                tool_version: "1".to_string(),
            },
            8,
            1,
        ))
        .unwrap();
        j.append(turn_fact_caps(0, 0xe1, ReactBranch::DeadLettered, 8, 1))
            .unwrap();
        let r = ReadOnly::new(j);

        let resp = list_react_turns(&r, None, None, None).unwrap();
        let dead = resp
            .turns
            .iter()
            .find(|t| t.branch == "dead_lettered")
            .expect("a dead-letter row");
        assert!(
            dead.rejection_reason.contains("tool-call budget exhausted")
                && dead.rejection_reason.contains("1/1"),
            "tool-tail dead-letter reports budget: {}",
            dead.rejection_reason
        );
    }

    #[test]
    fn dead_letter_with_no_context_gets_a_generic_terminal_reason() {
        // A dead-letter with no refusal and budget to spare (a dispatch failure) ⇒ the
        // generic terminal flavor — still informative, never blank.
        let j = InMemoryJournal::new();
        j.append(turn_fact_caps(0, 0xe2, ReactBranch::DeadLettered, 8, 8))
            .unwrap();
        let r = ReadOnly::new(j);

        let resp = list_react_turns(&r, None, None, None).unwrap();
        let dead = &resp.turns[0];
        assert_eq!(dead.branch, "dead_lettered");
        assert!(
            dead.rejection_reason.contains("could not progress")
                && dead.rejection_reason.contains("tool dispatch failed"),
            "generic terminal reason: {}",
            dead.rejection_reason
        );
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
        // A single-call Tool / non-tool branch carries call_index 0 (default).
        assert!(resp.turns.iter().all(|t| t.call_index == 0));
    }

    #[test]
    fn tool_batch_fans_into_call_indexed_tool_rows() {
        // T-MULTI-ELEMENT-TOOLCALLS: ONE ToolBatch fact fans into N "tool" rows,
        // call-indexed 0..N-1 in emission order, sharing the turn's coordinates.
        let j = InMemoryJournal::new();
        j.append(turn_fact(0, 0xc0, ReactBranch::Pending)).unwrap();
        j.append(turn_fact(
            0,
            0xc0,
            ReactBranch::ToolBatch {
                calls: vec![
                    ("mcp-echo".to_string(), "1".to_string()),
                    ("fs-read".to_string(), "1".to_string()),
                ],
            },
        ))
        .unwrap();
        j.append(turn_fact(1, 0xc0, ReactBranch::Answer)).unwrap();
        let r = ReadOnly::new(j);

        let resp = list_react_turns(&r, None, None, None).unwrap();
        // 1 Pending + 2 fanned tool rows + 1 Answer = 4 wire rows from 3 facts.
        assert_eq!(resp.turns.len(), 4, "the batch fact fans into 2 tool rows");
        let tool_rows: Vec<_> = resp.turns.iter().filter(|t| t.branch == "tool").collect();
        assert_eq!(tool_rows.len(), 2);
        // Both fanned rows share the turn's coordinates, distinguished by call_index.
        assert!(tool_rows.iter().all(|t| t.turn == 0));
        assert!(tool_rows
            .iter()
            .all(|t| t.turn_mote_id == tool_rows[0].turn_mote_id));
        assert!(tool_rows.iter().all(|t| t.seq == tool_rows[0].seq));
        let mut by_index: Vec<_> = tool_rows
            .iter()
            .map(|t| (t.call_index, t.tool_id.clone()))
            .collect();
        by_index.sort();
        assert_eq!(
            by_index,
            vec![(0, "mcp-echo".to_string()), (1, "fs-read".to_string())],
            "call_index 0 = first call, 1 = second, in emission order"
        );
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
