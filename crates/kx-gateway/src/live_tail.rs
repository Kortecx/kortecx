//! [`LiveTailer`] — the R5 live-tail [`EventTailer`] (the gRPC `StreamEvents`
//! upgrade + the source the WebSocket bridge reuses).
//!
//! Unlike the default snapshot-to-head [`kx_gateway_core::SnapshotTailer`], this
//! keeps the stream OPEN: after catching up to the current head it **subscribes** to the
//! journal and emits a new [`EventFrame`] whenever the journal advances. An idle stream
//! therefore reads the journal not at all, and a commit reaches a client as fast as the
//! frame can be built rather than on the next tick of a timer. It is **read-side only**:
//! it never writes the journal or touches the digest; the coordinator stays the sole
//! writer.
//!
//! This used to poll `current_seq()` every 250 ms, because the journal exposed no change
//! notification. It does now ([`kx_journal::WatchableJournal`]), and the old cadence
//! survives only as [`crate::journal_signal::JournalSignal::Poll`] under
//! `KX_SERVE_JOURNAL_WATCH=off`.
//!
//! What did **not** change is the delivery protocol, and that is the reason the swap is
//! safe: a subscriber advances its own cursor and reads the contiguous range
//! `(cursor, head]` from the journal, so wakeups may coalesce or arrive spuriously
//! without affecting what is delivered. The notification decides *when* to read, never
//! *what* was written.
//!
//! ## Lifecycle + backpressure
//! - **Bounded per-subscriber queue** (`SUBSCRIBER_QUEUE` frames). A consumer that
//!   falls behind fills the queue; the follower then terminates the stream with
//!   `Status::resource_exhausted` (the "CatchupRequired" signal — it is a `Status`,
//!   not a wire field, since the frozen proto has no such message). The client
//!   resumes a fresh `StreamEvents` from its last `next_seq` — bounded memory, the
//!   journal + other subscribers untouched.
//! - **No task leak.** The follower `select!`s its journal signal against
//!   [`Sender::closed`]; when the client disconnects (the `ReceiverStream` drops),
//!   the follower returns promptly. A send error (receiver gone) also returns.

use std::sync::Arc;

use kx_gateway_core::{
    check_run_ownership, frames_for_range, global_frames_for_range, seed_global_cursor,
    EventStream, EventTailer, GlobalCursor, GlobalEventStream, GlobalEventTailer, JournalReader,
};
use kx_proto::proto;
use tokio::sync::{mpsc, watch};
use tokio_stream::wrappers::ReceiverStream;
use tonic::Status;

use crate::journal_signal::JournalSignal;

/// Bounded per-subscriber frame queue. A consumer that lags past this is dropped
/// with `resource_exhausted` (CatchupRequired) and resumes from its last `next_seq`.
const SUBSCRIBER_QUEUE: usize = 256;

/// The live-tail [`EventTailer`]: per subscriber, spawn a follower that emits frames
/// as the journal advances. Held in the `kx-gateway` binary (where tokio
/// `time`/`sync` live) so `kx-gateway-core` keeps its passive-read-fold dep wall.
#[derive(Clone)]
pub struct LiveTailer {
    /// Flips to `true` on server shutdown so in-flight follow loops exit promptly,
    /// their streams end, and tonic's graceful drain completes — a live stream
    /// otherwise keeps its RPC in-flight forever and would deadlock shutdown.
    shutdown: watch::Receiver<bool>,
}

impl LiveTailer {
    /// Build a live tailer whose follow loops stop when `shutdown` flips to `true`.
    #[must_use]
    pub fn new(shutdown: watch::Receiver<bool>) -> Self {
        Self { shutdown }
    }
}

impl EventTailer for LiveTailer {
    #[allow(clippy::result_large_err)] // see the `EventTailer` trait method.
    fn stream(
        &self,
        reader: Arc<dyn JournalReader>,
        instance_id: [u8; 16],
        since_seq: u64,
    ) -> Result<EventStream, Status> {
        // Ownership is a clean PRE-stream error (uniform permission_denied), so an
        // unauthorized caller never spawns a follower.
        check_run_ownership(reader.as_ref(), instance_id).map_err(Status::from)?;
        // Subscribe BEFORE the follower's catch-up read, which happens inside the spawned
        // task. The order is the contract: a commit landing in between is either below
        // the head that read observes, or announced to this subscription. Subscribing
        // after the read would leave a window in which it is neither.
        let signal = crate::env_caps::journal_signal(reader.as_ref());
        let (tx, rx) = mpsc::channel::<Result<proto::EventFrame, Status>>(SUBSCRIBER_QUEUE);
        tokio::spawn(follow_loop(
            reader,
            since_seq,
            tx,
            self.shutdown.clone(),
            signal,
        ));
        Ok(Box::pin(ReceiverStream::new(rx)))
    }
}

/// The per-subscriber follow loop: catch up to head, then emit on each advance until
/// the client disconnects, a read fails, the consumer falls too far behind, or the
/// server shuts down.
async fn follow_loop(
    reader: Arc<dyn JournalReader>,
    since_seq: u64,
    tx: mpsc::Sender<Result<proto::EventFrame, Status>>,
    mut shutdown: watch::Receiver<bool>,
    mut signal: JournalSignal,
) {
    // Subscribed during shutdown (or the sender already dropped): stop immediately.
    if *shutdown.borrow() {
        return;
    }
    let mut cursor = since_seq;

    // Initial catch-up: ALWAYS emit (frames_for_range yields a boundary frame even
    // when the range is empty, so the client learns it is caught up + the stream is
    // live). Afterwards, only emit when the head actually advanced — an idle stream
    // must not enqueue an empty boundary frame every tick.
    let Some(head) = read_head(&reader, &tx).await else {
        return;
    };
    if !emit_range(&reader, &mut cursor, head, &tx).await {
        return;
    }

    loop {
        // Wait for the journal to advance OR a client disconnect OR server shutdown
        // (prompt cleanup, no task leak, no shutdown deadlock).
        tokio::select! {
            () = signal.changed() => {}
            () = tx.closed() => return,
            _ = shutdown.changed() => return,
        }
        let Some(head) = read_head(&reader, &tx).await else {
            return;
        };
        // The `head > cursor` guard stays, and still earns its keep: a wakeup is a hint
        // that something changed, not a promise that THIS subscriber is owed a frame.
        if head > cursor && !emit_range(&reader, &mut cursor, head, &tx).await {
            return;
        }
    }
}

/// Read the journal head; on error, signal it (best-effort) and return `None`.
/// Generic over the frame type so both the per-run and the global follow loops
/// share it.
async fn read_head<F>(
    reader: &Arc<dyn JournalReader>,
    tx: &mpsc::Sender<Result<F, Status>>,
) -> Option<u64> {
    match reader.current_seq() {
        Ok(head) => Some(head),
        Err(error) => {
            let _ = tx.send(Err(Status::internal(error.to_string()))).await;
            None
        }
    }
}

/// The Batch C live GLOBAL tailer — the [`LiveTailer`] twin behind
/// `StreamAllEvents` (and the WS `/events/all` channel). Same journal signal,
/// bounded per-subscriber queue, CatchupRequired overflow, and shutdown
/// discipline; two deliberate differences: NO ownership gate (operator-global —
/// the host auth interceptor is the gate; cloud must party-scope or deny, the
/// proto flag) and a STATEFUL cursor carrying the run-attribution watermark
/// (seeded once at subscribe).
#[derive(Clone)]
pub struct GlobalLiveTailer {
    /// See [`LiveTailer::shutdown`].
    shutdown: watch::Receiver<bool>,
}

impl GlobalLiveTailer {
    /// Build a global live tailer whose follow loops stop when `shutdown` flips
    /// to `true`.
    #[must_use]
    pub fn new(shutdown: watch::Receiver<bool>) -> Self {
        Self { shutdown }
    }
}

impl GlobalEventTailer for GlobalLiveTailer {
    #[allow(clippy::result_large_err)] // see the `GlobalEventTailer` trait method.
    fn stream_all(
        &self,
        reader: Arc<dyn JournalReader>,
        since_seq: u64,
    ) -> Result<GlobalEventStream, Status> {
        // Seed the attribution watermark as a clean PRE-stream error: a reader
        // failure surfaces as `internal` before any follower spawns.
        let cursor = seed_global_cursor(reader.as_ref(), since_seq).map_err(Status::from)?;
        // See `LiveTailer::stream`: subscribe before the follower's catch-up read.
        let signal = crate::env_caps::journal_signal(reader.as_ref());
        let (tx, rx) = mpsc::channel::<Result<proto::GlobalEventFrame, Status>>(SUBSCRIBER_QUEUE);
        tokio::spawn(global_follow_loop(
            reader,
            cursor,
            tx,
            self.shutdown.clone(),
            signal,
        ));
        Ok(Box::pin(ReceiverStream::new(rx)))
    }
}

/// The global per-subscriber follow loop — the [`follow_loop`] twin over the
/// stateful [`GlobalCursor`]. Same lifecycle: catch up, then emit on each
/// advance until disconnect / read failure / overflow / shutdown.
///
/// The two-field cursor (seq advanced per delivered frame, run-attribution watermark
/// adopted only after a whole range lands) is **untouched** by the move off the timer.
/// Only what the loop waits on changed; what it does on waking did not.
async fn global_follow_loop(
    reader: Arc<dyn JournalReader>,
    mut cursor: GlobalCursor,
    tx: mpsc::Sender<Result<proto::GlobalEventFrame, Status>>,
    mut shutdown: watch::Receiver<bool>,
    mut signal: JournalSignal,
) {
    if *shutdown.borrow() {
        return;
    }
    let Some(head) = read_head(&reader, &tx).await else {
        return;
    };
    if !emit_global_range(&reader, &mut cursor, head, &tx).await {
        return;
    }

    loop {
        tokio::select! {
            () = signal.changed() => {}
            () = tx.closed() => return,
            _ = shutdown.changed() => return,
        }
        let Some(head) = read_head(&reader, &tx).await else {
            return;
        };
        if head > cursor.seq && !emit_global_range(&reader, &mut cursor, head, &tx).await {
            return;
        }
    }
}

/// Emit the global frames for `(cursor.seq, head]`, advancing the cursor per
/// sent frame (seq via `next_seq`; the watermark advanced by the range builder).
/// Returns `false` (stop) on a read error, a client disconnect, or a
/// slow-consumer overflow (CatchupRequired).
async fn emit_global_range(
    reader: &Arc<dyn JournalReader>,
    cursor: &mut GlobalCursor,
    head: u64,
    tx: &mpsc::Sender<Result<proto::GlobalEventFrame, Status>>,
) -> bool {
    // The range builder advances the FULL cursor (seq to head + watermark);
    // per-frame resume safety comes from re-tracking the sent frontier below,
    // so a mid-range stop resumes from the last DELIVERED frame's next_seq.
    let mut range_cursor = *cursor;
    let frames = match global_frames_for_range(reader.as_ref(), &mut range_cursor, head) {
        Ok(frames) => frames,
        Err(error) => {
            let _ = tx.send(Err(Status::from(error))).await;
            return false;
        }
    };
    for frame in frames {
        let next = frame.next_seq;
        match tx.try_send(Ok(frame)) {
            Ok(()) => cursor.seq = next, // advance per-frame so a mid-range stop resumes correctly
            Err(mpsc::error::TrySendError::Closed(_)) => return false, // client gone
            Err(mpsc::error::TrySendError::Full(_)) => {
                // Slow consumer: terminate with CatchupRequired (the LiveTailer
                // contract). The buffered frames drain first, then this error;
                // the client resumes a fresh StreamAllEvents from its last
                // next_seq (the seed pass re-derives the watermark). Bounded
                // memory; never blocks the journal or other subscribers.
                let _ = tx
                    .send(Err(Status::resource_exhausted(
                        "catch up: resume StreamAllEvents from your last next_seq",
                    )))
                    .await;
                return false;
            }
        }
    }
    // The whole range delivered: adopt the advanced watermark (correct for the
    // next poll round; a partial delivery returned above without adopting it —
    // the resumed subscriber re-seeds instead).
    cursor.instance = range_cursor.instance;
    true
}

/// Emit the frames for `(cursor, head]`, advancing `cursor` per sent frame.
/// Returns `false` (stop) on a read error, a client disconnect, or a slow-consumer
/// overflow (CatchupRequired).
async fn emit_range(
    reader: &Arc<dyn JournalReader>,
    cursor: &mut u64,
    head: u64,
    tx: &mpsc::Sender<Result<proto::EventFrame, Status>>,
) -> bool {
    let frames = match frames_for_range(reader.as_ref(), *cursor, head) {
        Ok(frames) => frames,
        Err(error) => {
            let _ = tx.send(Err(Status::from(error))).await;
            return false;
        }
    };
    for frame in frames {
        let next = frame.next_seq;
        match tx.try_send(Ok(frame)) {
            Ok(()) => *cursor = next, // advance per-frame so a mid-range stop resumes correctly
            Err(mpsc::error::TrySendError::Closed(_)) => return false, // client gone
            Err(mpsc::error::TrySendError::Full(_)) => {
                // Slow consumer: terminate with CatchupRequired. The buffered
                // frames drain first, then this error; the client resumes from its
                // last `next_seq`. Bounded memory; never blocks the journal.
                let _ = tx
                    .send(Err(Status::resource_exhausted(
                        "catch up: resume StreamEvents from your last next_seq",
                    )))
                    .await;
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use kx_content::ContentRef;
    use kx_gateway_core::ReadOnly;
    use kx_journal::{InMemoryJournal, Journal, JournalEntry, INSTANCE_ID_LEN};
    use kx_mote::{MoteDefHash, MoteId, NdClass};
    use smallvec::SmallVec;

    use super::*;

    /// The queue-overflow terminal, pinned directly: a tiny channel + a range
    /// that yields MORE frames than its capacity forces `try_send` Full — the
    /// emit must stop with the CatchupRequired `resource_exhausted`, and the
    /// cursor must have advanced only through the DELIVERED frames so a resume
    /// from it is loss-free. (The e2e stress envelope can't reach overflow:
    /// 4096-delta chunking keeps real frame counts far under the 256 cap.)
    #[tokio::test]
    async fn overflow_terminates_with_catchup_required_and_a_resumable_cursor() {
        let j = InMemoryJournal::new();
        j.append(JournalEntry::RunRegistered {
            instance_id: [7; INSTANCE_ID_LEN],
            recipe_fingerprint: [8; 32],
            ts: 1,
            seq: 0,
        })
        .unwrap();
        // > MAX_FRAME_DELTAS surfaced deltas ⇒ the range builds 2+ frames.
        for i in 0..4_100u32 {
            let mut id = [0u8; 32];
            id[..4].copy_from_slice(&i.to_le_bytes());
            j.append(JournalEntry::Committed {
                mote_id: MoteId::from_bytes(id),
                idempotency_key: id,
                seq: 0,
                nondeterminism: NdClass::Pure,
                result_ref: ContentRef::from_bytes(id),
                parents: SmallVec::new(),
                warrant_ref: ContentRef::from_bytes([0xaa; 32]),
                mote_def_hash: MoteDefHash::from_bytes([0x09; 32]),
            })
            .unwrap();
        }
        let reader: Arc<dyn JournalReader> = Arc::new(ReadOnly::new(j));
        let head = reader.current_seq().unwrap();

        // Capacity 1: the first frame fills the queue, the second overflows.
        // The emit runs in its OWN task (the production shape — the follower and
        // the consumer are concurrent): the terminal-error `send().await` only
        // completes once the consumer drains the buffered frame.
        let (tx, mut rx) = mpsc::channel::<Result<proto::GlobalEventFrame, Status>>(1);
        let emit = {
            let reader = reader.clone();
            tokio::spawn(async move {
                let mut cursor = kx_gateway_core::seed_global_cursor(reader.as_ref(), 0).unwrap();
                let delivered = emit_global_range(&reader, &mut cursor, head, &tx).await;
                (delivered, cursor)
            })
        };

        // The one buffered frame drains first…
        let first = rx.recv().await.unwrap().unwrap();
        // …then the CatchupRequired terminal.
        let terminal = rx.recv().await.unwrap().unwrap_err();
        assert_eq!(terminal.code(), tonic::Code::ResourceExhausted);
        let (delivered, cursor) = emit.await.unwrap();
        assert!(!delivered, "an overflow stops the emit");
        assert_eq!(
            cursor.seq, first.next_seq,
            "the cursor advanced ONLY through the delivered frame (resume-safe)"
        );

        // A resume from the delivered cursor covers the rest exactly once.
        let mut resume = kx_gateway_core::seed_global_cursor(reader.as_ref(), cursor.seq).unwrap();
        let frames =
            kx_gateway_core::global_frames_for_range(reader.as_ref(), &mut resume, head).unwrap();
        let resumed: u64 = frames.iter().map(|f| f.deltas.len() as u64).sum();
        let total: u64 = first.deltas.len() as u64 + resumed;
        assert_eq!(
            total, head,
            "delivered + resumed = every delta exactly once"
        );
    }

    /// A [`JournalReader`] that counts how many times it is asked for the head.
    ///
    /// The whole point of this change is a number that must read **zero** for an idle
    /// follower, and could never read zero while it polled.
    struct CountingReader {
        inner: ReadOnly<InMemoryJournal>,
        head_reads: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl JournalReader for CountingReader {
        fn read_entries_by_seq(
            &self,
            range: std::ops::Range<u64>,
        ) -> Result<Box<dyn Iterator<Item = JournalEntry> + '_>, kx_journal::JournalError> {
            self.inner.read_entries_by_seq(range)
        }

        fn current_seq(&self) -> Result<u64, kx_journal::JournalError> {
            self.head_reads
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.inner.current_seq()
        }

        fn subscribe(&self) -> kx_journal::JournalSubscription {
            self.inner.subscribe()
        }
    }

    /// **An idle follower must not touch the journal at all.**
    ///
    /// This is the A/B observable for the whole change, chosen because its null value is
    /// structurally impossible under the fix. "Frames still arrive" would read identically
    /// with polling or with a subscription and would prove nothing. "Latency improved" is a
    /// distribution, so a lucky sample proves nothing either. A *count of reads while
    /// nothing is happening* can only be zero if nothing polls.
    ///
    /// The magnitude is what makes it a signal rather than a threshold: over this window a
    /// 250 ms poll performs `IDLE_WINDOW / 250ms` reads, so the assertion is `== 0` against
    /// an expectation of eight — not a tuned bound that could drift into passing.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_idle_follower_never_reads_the_journal() {
        const IDLE_WINDOW: std::time::Duration = std::time::Duration::from_secs(2);

        let journal = InMemoryJournal::new();
        journal
            .append(JournalEntry::RunRegistered {
                instance_id: [7; INSTANCE_ID_LEN],
                recipe_fingerprint: [8; 32],
                ts: 1,
                seq: 0,
            })
            .unwrap();

        let head_reads = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let reader: Arc<dyn JournalReader> = Arc::new(CountingReader {
            inner: ReadOnly::new(journal),
            head_reads: head_reads.clone(),
        });

        let (_shutdown_tx, shutdown_rx) = watch::channel(false);
        let (tx, mut rx) = mpsc::channel::<Result<proto::EventFrame, Status>>(SUBSCRIBER_QUEUE);
        let signal = JournalSignal::Watch(reader.subscribe());
        let follower = tokio::spawn(follow_loop(reader, 0, tx, shutdown_rx, signal));

        // Drain the catch-up frame, then count only what happens after it. The catch-up
        // read is legitimate and is not what this test is about.
        let _catch_up = rx.recv().await.unwrap().unwrap();
        head_reads.store(0, std::sync::atomic::Ordering::SeqCst);

        tokio::time::sleep(IDLE_WINDOW).await;

        let observed = head_reads.load(std::sync::atomic::Ordering::SeqCst);
        follower.abort();
        assert_eq!(
            observed,
            0,
            "an idle follower read the journal head {observed} time(s) in {IDLE_WINDOW:?}; \
             the 250 ms poll this replaced would have read it {} times",
            IDLE_WINDOW.as_millis() / 250
        );
    }

    /// The converse, so the guard above is a measurement rather than a tautology: under
    /// `JournalSignal::Poll` — the legacy mode `KX_SERVE_JOURNAL_WATCH=off` selects — the
    /// same idle follower reads the head repeatedly.
    ///
    /// Without this, `== 0` could be passing because the follower is broken, or wired to
    /// nothing, or never spawned. Pinning both arms of the same code path is what turns
    /// the number into evidence, and it is also the proof that the operator rollback
    /// actually restores the old behaviour rather than merely claiming to.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_legacy_poll_mode_still_reads_the_journal_while_idle() {
        const IDLE_WINDOW: std::time::Duration = std::time::Duration::from_secs(2);

        let journal = InMemoryJournal::new();
        journal
            .append(JournalEntry::RunRegistered {
                instance_id: [7; INSTANCE_ID_LEN],
                recipe_fingerprint: [8; 32],
                ts: 1,
                seq: 0,
            })
            .unwrap();

        let head_reads = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let reader: Arc<dyn JournalReader> = Arc::new(CountingReader {
            inner: ReadOnly::new(journal),
            head_reads: head_reads.clone(),
        });

        let (_shutdown_tx, shutdown_rx) = watch::channel(false);
        let (tx, mut rx) = mpsc::channel::<Result<proto::EventFrame, Status>>(SUBSCRIBER_QUEUE);
        let signal = JournalSignal::Poll(crate::journal_signal::LEGACY_POLL_INTERVAL);
        let follower = tokio::spawn(follow_loop(reader, 0, tx, shutdown_rx, signal));

        let _catch_up = rx.recv().await.unwrap().unwrap();
        head_reads.store(0, std::sync::atomic::Ordering::SeqCst);

        tokio::time::sleep(IDLE_WINDOW).await;

        let observed = head_reads.load(std::sync::atomic::Ordering::SeqCst);
        follower.abort();
        // A loose floor, not a count: this exists to prove the two modes DIFFER, and a
        // tight number here would be a scheduler-timing flake with nothing to say.
        assert!(
            observed >= 2,
            "the legacy poll mode read the journal head only {observed} time(s) in \
             {IDLE_WINDOW:?} — it is not polling, so the zero measured in the default \
             mode is not evidence of anything"
        );
    }
}
