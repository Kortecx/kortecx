//! `ListReplanRounds` — enumerate the journal's durable `ReplanRound` facts,
//! newest-first and paginated (PR-2c-2). A read-only filter over the **off-DAG**
//! re-plan-round metadata the live re-plan-on-failure loop commits — never a write,
//! never identity, never the projection digest. The operator-facing window into the
//! runtime's self-correction: which rounds ran, what steps triggered them, and
//! whether the model escalated (flag-a-human).
//!
//! SCOPE (single-node OSS): **operator-global** — the OSS journal records no party
//! on a `ReplanRound`, so this lists every round on the node (one operator, behind
//! the deny-all/bearer auth interceptor). CLOUD party-scopes this in the
//! gateway-auth layer (D102.1); do NOT fold party scoping in here.
//!
//! Pagination defends the O(journal) fold against an unbounded response: a page is
//! at most [`MAX_PAGE`] rounds; rounds are sparse (≤ `MAX_SHAPER_ROUNDS` per run),
//! so a single default page covers any realistic run.

use kx_journal::JournalEntry;
use kx_proto::proto;

use crate::error::{internal, GatewayError};
use crate::reader::JournalReader;

/// The server cap on a `ListReplanRounds` page. Bounds the response (and the
/// per-call allocation) regardless of journal size.
const MAX_PAGE: usize = 500;

/// The page size when the request omits `limit`.
const DEFAULT_PAGE: usize = 200;

/// Fold the journal's `ReplanRound` facts and return one newest-first page of
/// round summaries. `limit` is clamped to `[1, MAX_PAGE]` (or [`DEFAULT_PAGE`] when
/// absent).
///
/// # Errors
/// [`GatewayError::Internal`] on a journal read failure (never an oracle — this
/// surface enumerates the operator's own node).
pub(crate) fn list_replan_rounds(
    reader: &dyn JournalReader,
    limit: Option<u32>,
) -> Result<proto::ListReplanRoundsResponse, GatewayError> {
    let head = reader.current_seq().map_err(internal)?;
    // Collect ReplanRound facts in ascending journal order, then reverse for
    // newest-first (rounds are sparse, so this never materializes a large vec).
    let mut all: Vec<proto::ReplanRoundSummary> = reader
        .read_entries_by_seq(0..head.saturating_add(1))
        .map_err(internal)?
        .filter_map(|entry| match entry {
            JournalEntry::ReplanRound {
                round,
                shaper_mote_id,
                model_id,
                failed_steps,
                escalation_reason_ref,
                seq,
                ..
            } => Some(proto::ReplanRoundSummary {
                round,
                shaper_mote_id: shaper_mote_id.as_bytes().to_vec(),
                model_id,
                failed_step_ids: failed_steps.iter().map(|m| m.as_bytes().to_vec()).collect(),
                escalated: escalation_reason_ref.is_some(),
                seq,
            }),
            _ => None,
        })
        .collect();
    all.reverse(); // newest-first (descending seq)

    let page = limit.map_or(DEFAULT_PAGE, |l| (l as usize).clamp(1, MAX_PAGE));
    let has_more = all.len() > page;
    all.truncate(page);
    Ok(proto::ListReplanRoundsResponse {
        rounds: all,
        has_more,
    })
}

#[cfg(test)]
mod tests {
    use kx_journal::{InMemoryJournal, Journal, JournalEntry};
    use kx_mote::MoteId;
    use smallvec::{smallvec, SmallVec};

    use crate::reader::ReadOnly;

    use super::*;

    fn anchor(shaper: u8) -> JournalEntry {
        JournalEntry::ReplanRound {
            round: 0,
            shaper_mote_id: MoteId::from_bytes([shaper; 32]),
            base_prompt_ref: kx_content::ContentRef::from_bytes([0u8; 32]),
            corrected_prompt_ref: kx_content::ContentRef::from_bytes([0u8; 32]),
            warrant_ref: kx_content::ContentRef::from_bytes([0u8; 32]),
            model_id: "m".to_string(),
            failed_steps: SmallVec::new(),
            escalation_reason_ref: None,
            seq: 0,
        }
    }

    fn corrective(round: u32, shaper: u8, escalated: bool) -> JournalEntry {
        let failed: SmallVec<[MoteId; 4]> = smallvec![MoteId::from_bytes([0xee; 32])];
        JournalEntry::ReplanRound {
            round,
            shaper_mote_id: MoteId::from_bytes([shaper; 32]),
            base_prompt_ref: kx_content::ContentRef::from_bytes([0u8; 32]),
            corrected_prompt_ref: kx_content::ContentRef::from_bytes([1u8; 32]),
            warrant_ref: kx_content::ContentRef::from_bytes([0u8; 32]),
            model_id: "m".to_string(),
            failed_steps: failed,
            escalation_reason_ref: escalated
                .then(|| kx_content::ContentRef::from_bytes([0x77; 32])),
            seq: 0,
        }
    }

    #[test]
    fn empty_journal_lists_nothing() {
        let r = ReadOnly::new(InMemoryJournal::new());
        let resp = list_replan_rounds(&r, None).unwrap();
        assert!(resp.rounds.is_empty());
        assert!(!resp.has_more);
    }

    #[test]
    fn lists_newest_first_with_failed_steps_and_escalation() {
        let j = InMemoryJournal::new();
        j.append(anchor(0xa0)).unwrap();
        j.append(corrective(1, 0xa1, false)).unwrap();
        j.append(corrective(2, 0xa2, true)).unwrap();
        let r = ReadOnly::new(j);

        let resp = list_replan_rounds(&r, None).unwrap();
        assert_eq!(resp.rounds.len(), 3);
        assert!(!resp.has_more);
        // Newest-first: round 2 (highest seq) leads, and it escalated.
        assert_eq!(resp.rounds[0].round, 2);
        assert!(resp.rounds[0].escalated);
        assert_eq!(resp.rounds[0].failed_step_ids.len(), 1);
        // The anchor (round 0) is last, with no failures + no escalation.
        assert_eq!(resp.rounds[2].round, 0);
        assert!(!resp.rounds[2].escalated);
        assert!(resp.rounds[2].failed_step_ids.is_empty());
        // Strictly descending seq.
        assert!(resp.rounds[0].seq > resp.rounds[1].seq);
        assert!(resp.rounds[1].seq > resp.rounds[2].seq);
    }

    #[test]
    fn limit_clamps_and_signals_has_more() {
        let j = InMemoryJournal::new();
        for i in 0..5u8 {
            j.append(corrective(u32::from(i) + 1, 0xb0 + i, false))
                .unwrap();
        }
        let r = ReadOnly::new(j);
        let resp = list_replan_rounds(&r, Some(2)).unwrap();
        assert_eq!(resp.rounds.len(), 2);
        assert!(resp.has_more, "3 rounds remain beyond a page of 2");
    }

    #[test]
    fn non_replan_facts_are_ignored() {
        let j = InMemoryJournal::new();
        j.append(JournalEntry::RunRegistered {
            instance_id: [1u8; kx_journal::INSTANCE_ID_LEN],
            recipe_fingerprint: [2u8; 32],
            ts: 0,
            seq: 0,
        })
        .unwrap();
        j.append(anchor(0xc0)).unwrap();
        let r = ReadOnly::new(j);
        let resp = list_replan_rounds(&r, None).unwrap();
        assert_eq!(
            resp.rounds.len(),
            1,
            "only ReplanRound facts are enumerated"
        );
        assert_eq!(resp.rounds[0].round, 0);
    }
}
