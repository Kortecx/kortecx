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
    check_run_ownership, frames_for_range, EventStream, EventTailer, JournalReader,
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
async fn read_head(
    reader: &Arc<dyn JournalReader>,
    tx: &mpsc::Sender<Result<proto::EventFrame, Status>>,
) -> Option<u64> {
    match reader.current_seq() {
        Ok(head) => Some(head),
        Err(error) => {
            let _ = tx.send(Err(Status::internal(error.to_string()))).await;
            None
        }
    }
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
