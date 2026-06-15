//! [`LiveTokenTailer`] — the broker-backed [`TokenTailer`] behind the gRPC
//! `StreamModelTokens` RPC and the WebSocket `/tokens` bridge (PR-4.2 /
//! T-STREAM1).
//!
//! It subscribes a caller to ONE mote's ADVISORY token stream from the in-process
//! [`TokenBroker`](crate::token_broker::TokenBroker): first the replay snapshot
//! (everything emitted before the caller joined — so a turn discovered by the
//! ~1s projection poll still sees its first tokens), then the live broadcast
//! receiver until the mote finishes, the client disconnects, the consumer lags,
//! or the server shuts down. It is **read-side only / out-of-band**: it never
//! writes the journal or touches the digest; the committed `result_ref` stays the
//! authority.
//!
//! ## Lifecycle + backpressure
//! - **Ownership FIRST** ([`check_run_ownership`]) — a clean pre-stream
//!   `permission_denied` (the caller owns the run, the StreamEvents precedent), so
//!   an unauthorized caller never subscribes. `mote_id` is the unguessable broker
//!   key (not a second journal gate — a fresh terminal mote isn't journaled yet).
//! - **Lagging consumer** → the broadcast receiver yields `Lagged`; the loop ends
//!   the stream with `Status::resource_exhausted` (the "CatchupRequired" signal,
//!   mirroring [`LiveTailer`](crate::live_tail::LiveTailer)); the client resumes a
//!   fresh `StreamModelTokens` from its last `seq`. The publisher (the synchronous
//!   inference owner thread) is NEVER stalled — broadcast overwrites, never blocks.
//! - **No task leak** — the loop `select!`s the receiver against [`mpsc::Sender::closed`]
//!   and the shutdown `watch`, so a disconnect or shutdown returns promptly.

use std::sync::Arc;

use kx_gateway_core::{check_run_ownership, JournalReader, TokenStream, TokenTailer};
use kx_proto::proto;
use tokio::sync::{broadcast, mpsc, watch};
use tokio_stream::wrappers::ReceiverStream;
use tonic::Status;

use crate::token_broker::{TokenBroker, TokenChunk};

/// Bounded per-subscriber chunk queue into the gRPC/WS stream. A consumer lagging
/// past this is dropped with `resource_exhausted` (CatchupRequired); it resumes
/// from its last `seq`. (The broker's broadcast ring is the upstream bound.)
const SUBSCRIBER_QUEUE: usize = 256;

/// The broker-backed live token tailer. Held in the `kx-gateway` binary (where
/// tokio `sync`/`time` + the broker live) so `kx-gateway-core` keeps its passive
/// read-fold dep wall — the [`LiveTailer`](crate::live_tail::LiveTailer) posture.
#[derive(Clone)]
pub(crate) struct LiveTokenTailer {
    broker: Arc<TokenBroker>,
    /// Flips to `true` on server shutdown so in-flight forward loops exit, their
    /// streams end, and tonic's graceful drain completes.
    shutdown: watch::Receiver<bool>,
}

impl LiveTokenTailer {
    /// Build a live token tailer over `broker` whose forward loops stop when
    /// `shutdown` flips to `true`.
    #[must_use]
    pub(crate) fn new(broker: Arc<TokenBroker>, shutdown: watch::Receiver<bool>) -> Self {
        Self { broker, shutdown }
    }
}

impl TokenTailer for LiveTokenTailer {
    #[allow(clippy::result_large_err)] // see the `TokenTailer` trait method.
    fn stream(
        &self,
        reader: Arc<dyn JournalReader>,
        instance_id: [u8; 16],
        mote_id: [u8; 32],
        since_seq: u64,
    ) -> Result<TokenStream, Status> {
        // Ownership is a clean PRE-stream error (uniform permission_denied), so an
        // unauthorized caller never subscribes to the broker. The gate is RUN
        // ownership (the StreamEvents precedent): the caller must own `instance_id`.
        // We do NOT also require `mote_id` to be journaled yet — a freshly-submitted
        // terminal mote (the common TTFT case) isn't in the projection when the
        // client subscribes right after Invoke. The `mote_id` is the broker key
        // (a server-derived, unguessable 32-byte id); cross-tenant scoping in cloud
        // is enforced by the SN-8 wall ABOVE gateway-core, exactly as for StreamEvents.
        check_run_ownership(reader.as_ref(), instance_id).map_err(Status::from)?;
        let (snapshot, rx) = self.broker.subscribe(mote_id);
        let (tx, out_rx) = mpsc::channel::<Result<proto::TokenChunk, Status>>(SUBSCRIBER_QUEUE);
        tokio::spawn(forward_loop(
            snapshot,
            rx,
            since_seq,
            tx,
            self.shutdown.clone(),
        ));
        Ok(Box::pin(ReceiverStream::new(out_rx)))
    }
}

/// Map a broker chunk to the wire chunk.
fn to_proto(chunk: TokenChunk) -> proto::TokenChunk {
    proto::TokenChunk {
        seq: chunk.seq,
        mote_id: chunk.mote_id.to_vec(),
        text_piece: chunk.text_piece,
        done: chunk.done,
    }
}

/// Emit the replay snapshot (≥ `since_seq`) then drain the live receiver until the
/// mote's terminal `done`, a client disconnect, a lag, or server shutdown.
async fn forward_loop(
    snapshot: Vec<TokenChunk>,
    mut rx: broadcast::Receiver<TokenChunk>,
    since_seq: u64,
    tx: mpsc::Sender<Result<proto::TokenChunk, Status>>,
    mut shutdown: watch::Receiver<bool>,
) {
    // 1) Replay everything emitted before this subscriber joined.
    for chunk in snapshot {
        if chunk.seq < since_seq {
            continue;
        }
        let done = chunk.done;
        if tx.send(Ok(to_proto(chunk))).await.is_err() {
            return; // client gone
        }
        if done {
            return; // the mote had already finished — the snapshot held its done
        }
    }
    // 2) Drain the live stream.
    loop {
        tokio::select! {
            recv = rx.recv() => match recv {
                Ok(chunk) => {
                    if chunk.seq < since_seq {
                        continue;
                    }
                    let done = chunk.done;
                    if tx.send(Ok(to_proto(chunk))).await.is_err() {
                        return; // client gone
                    }
                    if done {
                        return; // generation ended
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    // Slow consumer: CatchupRequired (the LiveTailer overflow posture).
                    let _ = tx
                        .send(Err(Status::resource_exhausted(
                            "catch up: resume StreamModelTokens from your last seq",
                        )))
                        .await;
                    return;
                }
                Err(broadcast::error::RecvError::Closed) => return, // broker evicted
            },
            () = tx.closed() => return,        // client disconnected
            res = shutdown.changed() => {
                // Shutdown signalled (or the sender dropped) — end the stream.
                if res.is_err() || *shutdown.borrow() {
                    return;
                }
            }
        }
    }
}
