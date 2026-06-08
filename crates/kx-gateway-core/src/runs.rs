//! `ListRuns` — enumerate the journal's registered runs, newest-first and
//! paginated. A read-only fold over the run-registration facts via
//! kx-projection's **off-digest** [`RunMetadataFold`] (the same fold the planner
//! already uses for observability) — never a write, never identity, never the
//! projection digest.
//!
//! SCOPE (single-node OSS): **operator-global** — the OSS journal records no
//! party on a `RunRegistered`, so this lists every run on the node (one
//! operator, behind the deny-all/bearer auth interceptor). CLOUD party-scopes
//! this in the gateway-auth layer (D102.1); do NOT fold party scoping in here.
//!
//! Pagination defends the O(journal) fold against an unbounded response: a page
//! is at most [`MAX_PAGE`] runs; the client resumes with `before_seq` (the
//! `registered_seq` of the last run it saw).

use kx_projection::RunMetadataFold;
use kx_proto::proto;

use crate::error::{internal, GatewayError};
use crate::reader::JournalReader;

/// The server cap on a `ListRuns` page. Bounds the response (and the per-call
/// allocation) regardless of journal size; the client paginates with
/// `before_seq` for more.
const MAX_PAGE: usize = 500;

/// The page size when the request omits `limit`.
const DEFAULT_PAGE: usize = 100;

/// Fold the journal's `RunRegistered` facts and return one newest-first page of
/// run summaries. `before_seq` is the resume cursor (only runs whose
/// `registered_seq` is strictly less than it); `limit` is clamped to
/// `[1, MAX_PAGE]` (or [`DEFAULT_PAGE`] when absent).
///
/// # Errors
/// [`GatewayError::Internal`] on a journal read failure (never an oracle — this
/// surface enumerates the operator's own node).
pub(crate) fn list_runs(
    reader: &dyn JournalReader,
    limit: Option<u32>,
    before_seq: Option<u64>,
) -> Result<proto::ListRunsResponse, GatewayError> {
    let head = reader.current_seq().map_err(internal)?;
    let mut fold = RunMetadataFold::new();
    for entry in reader
        .read_entries_by_seq(0..head.saturating_add(1))
        .map_err(internal)?
    {
        fold.apply(&entry);
    }
    let md = fold.finish();

    let page = limit.map_or(DEFAULT_PAGE, |l| (l as usize).clamp(1, MAX_PAGE));
    // `records` are in ascending journal order (the reader yields ascending seq);
    // `.rev()` gives newest-first without an O(n log n) sort.
    let mut window = md
        .records
        .iter()
        .rev()
        .filter(|r| before_seq.is_none_or(|b| r.registered_seq < b));
    let runs: Vec<proto::RunSummary> = window
        .by_ref()
        .take(page)
        .map(|r| proto::RunSummary {
            instance_id: r.instance_id.to_vec(),
            recipe_fingerprint: r.recipe_fingerprint.to_vec(),
            registered_seq: r.registered_seq,
            registered_unix_ms: r.registered_ts,
        })
        .collect();
    // One more beyond the page ⇒ a further page exists.
    let has_more = window.next().is_some();

    Ok(proto::ListRunsResponse { runs, has_more })
}

#[cfg(test)]
mod tests {
    use kx_journal::{InMemoryJournal, Journal, JournalEntry, INSTANCE_ID_LEN};

    use crate::reader::ReadOnly;

    use super::*;

    fn reg(instance: u8, recipe: u8, ts: u64) -> JournalEntry {
        JournalEntry::RunRegistered {
            instance_id: [instance; INSTANCE_ID_LEN],
            recipe_fingerprint: [recipe; 32],
            ts,
            seq: 0, // journal `set_seq` overwrites this on append
        }
    }

    /// Append `n` runs (instance/recipe/ts == 1..=n) and wrap read-only.
    fn journal_with(n: u8) -> ReadOnly<InMemoryJournal> {
        let j = InMemoryJournal::new();
        for i in 1..=n {
            j.append(reg(i, i, u64::from(i) * 1000)).unwrap();
        }
        ReadOnly::new(j)
    }

    #[test]
    fn empty_journal_lists_nothing() {
        let r = ReadOnly::new(InMemoryJournal::new());
        let resp = list_runs(&r, None, None).unwrap();
        assert!(resp.runs.is_empty());
        assert!(!resp.has_more);
    }

    #[test]
    fn lists_newest_first_with_identity_recipe_and_ts() {
        let r = journal_with(3);
        let resp = list_runs(&r, None, None).unwrap();
        assert_eq!(resp.runs.len(), 3);
        assert!(!resp.has_more);
        // Newest-first: the third run (seq 3) leads.
        assert_eq!(resp.runs[0].instance_id, vec![3u8; INSTANCE_ID_LEN]);
        assert_eq!(resp.runs[0].recipe_fingerprint, vec![3u8; 32]);
        assert_eq!(resp.runs[0].registered_seq, 3);
        assert_eq!(resp.runs[0].registered_unix_ms, 3000);
        assert_eq!(resp.runs[2].registered_seq, 1);
        // Strictly descending seq (a valid newest-first cursor sequence).
        assert!(resp.runs[0].registered_seq > resp.runs[1].registered_seq);
        assert!(resp.runs[1].registered_seq > resp.runs[2].registered_seq);
    }

    #[test]
    fn limit_clamps_and_signals_has_more() {
        let r = journal_with(5);
        let resp = list_runs(&r, Some(2), None).unwrap();
        assert_eq!(resp.runs.len(), 2);
        assert!(resp.has_more, "3 runs remain beyond a page of 2");
        assert_eq!(resp.runs[0].registered_seq, 5);
        assert_eq!(resp.runs[1].registered_seq, 4);
    }

    #[test]
    fn before_seq_cursor_paginates_to_exhaustion() {
        let r = journal_with(5);
        // Page 1: newest 2.
        let p1 = list_runs(&r, Some(2), None).unwrap();
        assert_eq!(
            p1.runs.iter().map(|s| s.registered_seq).collect::<Vec<_>>(),
            vec![5, 4]
        );
        assert!(p1.has_more);
        // Page 2: before seq 4 ⇒ {3, 2}.
        let cursor = p1.runs.last().unwrap().registered_seq;
        let p2 = list_runs(&r, Some(2), Some(cursor)).unwrap();
        assert_eq!(
            p2.runs.iter().map(|s| s.registered_seq).collect::<Vec<_>>(),
            vec![3, 2]
        );
        assert!(p2.has_more);
        // Page 3: before seq 2 ⇒ {1}, no more.
        let p3 = list_runs(&r, Some(2), Some(p2.runs.last().unwrap().registered_seq)).unwrap();
        assert_eq!(
            p3.runs.iter().map(|s| s.registered_seq).collect::<Vec<_>>(),
            vec![1]
        );
        assert!(!p3.has_more);
    }

    #[test]
    fn limit_zero_clamps_to_one() {
        let r = journal_with(3);
        let resp = list_runs(&r, Some(0), None).unwrap();
        assert_eq!(resp.runs.len(), 1, "a zero limit clamps up to one");
        assert!(resp.has_more);
    }

    #[test]
    fn limit_above_cap_clamps_to_max_page() {
        let r = journal_with(3);
        let resp = list_runs(&r, Some(u32::MAX), None).unwrap();
        assert_eq!(
            resp.runs.len(),
            3,
            "an oversized limit still bounds by content"
        );
        assert!(!resp.has_more);
    }
}
