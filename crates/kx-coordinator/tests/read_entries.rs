//! `ReadEntries` serving (P2.4 distributed journal access, D55): a peer pulls
//! committed-entry deltas from a cursor and folds a local read model, so reads
//! scale off the coordinator's hot path while the journal stays single-writer.
//!
//! Contract exercised here:
//! - committed entries are returned in seq order, carrying the `result_ref`;
//! - `since_seq` is a cursor — a second poll returns only the delta;
//! - `next_seq` advances to `current_seq` once caught up;
//! - `max` caps a page and `next_seq` resumes right after the last returned entry;
//! - reads observe commits proposed before them (owner-thread flush ordering).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use kx_coordinator::CoordinatorService;
use kx_journal::InMemoryJournal;
use kx_mote::NdClass;

fn coordinator() -> CoordinatorService {
    CoordinatorService::new(InMemoryJournal::new())
}

#[tokio::test]
async fn reads_committed_entries_in_seq_order_with_result_ref() {
    let svc = coordinator();
    let warrant = common::sample_warrant();
    let worker = common::register(&svc, "w").await;

    let a = common::mote(1, NdClass::Pure, &[]);
    let b = common::mote(2, NdClass::Pure, &[a.id]);
    common::submit(&svc, &a, &warrant).await;
    common::submit(&svc, &b, &warrant).await;
    common::commit(&svc, &a, worker).await;
    common::commit(&svc, &b, worker).await;

    let resp = common::read_entries(&svc, 0, 16).await;
    assert_eq!(resp.entries.len(), 2, "both committed entries returned");

    let (id0, ref0, seq0) = common::committed_view(&resp.entries[0]);
    let (id1, _ref1, seq1) = common::committed_view(&resp.entries[1]);
    assert!(seq0 < seq1, "entries are in ascending seq order");
    assert_eq!(id0, a.id.as_bytes().to_vec(), "first committed is the root");
    assert_eq!(id1, b.id.as_bytes().to_vec());
    assert_eq!(ref0.len(), 32, "result_ref rides the wire");
    assert_eq!(
        resp.next_seq, seq1,
        "cursor caught up to the last committed seq"
    );

    // A second poll from the cursor returns no new work.
    let delta = common::read_entries(&svc, resp.next_seq, 16).await;
    assert!(delta.entries.is_empty(), "nothing new after the cursor");
}

#[tokio::test]
async fn since_seq_returns_only_the_delta() {
    let svc = coordinator();
    let warrant = common::sample_warrant();
    let worker = common::register(&svc, "w").await;

    let a = common::mote(1, NdClass::Pure, &[]);
    common::submit(&svc, &a, &warrant).await;
    common::commit(&svc, &a, worker).await;

    let first = common::read_entries(&svc, 0, 16).await;
    assert_eq!(first.entries.len(), 1);

    // Commit a second Mote, then poll from the prior cursor: only the new one.
    let b = common::mote(2, NdClass::Pure, &[a.id]);
    common::submit(&svc, &b, &warrant).await;
    common::commit(&svc, &b, worker).await;

    let delta = common::read_entries(&svc, first.next_seq, 16).await;
    assert_eq!(
        delta.entries.len(),
        1,
        "only the entry committed after the cursor"
    );
    let (id, _, _) = common::committed_view(&delta.entries[0]);
    assert_eq!(id, b.id.as_bytes().to_vec());
}

#[tokio::test]
async fn max_caps_the_page_and_cursor_resumes() {
    let svc = coordinator();
    let warrant = common::sample_warrant();
    let worker = common::register(&svc, "w").await;

    // Three independent ready PURE roots, all committed.
    let motes: Vec<_> = (1u8..=3)
        .map(|s| common::mote(s, NdClass::Pure, &[]))
        .collect();
    for m in &motes {
        common::submit(&svc, m, &warrant).await;
    }
    for m in &motes {
        common::commit(&svc, m, worker).await;
    }

    // Page of 2, then the cursor yields the remaining 1.
    let page1 = common::read_entries(&svc, 0, 2).await;
    assert_eq!(page1.entries.len(), 2, "first page capped at max");
    let page2 = common::read_entries(&svc, page1.next_seq, 2).await;
    assert_eq!(page2.entries.len(), 1, "remaining entry on the next page");

    // Pages do not overlap and cover all three.
    let mut ids: Vec<Vec<u8>> = page1
        .entries
        .iter()
        .chain(&page2.entries)
        .map(|e| common::committed_view(e).0)
        .collect();
    ids.sort();
    ids.dedup();
    assert_eq!(
        ids.len(),
        3,
        "the two pages cover all three commits, no overlap"
    );
}

#[tokio::test]
async fn empty_journal_reads_empty() {
    let svc = coordinator();
    let resp = common::read_entries(&svc, 0, 16).await;
    assert!(resp.entries.is_empty());
    assert_eq!(resp.next_seq, 0);
}
