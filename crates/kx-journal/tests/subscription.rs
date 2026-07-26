// Integration-test file: compiled as a separate crate from the host lib; inherits the
// workspace `[lints]` deny on `unwrap_used` / `expect_used`, which fixture construction
// legitimately uses. `pedantic` is allowed here for the same reason as `dod.rs`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]
//! The subscription contract: **a subscriber observes every committed entry exactly
//! once, under concurrent writers, including a subscriber that arrives mid-write.**
//!
//! Every test here is written so that it **cannot pass if the notification is absent**.
//! That is the whole point, and it is not automatic — the obvious version of an
//! exactly-once test is a loop that re-reads the journal and happens to await something
//! in between, which passes identically whether or not the wakeup ever arrives. So the
//! subscriber loop below reads **only after `changed()` returns**: delete the publish and
//! these tests hang to their deadline and fail, rather than quietly succeeding as
//! busy-polls. `dod.rs::writes_are_serialized_per_journal_handle` covers concurrent
//! writers alone; `proptest_entry.rs::reader_never_observes_partial_entry` covers a
//! reader racing a writer. This file covers the seam between them.
//!
//! Deadlines are generous and are failure bounds, not timing assertions — a broken build
//! must terminate rather than hang CI, but a slow machine must not go red.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use kx_content::ContentRef;
use kx_journal::{InMemoryJournal, Journal, JournalEntry, SqliteJournal, WatchableJournal};
use kx_mote::{MoteDefHash, MoteId, NdClass};
use smallvec::SmallVec;
use tempfile::TempDir;

/// How long a subscriber waits for the commits it is owed before declaring the seam
/// broken. Large enough that machine load cannot cause a false red; small enough that a
/// build with no notification at all fails in seconds rather than hanging.
const DRAIN_DEADLINE: Duration = Duration::from_secs(20);

const WRITERS: u8 = 4;
const PER_WRITER: u8 = 25;
const TOTAL: u64 = WRITERS as u64 * PER_WRITER as u64;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// A Committed entry with an identity unique to `(writer, n)`, so no two appends
/// anywhere in these tests dedupe against each other and every one consumes a `seq`.
fn committed(writer: u8, n: u8) -> JournalEntry {
    JournalEntry::Committed {
        mote_id: MoteId::from_bytes([
            writer, n, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0,
        ]),
        idempotency_key: [
            writer, n, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0,
        ],
        seq: 0, // assigned by the journal
        nondeterminism: NdClass::Pure,
        result_ref: ContentRef::from_bytes([writer ^ 0xa5; 32]),
        parents: SmallVec::new(),
        warrant_ref: ContentRef::from_bytes([0xaa; 32]),
        mote_def_hash: MoteDefHash::from_bytes([0x11; 32]),
    }
}

/// Drive one subscriber to completion and return **every `seq` it observed, in the order
/// it observed them** — duplicates included, because detecting a duplicate is half of
/// what these tests are for.
///
/// The protocol under test, and the reason it is exactly-once: read the contiguous
/// half-open range `(cursor, head]`, then advance the cursor to `head`. Successive
/// ranges abut, so nothing is skipped; they never overlap, so nothing is delivered twice.
/// Wakeups may coalesce freely — a watermark that jumps by 50 yields one wider read.
///
/// `from_cursor` is where the subscriber believes it already is. `0` means "I have seen
/// nothing"; `journal.current_seq()` at subscribe time means "I am caught up as of now".
///
/// `ready` fires once the initial catch-up read is done, and it is **load-bearing, not
/// instrumentation**. A test that spawns this and then immediately writes is racing the
/// spawn: if the catch-up read happens to run after the write, it reads the new entries
/// directly and the test passes *with the notification deleted*. That is not a
/// theoretical concern — it is how two of the guards in this file first passed a
/// deliberately broken build. A test whose subscriber must be caught up before the write
/// awaits this first, which is the repo's enter→park→release rendezvous idiom applied to
/// a subscriber instead of a capability.
async fn drain<J>(
    journal: Arc<J>,
    mut sub: kx_journal::JournalSubscription,
    from_cursor: u64,
    want: u64,
    ready: Option<tokio::sync::oneshot::Sender<()>>,
) -> Vec<u64>
where
    J: Journal + Send + Sync + 'static,
{
    let mut cursor = from_cursor;
    let mut seen = Vec::new();
    let deadline = tokio::time::Instant::now() + DRAIN_DEADLINE;

    // The initial catch-up: everything committed before this subscriber existed. A
    // subscriber that starts at the current head reads an empty range here, which is
    // correct — its first wakeup will carry the first commit it is actually owed.
    let head = journal.current_seq().unwrap();
    if head > cursor {
        seen.extend(read_range(journal.as_ref(), cursor, head));
        cursor = head;
    }
    if let Some(ready) = ready {
        let _ = ready.send(());
    }

    while (seen.len() as u64) < want {
        // Read ONLY after a wakeup. This is what makes the test a test: with the publish
        // removed, `changed()` never returns, the deadline elapses, and the assertion at
        // the call site reports a short set. A loop that re-read the head on each pass
        // would pass with or without the seam and would prove nothing.
        if tokio::time::timeout_at(deadline, sub.changed())
            .await
            .is_err()
        {
            break; // deadline: return what we have and let the caller assert on it
        }
        let head = journal.current_seq().unwrap();
        if head > cursor {
            seen.extend(read_range(journal.as_ref(), cursor, head));
            cursor = head;
        }
    }
    seen
}

fn read_range(journal: &dyn Journal, cursor: u64, head: u64) -> Vec<u64> {
    journal
        .read_entries_by_seq(cursor + 1..head + 1)
        .unwrap()
        .map(|e| e.seq())
        .collect()
}

/// Assert an observation is exactly the contiguous set `1..=want`, naming which of the
/// two failure modes occurred rather than just reporting a length mismatch.
fn assert_exactly_once(seen: &[u64], want: u64, who: &str) {
    let unique: BTreeSet<u64> = seen.iter().copied().collect();
    assert_eq!(
        seen.len(),
        unique.len(),
        "{who}: a commit was delivered more than once ({} observations, {} distinct)",
        seen.len(),
        unique.len()
    );
    let expected: BTreeSet<u64> = (1..=want).collect();
    let missed: Vec<u64> = expected.difference(&unique).copied().collect();
    assert!(
        missed.is_empty(),
        "{who}: missed {} of {want} commits — seqs {:?} were committed and never delivered",
        missed.len(),
        &missed[..missed.len().min(12)]
    );
    assert_eq!(
        unique.len() as u64,
        want,
        "{who}: observed seqs outside the committed range"
    );
}

/// Spawn `WRITERS` threads, each appending `PER_WRITER` entries with distinct identities.
fn spawn_writers<J>(journal: &Arc<J>) -> Vec<std::thread::JoinHandle<()>>
where
    J: Journal + Send + Sync + 'static,
{
    (0..WRITERS)
        .map(|w| {
            let j = Arc::clone(journal);
            std::thread::spawn(move || {
                for n in 0..PER_WRITER {
                    j.append(committed(w, n)).unwrap();
                }
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// G1 — exactly-once under concurrent writers
// ---------------------------------------------------------------------------

/// **The contract.** Four writers commit concurrently; three subscribers with different
/// arrival times must each end up holding the identical, complete, duplicate-free set.
///
/// The three arrival times are the three distinct cases:
/// - **before** any write — the steady-state subscriber;
/// - **mid-write** — the case the design is most likely to get wrong, because it spans
///   the join between "what I read at subscribe" and "what my first wakeup delivers";
/// - **after** all writers finish — pure catch-up, which must not need a wakeup at all.
///
/// Run against both backends: the SQLite path publishes after `txn.commit()`, the
/// in-memory path after the write lock is released, and both must satisfy the same
/// contract from the subscriber's side.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_commit_reaches_every_subscriber_exactly_once_sqlite() {
    let tmp = TempDir::new().unwrap();
    let journal = Arc::new(SqliteJournal::open(tmp.path().join("run.kxjournal")).unwrap());
    exactly_once_case(journal).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_commit_reaches_every_subscriber_exactly_once_in_memory() {
    exactly_once_case(Arc::new(InMemoryJournal::new())).await;
}

async fn exactly_once_case<J>(journal: Arc<J>)
where
    J: Journal + WatchableJournal + Send + Sync + 'static,
{
    // (1) Subscribed before anything was written. The rendezvous pins that: it must have
    //     completed its (empty) catch-up read before the first writer starts, so every
    //     one of the 100 commits has to arrive through a wakeup.
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let early = tokio::spawn(drain(
        Arc::clone(&journal),
        journal.subscribe(),
        0,
        TOTAL,
        Some(ready_tx),
    ));
    ready_rx.await.unwrap();

    let writers = spawn_writers(&journal);

    // (2) Subscribed while the writers are running — no rendezvous, because racing the
    //     writers IS the case under test. It starts from cursor 0, so it must reconstruct
    //     the entire set including whatever landed before it existed.
    let mid = tokio::spawn(drain(
        Arc::clone(&journal),
        journal.subscribe(),
        0,
        TOTAL,
        None,
    ));

    for w in writers {
        w.join().unwrap();
    }
    assert_eq!(
        journal.current_seq().unwrap(),
        TOTAL,
        "fixture: every append must consume a seq (a dedupe would invalidate the counts)"
    );

    // (3) Subscribed after the last commit. Its catch-up read alone must satisfy it — if
    //     this one needed a wakeup it would hang, because none is coming.
    let late = tokio::spawn(drain(
        Arc::clone(&journal),
        journal.subscribe(),
        0,
        TOTAL,
        None,
    ));

    assert_exactly_once(
        &early.await.unwrap(),
        TOTAL,
        "subscriber that arrived first",
    );
    assert_exactly_once(
        &mid.await.unwrap(),
        TOTAL,
        "subscriber that arrived mid-write",
    );
    assert_exactly_once(&late.await.unwrap(), TOTAL, "subscriber that arrived last");
}

// ---------------------------------------------------------------------------
// G2 — the mid-write arrival, at the boundary that matters
// ---------------------------------------------------------------------------

/// A subscriber that arrives mid-write and declares itself **already caught up** must
/// still receive every *later* commit exactly once.
///
/// This is the sharper half of the mid-write case. G1's mid subscriber starts at cursor 0,
/// so a missed edge at the subscribe instant is masked by its catch-up read. Here the
/// cursor is sampled at subscribe time, so the join is load-bearing: any commit that
/// lands between `subscribe()` and the `current_seq()` sample must be covered by exactly
/// one of the two, never by neither.
///
/// The assertion is therefore on the *tail*: everything above the sampled cursor arrives,
/// contiguously, once each.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_subscriber_that_arrives_mid_write_misses_nothing() {
    let tmp = TempDir::new().unwrap();
    let journal = Arc::new(SqliteJournal::open(tmp.path().join("run.kxjournal")).unwrap());

    // Some history to arrive after.
    for n in 0..10 {
        journal.append(committed(9, n)).unwrap();
    }

    let writers = spawn_writers(&journal);

    // Subscribe FIRST, then sample the cursor. The order matters and is the contract:
    // the subscription covers every commit from the moment it exists, so a commit racing
    // this pair is either below the sampled cursor (already durable, read by the caller)
    // or announced by a wakeup. Sampling first and subscribing second would open a
    // window in which a commit is neither.
    let sub = journal.subscribe();
    let cursor = journal.current_seq().unwrap();

    let tail_expected = TOTAL + 10 - cursor;
    let observed = drain(Arc::clone(&journal), sub, cursor, tail_expected, None).await;

    for w in writers {
        w.join().unwrap();
    }

    let unique: BTreeSet<u64> = observed.iter().copied().collect();
    assert_eq!(
        observed.len(),
        unique.len(),
        "a commit after the subscribe boundary was delivered twice"
    );
    let expected: BTreeSet<u64> = (cursor + 1..=TOTAL + 10).collect();
    assert_eq!(
        unique, expected,
        "the tail above the subscribe boundary must arrive complete and contiguous \
         (a hole at `cursor + 1` is the missed edge this test exists to catch)"
    );
}

// ---------------------------------------------------------------------------
// G3 — the production topology: two handles, one file
// ---------------------------------------------------------------------------

/// **The serve opens two journal handles on one path** — the embedded coordinator writes
/// through one, the gateway's read seam reads through the other. A watch owned by a
/// handle would therefore never reach the other handle's subscribers, and the symptom
/// would be silence: streams that simply never advance, with no error to trace.
///
/// So: append through handle A, and require a subscriber taken from handle B to wake and
/// read it. Under a per-handle watch this test hangs and fails. It is the only test here
/// that can catch that failure, and that failure is the most likely one in the change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_subscriber_on_a_second_handle_sees_the_first_handles_commits() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("run.kxjournal");

    let writer = Arc::new(SqliteJournal::open(&path).unwrap());
    let reader = Arc::new(SqliteJournal::open(&path).unwrap());

    // The subscription comes from the READ handle; every commit comes from the WRITE
    // handle. Nothing in this test touches both.
    let sub = reader.subscribe();
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let observed = tokio::spawn(drain(Arc::clone(&reader), sub, 0, TOTAL, Some(ready_tx)));
    ready_rx.await.unwrap();

    let writers = spawn_writers(&writer);
    for w in writers {
        w.join().unwrap();
    }

    assert_exactly_once(
        &observed.await.unwrap(),
        TOTAL,
        "a subscriber on the serve's read handle",
    );
}

/// The converse, so the sharing above is identity-based rather than accidental: two
/// in-memory journals are two different journals, and one must not announce the other's
/// commits.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_distinct_journals_do_not_cross_announce() {
    let one = Arc::new(InMemoryJournal::new());
    let two = Arc::new(InMemoryJournal::new());

    let mut sub = two.subscribe();
    one.append(committed(1, 1)).unwrap();

    assert!(
        tokio::time::timeout(Duration::from_millis(250), sub.changed())
            .await
            .is_err(),
        "a commit to one journal woke a subscriber on a different journal"
    );
}

// ---------------------------------------------------------------------------
// Batch delivery
// ---------------------------------------------------------------------------

/// A group commit must announce its **whole** range, not just its first entry.
///
/// `append_batch` assigns N contiguous seqs and makes them visible at once. Announcing
/// the first would leave entries 2..N unannounced until some later, unrelated write
/// happened to raise the watermark past them — a stall that is invisible on a busy
/// journal and permanent on an idle one. The subscriber here is owed the entire batch off
/// a single wakeup.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_group_commit_announces_its_whole_range() {
    let tmp = TempDir::new().unwrap();
    let journal = Arc::new(SqliteJournal::open(tmp.path().join("run.kxjournal")).unwrap());

    let batch: Vec<JournalEntry> = (0..32).map(|n| committed(7, n)).collect();
    let want = batch.len() as u64;

    // Rendezvous: the subscriber must be caught up (on an empty journal) BEFORE the batch
    // lands, or its catch-up read alone could satisfy it and the notification would go
    // untested. Without this the test passes on a build with no publish at all.
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let observed = tokio::spawn(drain(
        Arc::clone(&journal),
        journal.subscribe(),
        0,
        want,
        Some(ready_tx),
    ));
    ready_rx.await.unwrap();

    journal.append_batch(batch).unwrap();

    assert_exactly_once(&observed.await.unwrap(), want, "a batch subscriber");
}

/// A batch that is mostly re-appends must still announce the one entry that was new.
///
/// This is the case that decides *which* seq a batch publishes, and it is not the obvious
/// one. For an all-new batch any member's seq would do, because the subscriber reads
/// `current_seq()` as the authority and would find the whole range anyway. But a dedupe
/// hit returns its **original** seq, which may be far below the current watermark — so a
/// batch that publishes anything less than its maximum announces a watermark that
/// `publish` correctly refuses to move, and the one genuinely new commit in the batch
/// reaches nobody. On a busy journal the next unrelated write hides it; on an idle one it
/// is permanent.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_batch_of_mostly_dedupe_hits_still_announces_its_one_new_commit() {
    let tmp = TempDir::new().unwrap();
    let journal = Arc::new(SqliteJournal::open(tmp.path().join("run.kxjournal")).unwrap());

    // History: seqs 1..=20, all of which the batch below will re-present.
    let history: Vec<JournalEntry> = (0..20).map(|n| committed(4, n)).collect();
    journal.append_batch(history.clone()).unwrap();
    assert_eq!(journal.current_seq().unwrap(), 20, "fixture");

    // A subscriber that is fully caught up: it will see nothing on its catch-up read and
    // is owed exactly one commit, which can only reach it via a wakeup. The rendezvous is
    // what makes that true — without it the spawned catch-up read can land *after* the
    // batch and satisfy the test directly, which is precisely how this guard first passed
    // a build whose batch published the wrong seq.
    let sub = journal.subscribe();
    let cursor = journal.current_seq().unwrap();
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let observed = tokio::spawn(drain(Arc::clone(&journal), sub, cursor, 1, Some(ready_tx)));
    ready_rx.await.unwrap();

    // 20 dedupe hits (returning seqs 1..=20) plus one new entry (seq 21). The minimum
    // seq in the durable result is 1; the maximum is 21. Only the maximum moves the
    // watermark.
    let mut batch = history;
    batch.push(committed(4, 99));
    let durable = journal.append_batch(batch).unwrap();
    assert_eq!(
        durable.len(),
        21,
        "fixture: every input returns a durable form"
    );
    assert_eq!(
        journal.current_seq().unwrap(),
        21,
        "fixture: exactly one new seq"
    );

    let observed = observed.await.unwrap();
    assert_eq!(
        observed,
        vec![21],
        "the batch's single new commit was never announced — publishing any seq below \
         the batch maximum is suppressed as a watermark regression"
    );
}

// ---------------------------------------------------------------------------
// The seam's own invariants
// ---------------------------------------------------------------------------

/// A dedupe hit is not a commit. `Committed` dedupes by idempotency key and consumes no
/// `seq`, so the re-append is a no-op that must announce nothing — otherwise an idle
/// serve retrying a settled Mote would wake every subscriber on the box to deliver an
/// empty range.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_dedupe_hit_announces_nothing() {
    let journal = InMemoryJournal::new();
    journal.append(committed(3, 3)).unwrap();

    let mut sub = journal.subscribe();
    let again = journal.append(committed(3, 3)).unwrap();
    assert_eq!(
        again.seq(),
        1,
        "fixture: the second append must be a dedupe hit"
    );

    assert!(
        tokio::time::timeout(Duration::from_millis(250), sub.changed())
            .await
            .is_err(),
        "a dedupe no-op woke subscribers"
    );
}

/// Wakeups coalesce, and that is by design — the subscriber's cursor, not the wakeup
/// count, is what makes delivery complete. This pins the property so a future change that
/// tried to guarantee one-wakeup-per-commit would have to argue with a test rather than
/// silently make the seam more expensive than polling under load.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn coalesced_wakeups_still_deliver_every_commit() {
    let journal = Arc::new(InMemoryJournal::new());
    let mut sub = journal.subscribe();

    // Commit a burst with no subscriber awake to observe the intermediate watermarks.
    for n in 0..50 {
        journal.append(committed(5, n)).unwrap();
    }

    // Bounded, because an unbounded `changed().await` in a test does not fail on a broken
    // build — it HANGS, which blocks the suite instead of reporting. (Found exactly that
    // way: with the publish deleted, this test ran forever while its four siblings failed
    // cleanly in twenty seconds.) A guard must terminate to be a guard.
    tokio::time::timeout(DRAIN_DEADLINE, sub.changed())
        .await
        .expect("no wakeup arrived for a 50-commit burst")
        .unwrap();
    let head = journal.current_seq().unwrap();
    let delivered = read_range(journal.as_ref(), 0, head);
    assert_eq!(
        delivered.len(),
        50,
        "one wakeup after a 50-commit burst must still yield all 50 (the range read is \
         what delivers, not the wakeup count)"
    );
}
