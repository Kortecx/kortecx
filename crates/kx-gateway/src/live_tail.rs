//! [`LiveTailer`] — the R5 live-tail [`EventTailer`] (the gRPC `StreamEvents`
//! upgrade + the source the WebSocket bridge reuses).
//!
//! Unlike the default snapshot-to-head [`kx_gateway_core::SnapshotTailer`], this
//! keeps the stream OPEN: after catching up to the current head it polls
//! `current_seq()` on a fixed interval and emits a new [`EventFrame`] whenever the
//! journal advances. The journal exposes no change-notification, so polling is the
//! honest interim — a push-based journal `watch` seam is a flagged optimization
//! (own PR). It is **read-side only**: it never writes the journal or touches the
//! digest; the coordinator stays the sole writer.
//!
//! ## Lifecycle + backpressure
//! - **Bounded per-subscriber queue** (`SUBSCRIBER_QUEUE` frames). A consumer that
//!   falls behind fills the queue; the poller then terminates the stream with
//!   `Status::resource_exhausted` (the "CatchupRequired" signal — it is a `Status`,
//!   not a wire field, since the frozen proto has no such message). The client
//!   resumes a fresh `StreamEvents` from its last `next_seq` — bounded memory, the
//!   journal + other subscribers untouched.
//! - **No task leak.** The poller `select!`s the poll timer against
//!   [`Sender::closed`]; when the client disconnects (the `ReceiverStream` drops),
//!   the poller returns promptly. A send error (receiver gone) also returns.

use std::sync::Arc;
use std::time::Duration;

use kx_gateway_core::{
    check_run_ownership, frames_for_range, global_frames_for_range, seed_global_cursor,
    EventStream, EventTailer, GlobalCursor, GlobalEventStream, GlobalEventTailer, JournalReader,
};
use kx_proto::proto;
use tokio::sync::{mpsc, watch};
use tokio_stream::wrappers::ReceiverStream;
use tonic::Status;

/// How often the poller re-reads `current_seq()` to detect new entries. Matches
/// the CLI/worker idle cadence; sub-interval latency awaits the push-based journal
/// `watch` seam (a flagged optimization).
const POLL_INTERVAL: Duration = Duration::from_millis(250);

/// Bounded per-subscriber frame queue. A consumer that lags past this is dropped
/// with `resource_exhausted` (CatchupRequired) and resumes from its last `next_seq`.
const SUBSCRIBER_QUEUE: usize = 256;

/// The live-tail [`EventTailer`]: per subscriber, spawn a poller that emits frames
/// as the journal advances. Held in the `kx-gateway` binary (where tokio
/// `time`/`sync` live) so `kx-gateway-core` keeps its passive-read-fold dep wall.
#[derive(Clone)]
pub struct LiveTailer {
    /// Flips to `true` on server shutdown so in-flight poll loops exit promptly,
    /// their streams end, and tonic's graceful drain completes — a live stream
    /// otherwise keeps its RPC in-flight forever and would deadlock shutdown.
    shutdown: watch::Receiver<bool>,
}

impl LiveTailer {
    /// Build a live tailer whose poll loops stop when `shutdown` flips to `true`.
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
        // unauthorized caller never spawns a poller.
        check_run_ownership(reader.as_ref(), instance_id).map_err(Status::from)?;
        let (tx, rx) = mpsc::channel::<Result<proto::EventFrame, Status>>(SUBSCRIBER_QUEUE);
        tokio::spawn(poll_loop(reader, since_seq, tx, self.shutdown.clone()));
        Ok(Box::pin(ReceiverStream::new(rx)))
    }
}

/// The per-subscriber poll loop: catch up to head, then emit on each advance until
/// the client disconnects, a read fails, the consumer falls too far behind, or the
/// server shuts down.
async fn poll_loop(
    reader: Arc<dyn JournalReader>,
    since_seq: u64,
    tx: mpsc::Sender<Result<proto::EventFrame, Status>>,
    mut shutdown: watch::Receiver<bool>,
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
        // Wait for the next tick OR a client disconnect OR server shutdown (prompt
        // cleanup, no task leak, no shutdown deadlock).
        tokio::select! {
            () = tokio::time::sleep(POLL_INTERVAL) => {}
            () = tx.closed() => return,
            _ = shutdown.changed() => return,
        }
        let Some(head) = read_head(&reader, &tx).await else {
            return;
        };
        if head > cursor && !emit_range(&reader, &mut cursor, head, &tx).await {
            return;
        }
    }
}

/// Read the journal head; on error, signal it (best-effort) and return `None`.
/// Generic over the frame type so both the per-run and the global poll loops
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
/// `StreamAllEvents` (and the WS `/events/all` channel). Same poll cadence,
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
    /// Build a global live tailer whose poll loops stop when `shutdown` flips
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
        // failure surfaces as `internal` before any poller spawns.
        let cursor = seed_global_cursor(reader.as_ref(), since_seq).map_err(Status::from)?;
        let (tx, rx) = mpsc::channel::<Result<proto::GlobalEventFrame, Status>>(SUBSCRIBER_QUEUE);
        tokio::spawn(global_poll_loop(reader, cursor, tx, self.shutdown.clone()));
        Ok(Box::pin(ReceiverStream::new(rx)))
    }
}

/// The global per-subscriber poll loop — the [`poll_loop`] twin over the
/// stateful [`GlobalCursor`]. Same lifecycle: catch up, then emit on each
/// advance until disconnect / read failure / overflow / shutdown.
async fn global_poll_loop(
    reader: Arc<dyn JournalReader>,
    mut cursor: GlobalCursor,
    tx: mpsc::Sender<Result<proto::GlobalEventFrame, Status>>,
    mut shutdown: watch::Receiver<bool>,
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
            () = tokio::time::sleep(POLL_INTERVAL) => {}
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
        // The emit runs in its OWN task (the production shape — the poller and
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
        assert_eq!(total, head, "delivered + resumed = every delta exactly once");
    }
}
