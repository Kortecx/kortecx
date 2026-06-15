//! The ADVISORY token broker (PR-4.2 / T-STREAM1) — the in-process rendezvous
//! between the model executor (the PUBLISHER, deep on the inference owner thread)
//! and the live-token subscribers (the gRPC `StreamModelTokens` tailer + the
//! browser WS `/tokens` bridge).
//!
//! ## Out-of-band, never truth
//! Tokens here NEVER touch the journal, the digest, or identity. The durable
//! fact stays the committed `result_ref` (the WHOLE completion). The broker is a
//! pure read-tap on the generation loop: a subscriber that ignores it still
//! polls `GetProjection` and fetches the committed bytes (the authority). A serve
//! that never wires the broker simply has no live tokens (the default
//! `NoTokenTailer` serves an empty stream).
//!
//! ## Key = `mote_id`
//! The publisher (`ModelRouterExecutor::dispatch_model`) only knows `mote.id`, so
//! the broker is keyed by the 32-byte (server-derived) `MoteId`. A subscriber
//! presents `instance_id` (its run-ownership ticket) AND `mote_id`; the gateway
//! gate (`check_mote_in_run`) proves the caller owns the run AND the mote belongs
//! to it before any `subscribe`.
//!
//! ## Lifecycle (bounded + self-cleaning)
//! Each mote gets a [`broadcast`] channel (non-blocking publisher — a slow
//! subscriber can NEVER stall the synchronous inference owner thread) plus a
//! bounded `history` ring so a subscriber that joins mid-generation (e.g. a ReAct
//! turn discovered by the ~1s projection poll) still replays the tokens emitted
//! before it connected. `finish` posts the terminal `done` chunk; `evict_idle`
//! sweeps channels idle past the TTL (and finished + drained ones), so a long-
//! lived serve never grows unboundedly.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::broadcast;

/// The broadcast channel capacity AND the per-mote history ring size. One chunk
/// per generated token; 1024 comfortably covers a typical chat completion. A
/// subscriber lagging by more than this gets `Lagged` (→ `ResourceExhausted` →
/// resume), and history older than this is dropped from the replay snapshot.
const RING_CAP: usize = 1024;

/// How long a mote's channel survives its last activity before `evict_idle`
/// reclaims it. Generous enough that a late subscriber to a just-finished mote
/// still gets the `done` + history, bounded enough to self-clean.
pub(crate) const BROKER_TTL: Duration = Duration::from_secs(60);

/// One ADVISORY token chunk — the in-process twin of `proto::TokenChunk`. `seq`
/// is a per-mote monotone counter assigned here (NOT a journal seq). `text_piece`
/// is the NEW detokenized bytes for this step; the concatenation in `seq` order
/// equals the committed completion. `done` marks the terminal chunk.
#[derive(Clone, Debug)]
pub(crate) struct TokenChunk {
    pub seq: u64,
    pub mote_id: [u8; 32],
    pub text_piece: Vec<u8>,
    pub done: bool,
}

/// One mote's live stream: the broadcast sender (future chunks), a bounded
/// history ring (replay for late joiners), the next seq to assign, an idle stamp
/// for TTL eviction, and the terminal flag.
struct MoteChannel {
    tx: broadcast::Sender<TokenChunk>,
    history: VecDeque<TokenChunk>,
    next_seq: u64,
    last_touch: Instant,
    done: bool,
}

impl MoteChannel {
    fn new() -> Self {
        let (tx, _rx) = broadcast::channel(RING_CAP);
        Self {
            tx,
            history: VecDeque::with_capacity(64),
            next_seq: 0,
            last_touch: Instant::now(),
            done: false,
        }
    }

    /// Append a chunk to the bounded history and fan it out. A send error (no
    /// live receivers) is fine — the history still records it for a later joiner.
    fn push(&mut self, chunk: TokenChunk) {
        if self.history.len() >= RING_CAP {
            self.history.pop_front();
        }
        self.history.push_back(chunk.clone());
        let _ = self.tx.send(chunk);
        self.last_touch = Instant::now();
    }
}

/// Process-wide token broker, keyed by `mote_id`. Cheap to clone (one shared
/// `Arc<Mutex<..>>`); the publisher holds one clone, each subscriber path another.
#[derive(Clone, Default)]
pub(crate) struct TokenBroker {
    inner: Arc<Mutex<HashMap<[u8; 32], MoteChannel>>>,
}

impl TokenBroker {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Publish one token's NEW bytes for `mote_id`. Get-or-create the channel,
    /// assign the next seq, record + fan out. **Non-blocking + infallible** (the
    /// `UsageSink` posture): a poisoned lock or absent subscriber never disturbs
    /// the synchronous inference owner thread that calls this per token.
    pub(crate) fn publish(&self, mote_id: [u8; 32], piece: &[u8]) {
        let Ok(mut map) = self.inner.lock() else {
            return;
        };
        let ch = map.entry(mote_id).or_insert_with(MoteChannel::new);
        if ch.done {
            return; // a stray publish after finish — ignore (never re-open).
        }
        let seq = ch.next_seq;
        ch.next_seq = ch.next_seq.saturating_add(1);
        ch.push(TokenChunk {
            seq,
            mote_id,
            text_piece: piece.to_vec(),
            done: false,
        });
    }

    /// Post the terminal `done` chunk for `mote_id` and mark it finished. MUST be
    /// called on BOTH the success and error paths of a dispatch so a subscriber's
    /// stream always ends (never hangs). Idempotent: a second `finish` is a no-op.
    pub(crate) fn finish(&self, mote_id: [u8; 32]) {
        let Ok(mut map) = self.inner.lock() else {
            return;
        };
        let ch = map.entry(mote_id).or_insert_with(MoteChannel::new);
        if ch.done {
            return;
        }
        ch.done = true;
        let seq = ch.next_seq;
        ch.next_seq = ch.next_seq.saturating_add(1);
        ch.push(TokenChunk {
            seq,
            mote_id,
            text_piece: Vec::new(),
            done: true,
        });
    }

    /// Subscribe to `mote_id`'s tokens. Returns the history snapshot (everything
    /// emitted so far, for replay) PLUS a receiver for future chunks — both taken
    /// atomically under the lock, so a concurrent `publish` lands in exactly one
    /// of them (no gap, no duplicate at the split point). The caller emits the
    /// snapshot (filtered by its `since_seq`) then drains the receiver.
    pub(crate) fn subscribe(
        &self,
        mote_id: [u8; 32],
    ) -> (Vec<TokenChunk>, broadcast::Receiver<TokenChunk>) {
        let mut map = match self.inner.lock() {
            Ok(map) => map,
            Err(poisoned) => poisoned.into_inner(),
        };
        let ch = map.entry(mote_id).or_insert_with(MoteChannel::new);
        ch.last_touch = Instant::now();
        let snapshot: Vec<TokenChunk> = ch.history.iter().cloned().collect();
        let rx = ch.tx.subscribe();
        (snapshot, rx)
    }

    /// Sweep channels idle longer than `ttl`. A `done` channel with no live
    /// receivers is also reclaimed once idle. Called from a background tick.
    pub(crate) fn evict_idle(&self, ttl: Duration) {
        let Ok(mut map) = self.inner.lock() else {
            return;
        };
        let now = Instant::now();
        map.retain(|_, ch| {
            let idle = now.duration_since(ch.last_touch);
            // Keep if recently touched; drop stale entries (finished or not — a
            // stalled-then-abandoned generation is reclaimed by the same TTL).
            idle < ttl || ch.tx.receiver_count() > 0
        });
    }

    /// Number of live mote channels (test/diagnostic only).
    #[cfg(test)]
    pub(crate) fn channel_count(&self) -> usize {
        self.inner.lock().map(|m| m.len()).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mid(b: u8) -> [u8; 32] {
        [b; 32]
    }

    #[tokio::test]
    async fn publish_then_subscribe_replays_history_in_order() {
        let broker = TokenBroker::new();
        broker.publish(mid(1), b"he");
        broker.publish(mid(1), b"llo");
        // Subscribe AFTER two chunks were published — the snapshot replays them.
        let (snapshot, mut rx) = broker.subscribe(mid(1));
        let joined: Vec<u8> = snapshot.iter().flat_map(|c| c.text_piece.clone()).collect();
        assert_eq!(&joined, b"hello");
        assert_eq!(
            snapshot.iter().map(|c| c.seq).collect::<Vec<_>>(),
            vec![0, 1]
        );
        // A subsequent publish reaches the live receiver, NOT the snapshot.
        broker.publish(mid(1), b"!");
        let live = rx.recv().await.expect("live chunk");
        assert_eq!(live.seq, 2);
        assert_eq!(&live.text_piece, b"!");
        assert!(!live.done);
    }

    #[tokio::test]
    async fn finish_posts_terminal_done_and_is_idempotent() {
        let broker = TokenBroker::new();
        let (_snap, mut rx) = broker.subscribe(mid(7));
        broker.publish(mid(7), b"x");
        broker.finish(mid(7));
        broker.finish(mid(7)); // idempotent — no second done.
        let first = rx.recv().await.unwrap();
        assert_eq!(&first.text_piece, b"x");
        let done = rx.recv().await.unwrap();
        assert!(done.done);
        assert!(done.text_piece.is_empty());
        // No further chunks (the second finish was a no-op): the next recv errors
        // only once all senders drop, so assert no extra DONE by seq monotonicity.
        assert_eq!(done.seq, 1);
    }

    #[test]
    fn evict_idle_reclaims_stale_channels() {
        let broker = TokenBroker::new();
        broker.publish(mid(3), b"a");
        assert_eq!(broker.channel_count(), 1);
        // A zero TTL with no live receiver ⇒ reclaimed.
        broker.evict_idle(Duration::from_secs(0));
        assert_eq!(broker.channel_count(), 0);
    }

    #[test]
    fn distinct_motes_are_isolated() {
        let broker = TokenBroker::new();
        broker.publish(mid(1), b"one");
        broker.publish(mid(2), b"two");
        let (s1, _r1) = broker.subscribe(mid(1));
        let (s2, _r2) = broker.subscribe(mid(2));
        assert_eq!(&s1[0].text_piece, b"one");
        assert_eq!(&s2[0].text_piece, b"two");
    }
}
