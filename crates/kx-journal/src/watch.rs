//! The journal change-notification seam — subscribe instead of poll.
//!
//! This module is private; everything in it is re-exported from the crate root, and the
//! contract lives on the public items so it renders in the published docs:
//! [`WatchableJournal`] holds the exactly-once argument, [`JournalWatch`] holds the
//! publish-ordering and watch-identity rules, and [`JournalSubscription`] holds the
//! lost-wakeup argument.
//!
//! Before this seam, every live surface re-read `Journal::current_seq` on a timer
//! because the journal offered nothing else. That cost an idle serve a steady stream of
//! reads it did not need and quantized commit→visibility latency to the poll interval.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use tokio::sync::watch;

/// The writer half of the change-notification seam: a monotone high-water `seq`.
///
/// Held by the backends and shared via [`Arc`]; also constructible directly, so a test
/// double can drive the same seam the production backends do rather than faking one.
///
/// # Where a commit is announced
///
/// At the [`crate::Journal::append`] / [`crate::Journal::append_batch`] boundary,
/// **after** the
/// transaction commits. Two ordering rules make that the only correct site:
///
/// - **Publishing before the commit is a permanently missed edge.** A woken reader would
///   read, see nothing (the row is not visible yet), and never be woken for that `seq`
///   again — the one watermark it was going to get has already been spent.
/// - **Not inside the per-entry funnel.** Offline migration replays an entire old journal
///   through that funnel; a hook there would fire once per historical entry, for work no
///   subscriber is waiting on.
///
/// A group commit announces its **highest** `seq`. For an all-new batch any member would
/// do, since subscribers read the journal for the real head — but a batch may mix dedupe
/// hits with new entries, and a dedupe hit returns its original, much older `seq`.
/// Publishing below the current watermark is suppressed as a regression, so a
/// mostly-duplicate batch would announce its one genuinely new entry to nobody.
///
/// # Watches are keyed by journal file, not by journal handle
///
/// A serve opens **two** [`SqliteJournal`](crate::SqliteJournal) handles on one path — one
/// writes, one reads. Two connections, two mutexes. A watch owned by a handle would be
/// invisible to the other handle, and the failure mode is silence: subscribers that simply
/// never wake, with no error anywhere.
///
/// So [`SqliteJournal::open`](crate::SqliteJournal::open) resolves its watch from a
/// process-level registry keyed on the **canonicalized path**. Same file ⇒ same watch,
/// with nothing to wire up and nothing to forget.
/// [`open_in_memory`](crate::SqliteJournal::open_in_memory) gets a private watch instead:
/// each in-memory database is a distinct journal, and sharing across them would be wrong.
///
/// **Scope: in-process.** A writer in another process advances the file but not this
/// process's watch. That is within contract — the coordinator is the sole journal writer
/// (`journal-txn.md` §7), and the shipped serve embeds it.
#[derive(Debug)]
pub struct JournalWatch {
    head: watch::Sender<u64>,
}

impl Default for JournalWatch {
    fn default() -> Self {
        Self {
            head: watch::channel(0).0,
        }
    }
}

impl JournalWatch {
    /// A fresh watch at watermark `0` (the empty-journal head).
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Publish a new high-water `seq`, waking every subscriber.
    ///
    /// **Monotone and idempotent**: a value at or below the current watermark is dropped
    /// without waking anyone. That matters twice over — a dedupe hit consumes no `seq` and
    /// so must not manufacture a wakeup, and two writers that commit concurrently may
    /// reach this call out of order, which must not walk the watermark backwards.
    pub fn publish(&self, head: u64) {
        self.head.send_if_modified(|current| {
            if head > *current {
                *current = head;
                true
            } else {
                false
            }
        });
    }

    /// The current watermark. Diagnostic — the journal, not this number, is the authority
    /// on what is durable.
    #[must_use]
    pub fn head(&self) -> u64 {
        *self.head.borrow()
    }

    /// A new subscription. Wakeups cover every commit published from now on; the caller's
    /// own cursor covers everything before, so there is no gap at the subscribe boundary.
    #[must_use]
    pub fn subscribe(&self) -> JournalSubscription {
        JournalSubscription {
            head: self.head.subscribe(),
        }
    }
}

/// The reader half: await [`changed`](JournalSubscription::changed) to learn the journal
/// advanced, then read the range the caller's cursor says it still owes.
///
/// # Examples
///
/// ```
/// use kx_journal::{InMemoryJournal, Journal, JournalEntry, WatchableJournal};
/// use kx_mote::MoteId;
///
/// # tokio_test_block_on(async {
/// let journal = InMemoryJournal::new();
/// let mut sub = journal.subscribe();
/// let mut cursor = journal.current_seq().unwrap();
///
/// journal
///     .append(JournalEntry::Failed {
///         mote_id: MoteId::from_bytes([1u8; 32]),
///         idempotency_key: [0xaa; 32],
///         seq: 0,
///         reason_class: kx_journal::FailureReason::TimedOut,
///         reporter_id: 42,
///     })
///     .unwrap();
///
/// sub.changed().await.unwrap();
/// let head = journal.current_seq().unwrap();
/// let owed: Vec<_> = journal.read_entries_by_seq(cursor + 1..head + 1).unwrap().collect();
/// assert_eq!(owed.len(), 1);
/// cursor = head;
/// # let _ = cursor;
/// # });
/// # fn tokio_test_block_on<F: std::future::Future>(f: F) -> F::Output {
/// #     tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(f)
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct JournalSubscription {
    head: watch::Receiver<u64>,
}

/// Every [`JournalWatch`] for this subscription's journal has been dropped, so no further
/// commit can ever be announced on it. A live journal handle keeps its watch alive, so in
/// practice this means the journal itself is gone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("journal watch closed: the journal it belonged to has been dropped")]
pub struct JournalWatchClosed;

impl JournalSubscription {
    /// Wait until the journal advances past the watermark this subscription last observed.
    ///
    /// Returns immediately when a commit landed since the previous call, so there is no
    /// lost wakeup between reading the head and awaiting the next one: the receiver tracks
    /// its own version, and a publish that races the caller's read still marks it stale.
    ///
    /// Wakeups **coalesce**. One return can cover any number of commits, which is why the
    /// caller must drive its own cursor rather than count wakeups.
    pub async fn changed(&mut self) -> Result<(), JournalWatchClosed> {
        self.head.changed().await.map_err(|_| JournalWatchClosed)
    }

    /// The watermark this subscription has observed. Diagnostic; see [`WatchableJournal`] for
    /// why this is a lower bound and not the authority.
    #[must_use]
    pub fn head(&self) -> u64 {
        *self.head.borrow()
    }
}

/// A [`Journal`](crate::Journal) that can announce its own commits.
///
/// Deliberately a **separate** trait rather than a method on `Journal`, and deliberately
/// **without a default body**. A defaulted `subscribe` that a backend forgot to override
/// would compile, pass every test that does not wait on it, and then hang every subscriber
/// in production — the failure is silence, and silence is the one failure mode this seam
/// exists to remove. Requiring the method makes a non-notifying backend a compile error.
///
/// Being additive also keeps the frozen `Journal` surface — and therefore every
/// `journal.append` call site — untouched.
///
/// Implemented for [`SqliteJournal`](crate::SqliteJournal) and
/// [`InMemoryJournal`](crate::InMemoryJournal). **Not** implemented for
/// [`ReplayJournal`](crate::ReplayJournal): that backend refuses every write, so its watch
/// could never fire, and a subscription that cannot move is worse than none — it looks
/// like a live seam and behaves like a hang. Not implementing the trait says so in the
/// type system.
pub trait WatchableJournal: crate::Journal {
    /// A subscription that wakes on every commit to this journal from now on.
    ///
    /// # The contract
    ///
    /// A subscriber observes **every committed entry exactly once**, including one that
    /// arrives while a write is in flight. That is a property of the *protocol*, not of
    /// the channel:
    ///
    /// - The notification payload is a **watermark** (the journal's high-water `seq`),
    ///   never the entries themselves. A channel carrying entries would have to drop them
    ///   under a slow subscriber — at-most-once, which is not the contract.
    /// - The subscriber keeps its own cursor and reads the contiguous half-open range
    ///   `[cursor + 1, head + 1)` from the journal. Successive ranges abut, so nothing is
    ///   skipped; they never overlap, so nothing is delivered twice. **Coalescing is
    ///   therefore harmless** — a watermark that jumps by fifty is as correct as fifty
    ///   wakeups, and a subscriber arriving mid-write simply reads a wider first range.
    ///
    /// Because the watermark is only a wakeup token,
    /// [`JournalSubscription::changed`] returns `()` rather than the head. The journal
    /// stays the single authority on what is durable; a consumer that treated a
    /// notification value as data would be reading a number that is by construction a
    /// lower bound.
    ///
    /// Subscribe **before** reading the head you intend to start from. A commit landing
    /// between the two is then either below that head or announced to this subscription;
    /// doing it the other way round leaves a window in which it is neither.
    fn subscribe(&self) -> JournalSubscription;
}

// ---------------------------------------------------------------------------
// The per-file watch registry
// ---------------------------------------------------------------------------

/// Live watches by canonicalized journal path.
///
/// [`Weak`] so a path whose handles have all been dropped stops being tracked — the map
/// holds no journal alive, and a re-open after a drop starts a fresh watch rather than
/// inheriting a stale watermark.
static WATCHES: OnceLock<Mutex<HashMap<PathBuf, Weak<JournalWatch>>>> = OnceLock::new();

/// The shared watch for `path`, creating it if this is the first handle.
///
/// `path` must already exist — call this *after* opening, since opening is what creates
/// the file, and canonicalization is what makes two spellings of one path agree. If it
/// cannot be canonicalized the caller gets a private watch: an unshared watch degrades to
/// "this handle notifies its own subscribers", which is the pre-existing behaviour for a
/// single-handle journal and never silently mis-attributes one file's commits to another.
pub(crate) fn watch_for_path(path: &Path) -> Arc<JournalWatch> {
    let Ok(key) = path.canonicalize() else {
        return JournalWatch::new();
    };
    let map = WATCHES.get_or_init(|| Mutex::new(HashMap::new()));
    let mut map = map.lock().expect("poisoned lock");

    if let Some(existing) = map.get(&key).and_then(Weak::upgrade) {
        return existing;
    }
    let fresh = JournalWatch::new();
    map.insert(key, Arc::downgrade(&fresh));
    // Reap paths whose handles have all gone. Cheap: the map holds one entry per journal
    // file ever opened in this process, and the serve opens exactly one.
    map.retain(|_, w| w.strong_count() > 0);
    fresh
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publish_is_monotone_and_ignores_a_regression() {
        let w = JournalWatch::new();
        w.publish(5);
        w.publish(3);
        assert_eq!(w.head(), 5, "a later, lower publish must not walk it back");
    }

    #[test]
    fn a_republished_watermark_does_not_wake_anyone() {
        let w = JournalWatch::new();
        w.publish(7);
        let mut sub = w.subscribe();
        w.publish(7); // a dedupe hit: consumed no seq, so it must announce nothing.
        assert!(
            sub.changed().now_or_never().is_none(),
            "re-publishing the same watermark woke a subscriber"
        );
    }

    #[test]
    fn two_handles_on_one_path_resolve_to_one_watch() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("run.kxjournal");
        std::fs::write(&path, b"").unwrap();

        let a = watch_for_path(&path);
        let b = watch_for_path(&path);
        assert!(
            Arc::ptr_eq(&a, &b),
            "two handles on one journal file must share one watch"
        );
    }

    #[test]
    fn different_paths_get_different_watches() {
        let tmp = tempfile::TempDir::new().unwrap();
        let one = tmp.path().join("one.kxjournal");
        let two = tmp.path().join("two.kxjournal");
        std::fs::write(&one, b"").unwrap();
        std::fs::write(&two, b"").unwrap();

        assert!(
            !Arc::ptr_eq(&watch_for_path(&one), &watch_for_path(&two)),
            "distinct journals must not share a watch"
        );
    }

    #[test]
    fn a_dropped_path_does_not_leave_a_stale_watermark_behind() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("run.kxjournal");
        std::fs::write(&path, b"").unwrap();

        let first = watch_for_path(&path);
        first.publish(42);
        drop(first);

        assert_eq!(
            watch_for_path(&path).head(),
            0,
            "re-opening after the last handle dropped must start a fresh watch"
        );
    }

    /// `now_or_never` without pulling in futures-util: poll once against a no-op waker.
    trait NowOrNever: std::future::Future + Sized {
        fn now_or_never(self) -> Option<Self::Output> {
            let mut fut = Box::pin(self);
            let waker = std::task::Waker::noop();
            let mut cx = std::task::Context::from_waker(waker);
            match fut.as_mut().poll(&mut cx) {
                std::task::Poll::Ready(v) => Some(v),
                std::task::Poll::Pending => None,
            }
        }
    }
    impl<F: std::future::Future + Sized> NowOrNever for F {}
}
