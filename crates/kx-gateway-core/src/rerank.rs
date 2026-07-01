//! `ListReRankTurns` — enumerate the journal's durable `ReRankRound` facts,
//! newest-first and paginated (RC4c-2). A read-only filter over the **off-DAG**
//! LLM-listwise-rerank metadata the live RAG chain commits — never a write, never
//! identity, never the projection digest. The operator-facing window into the
//! runtime's retrieval reranking: which reranks ran, on what model, and how the
//! retrieved candidates were reordered (the frozen permutation).
//!
//! SCOPE (single-node OSS): **operator-global** — the OSS journal records no party
//! on a `ReRankRound`, so this lists every rerank on the node (one operator, behind
//! the deny-all/bearer auth interceptor). An optional `instance_id` filter scopes to
//! one run (serve's journal is shared). CLOUD party-scopes this in the gateway-auth
//! layer (D102.1); do NOT fold party scoping in here.

use kx_journal::{JournalEntry, ReRankOutcome, INSTANCE_ID_LEN};
use kx_proto::proto;

use crate::error::{internal, GatewayError};
use crate::reader::JournalReader;

/// The server cap on a `ListReRankTurns` page. Bounds the response (and the
/// per-call allocation) regardless of journal size.
const MAX_PAGE: usize = 500;

/// The page size when the request omits `limit`.
const DEFAULT_PAGE: usize = 200;

/// The stable string tag for a rerank's frozen outcome (display + client render).
fn outcome_tag(outcome: &ReRankOutcome) -> &'static str {
    match outcome {
        ReRankOutcome::Pending => "pending",
        ReRankOutcome::Reranked { .. } => "reranked",
        ReRankOutcome::FailedClosed => "failed_closed",
    }
}

/// Fold the journal's `ReRankRound` facts and return one newest-first page of
/// rerank-turn summaries, optionally scoped to one run's `instance_id`. `limit` is
/// clamped to `[1, MAX_PAGE]` (or [`DEFAULT_PAGE`] when absent); a malformed
/// `instance_id` (wrong length) is refused loudly rather than silently ignored.
///
/// # Errors
/// [`GatewayError::InvalidArgument`] on a malformed `instance_id` filter;
/// [`GatewayError::Internal`] on a journal read failure (never an oracle — this
/// surface enumerates the operator's own node).
pub(crate) fn list_rerank_turns(
    reader: &dyn JournalReader,
    limit: Option<u32>,
    instance_filter: Option<&[u8]>,
) -> Result<proto::ListReRankTurnsResponse, GatewayError> {
    let filter: Option<[u8; INSTANCE_ID_LEN]> = match instance_filter {
        None => None,
        Some(raw) => Some(<[u8; INSTANCE_ID_LEN]>::try_from(raw).map_err(|_| {
            GatewayError::InvalidArgument("rerank instance_id filter must be 16 bytes")
        })?),
    };
    let head = reader.current_seq().map_err(internal)?;
    // Collect ReRankRound facts in ascending journal order, then reverse for
    // newest-first (reranks are opt-in + bounded per run, so this never
    // materializes a large vec).
    let mut all: Vec<proto::ReRankTurnSummary> = reader
        .read_entries_by_seq(0..head.saturating_add(1))
        .map_err(internal)?
        .filter_map(|entry| match entry {
            JournalEntry::ReRankRound {
                round,
                rerank_mote_id,
                instance_id,
                model_id,
                candidate_count,
                outcome,
                seq,
                ..
            } => {
                if filter.is_some_and(|f| f != instance_id) {
                    return None;
                }
                let permutation = match &outcome {
                    ReRankOutcome::Reranked { permutation } => permutation.clone(),
                    ReRankOutcome::Pending | ReRankOutcome::FailedClosed => Vec::new(),
                };
                Some(proto::ReRankTurnSummary {
                    round,
                    rerank_mote_id: rerank_mote_id.as_bytes().to_vec(),
                    instance_id: instance_id.to_vec(),
                    model_id,
                    outcome: outcome_tag(&outcome).to_string(),
                    candidate_count,
                    permutation,
                    seq,
                })
            }
            _ => None,
        })
        .collect();
    all.reverse(); // newest-first (descending seq)

    let page = limit.map_or(DEFAULT_PAGE, |l| (l as usize).clamp(1, MAX_PAGE));
    let has_more = all.len() > page;
    all.truncate(page);
    Ok(proto::ListReRankTurnsResponse {
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

    fn rerank(instance: u8, outcome: ReRankOutcome) -> JournalEntry {
        JournalEntry::ReRankRound {
            round: 0,
            rerank_mote_id: MoteId::from_bytes([instance ^ 0x5a; 32]),
            instance_id: [instance; INSTANCE_ID_LEN],
            base_results_ref: kx_content::ContentRef::from_bytes([0x11; 32]),
            query_ref: kx_content::ContentRef::from_bytes([0x22; 32]),
            warrant_ref: kx_content::ContentRef::from_bytes([0x33; 32]),
            model_id: "gemma-4".to_string(),
            candidate_count: 3,
            outcome,
            seq: 0,
        }
    }

    #[test]
    fn empty_journal_lists_nothing() {
        let r = ReadOnly::new(InMemoryJournal::new());
        let resp = list_rerank_turns(&r, None, None).unwrap();
        assert!(resp.turns.is_empty());
        assert!(!resp.has_more);
    }

    #[test]
    fn lists_newest_first_with_permutation_and_outcome() {
        let j = InMemoryJournal::new();
        j.append(rerank(0xa0, ReRankOutcome::Pending)).unwrap();
        j.append(rerank(
            0xa0,
            ReRankOutcome::Reranked {
                permutation: vec![2, 0, 1],
            },
        ))
        .unwrap();
        j.append(rerank(0xa0, ReRankOutcome::FailedClosed)).unwrap();
        let r = ReadOnly::new(j);

        let resp = list_rerank_turns(&r, None, None).unwrap();
        assert_eq!(resp.turns.len(), 3);
        assert!(!resp.has_more);
        // Newest-first: the FailedClosed fact (highest seq) leads.
        assert_eq!(resp.turns[0].outcome, "failed_closed");
        assert!(resp.turns[0].permutation.is_empty());
        // The Reranked fact carries the frozen permutation.
        assert_eq!(resp.turns[1].outcome, "reranked");
        assert_eq!(resp.turns[1].permutation, vec![2, 0, 1]);
        assert_eq!(resp.turns[1].candidate_count, 3);
        // Strictly descending seq.
        assert!(resp.turns[0].seq > resp.turns[1].seq);
        assert!(resp.turns[1].seq > resp.turns[2].seq);
    }

    #[test]
    fn instance_id_filter_scopes_to_one_run() {
        let j = InMemoryJournal::new();
        j.append(rerank(0xa0, ReRankOutcome::FailedClosed)).unwrap();
        j.append(rerank(0xb0, ReRankOutcome::FailedClosed)).unwrap();
        let r = ReadOnly::new(j);
        let resp = list_rerank_turns(&r, None, Some(&[0xb0; INSTANCE_ID_LEN])).unwrap();
        assert_eq!(resp.turns.len(), 1);
        assert_eq!(resp.turns[0].instance_id, vec![0xb0; INSTANCE_ID_LEN]);
    }

    #[test]
    fn malformed_instance_filter_is_refused() {
        let r = ReadOnly::new(InMemoryJournal::new());
        assert!(matches!(
            list_rerank_turns(&r, None, Some(&[0u8; 4])),
            Err(GatewayError::InvalidArgument(_))
        ));
    }

    #[test]
    fn limit_clamps_and_signals_has_more() {
        let j = InMemoryJournal::new();
        for _ in 0..5u8 {
            j.append(rerank(0xc0, ReRankOutcome::FailedClosed)).unwrap();
        }
        let r = ReadOnly::new(j);
        let resp = list_rerank_turns(&r, Some(2), None).unwrap();
        assert_eq!(resp.turns.len(), 2);
        assert!(resp.has_more, "3 turns remain beyond a page of 2");
    }

    #[test]
    fn non_rerank_facts_are_ignored() {
        let j = InMemoryJournal::new();
        j.append(JournalEntry::RunRegistered {
            instance_id: [1u8; INSTANCE_ID_LEN],
            recipe_fingerprint: [2u8; 32],
            ts: 0,
            seq: 0,
        })
        .unwrap();
        j.append(rerank(0xc0, ReRankOutcome::FailedClosed)).unwrap();
        let r = ReadOnly::new(j);
        let resp = list_rerank_turns(&r, None, None).unwrap();
        assert_eq!(resp.turns.len(), 1, "only ReRankRound facts are enumerated");
    }
}
