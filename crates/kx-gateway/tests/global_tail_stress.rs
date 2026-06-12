//! Batch C exit gate — the slow-consumer fan-out stress envelope for the
//! GLOBAL live tail: **32 subscribers, 1 deliberately stalled, 10,000 commits**
//! over a real `SqliteJournal`. Pins, at scale, the LiveTailer policy the
//! global tailer inherits:
//! - every LIVE subscriber receives every delta to the final head;
//! - the STALLED subscriber buffers a BOUNDED number of frames (≤ its
//!   256-frame queue — at this envelope the 4096-delta frame chunking keeps it
//!   far below the cap, so the stream stays healthy by design; the
//!   queue-overflow → CatchupRequired terminal is pinned at the unit level in
//!   `live_tail.rs`, where a tiny channel can be driven directly) and a
//!   handover to a fresh stream from its last `next_seq` is loss-free;
//! - the journal writer is never blocked by subscribers (append throughput
//!   stays within a generous noise factor of the no-subscriber baseline —
//!   per-subscriber bounded queues are the structural guarantee).
//!
//! Run in release via `just scale-smoke` (`#[ignore]` keeps it out of the
//! default debug suite; the envelope is meaningless unoptimized).

#![cfg(feature = "embedded-worker")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use kx_content::ContentRef;
use kx_gateway::GlobalLiveTailer;
use kx_gateway_core::{GlobalEventTailer, JournalReader, ReadOnly};
use kx_journal::{Journal, JournalEntry, SqliteJournal, INSTANCE_ID_LEN};
use kx_mote::{MoteDefHash, MoteId, NdClass};
use smallvec::SmallVec;
use tokio::sync::watch;
use tokio::time::timeout;
use tonic::Code;

const SUBSCRIBERS: usize = 32;
const COMMITS: u64 = 10_000;

fn committed(n: u64) -> JournalEntry {
    let mut id = [0u8; 32];
    id[..8].copy_from_slice(&n.to_le_bytes());
    JournalEntry::Committed {
        mote_id: MoteId::from_bytes(id),
        idempotency_key: id,
        seq: 0,
        nondeterminism: NdClass::Pure,
        result_ref: ContentRef::from_bytes(id),
        parents: SmallVec::new(),
        warrant_ref: ContentRef::from_bytes([0xaa; 32]),
        mote_def_hash: MoteDefHash::from_bytes([0x09; 32]),
    }
}

fn append_run(journal: &SqliteJournal, offset: u64, n: u64) {
    for i in 0..n {
        journal.append(committed(offset + i)).unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "scale envelope — run in release via `just scale-smoke`"]
async fn fan_out_holds_the_envelope_with_a_stalled_subscriber() {
    let dir = tempfile::TempDir::new().unwrap();

    // Baseline: 10k appends with NO subscribers (a separate journal file so
    // the measured run starts cold-equal).
    let baseline_path = dir.path().join("baseline.db");
    let baseline_journal = SqliteJournal::open(&baseline_path).unwrap();
    baseline_journal
        .append(JournalEntry::RunRegistered {
            instance_id: [7; INSTANCE_ID_LEN],
            recipe_fingerprint: [8; 32],
            ts: 1,
            seq: 0,
        })
        .unwrap();
    let t0 = Instant::now();
    append_run(&baseline_journal, 0, COMMITS);
    let baseline = t0.elapsed();

    // The measured journal + a SHARED read-only handle (the serve shape: one
    // reader Arc backs every subscriber's poller).
    let path = dir.path().join("kx.db");
    let journal = SqliteJournal::open(&path).unwrap();
    journal
        .append(JournalEntry::RunRegistered {
            instance_id: [7; INSTANCE_ID_LEN],
            recipe_fingerprint: [8; 32],
            ts: 1,
            seq: 0,
        })
        .unwrap();
    let reader: Arc<dyn JournalReader> = Arc::new(ReadOnly::new(SqliteJournal::open(&path).unwrap()));
    let (_shutdown_tx, shutdown_rx) = watch::channel(false);
    let tailer = GlobalLiveTailer::new(shutdown_rx);

    // 31 live subscribers, each counting every delta to the final head; the
    // 32nd stream is NEVER polled (the stalled consumer).
    let final_head = 1 + COMMITS;
    let mut drains = Vec::new();
    for _ in 0..(SUBSCRIBERS - 1) {
        let mut stream = tailer.stream_all(reader.clone(), 0).unwrap();
        drains.push(tokio::spawn(async move {
            let mut seen = 0u64;
            while let Some(item) = stream.next().await {
                let frame = item.expect("a live subscriber never overflows");
                seen += frame.deltas.len() as u64;
                if frame.journal_boundary && frame.next_seq >= final_head {
                    break;
                }
            }
            seen
        }));
    }
    let mut stalled = tailer.stream_all(reader.clone(), 0).unwrap();

    // Drive the writer while the fan-out is live.
    let t1 = Instant::now();
    append_run(&journal, 0, COMMITS);
    let with_subs = t1.elapsed();

    // (a) Every live subscriber received EVERY delta (registration + commits).
    for drain in drains {
        let seen = timeout(Duration::from_secs(60), drain)
            .await
            .expect("live subscribers reach the head in time")
            .unwrap();
        assert_eq!(seen, final_head, "no delta lost on a live subscriber");
    }

    // (b) The stalled subscriber buffered a BOUNDED number of frames while the
    // whole run streamed past it (memory safety is per-frame, and the 4096-delta
    // chunking keeps frame counts tiny at this envelope), and a handover to a
    // fresh stream from its last delivered next_seq is loss-free: buffered
    // deltas + resumed deltas cover every seq exactly once.
    let mut buffered_frames = 0usize;
    let mut buffered_deltas = 0u64;
    let mut last_next = 0u64;
    loop {
        // The appends are done: anything still coming is already queued. A
        // quiet 2 s means the buffer is drained and the stream is simply OPEN
        // (healthy — it never overflowed at this envelope).
        match timeout(Duration::from_secs(2), stalled.next()).await {
            Ok(Some(Ok(frame))) => {
                buffered_frames += 1;
                buffered_deltas += frame.deltas.len() as u64;
                last_next = frame.next_seq;
                if frame.journal_boundary && frame.next_seq >= final_head {
                    break; // fully caught up through the buffer alone
                }
            }
            Ok(Some(Err(status))) => {
                // If the envelope ever DOES overflow the queue, the terminal
                // must be the CatchupRequired contract — never another error.
                assert_eq!(status.code(), Code::ResourceExhausted, "CatchupRequired");
                break;
            }
            Ok(None) => panic!("the stalled stream must not EOF mid-tail"),
            Err(_) => break, // quiet: buffer drained, stream open + healthy
        }
    }
    drop(stalled);
    assert!(
        buffered_frames <= 256,
        "the per-subscriber queue bound held (saw {buffered_frames} frames)"
    );
    if last_next < final_head {
        let mut resumed = tailer.stream_all(reader.clone(), last_next).unwrap();
        timeout(Duration::from_secs(60), async {
            while let Some(item) = resumed.next().await {
                let frame = item.expect("the resumed subscriber drains cleanly");
                buffered_deltas += frame.deltas.len() as u64;
                if frame.journal_boundary && frame.next_seq >= final_head {
                    break;
                }
            }
        })
        .await
        .expect("the resumed subscriber reaches the head");
    }
    assert_eq!(
        buffered_deltas, final_head,
        "buffered + resumed covers every delta exactly once"
    );

    // (c) The writer was never blocked by the fan-out. The bounded queues are
    // the structural guarantee; this asserts a GENEROUS ceiling so CI noise
    // (shared SQLite file locks, 31 concurrent readers) can't flake it while a
    // real regression (writer awaiting a subscriber) still fails loudly.
    let ceiling = baseline.checked_mul(4).unwrap();
    assert!(
        with_subs < ceiling,
        "append throughput within noise: baseline {baseline:?}, with subscribers {with_subs:?}"
    );
    println!(
        "fan-out envelope: {COMMITS} commits · {SUBSCRIBERS} subscribers (1 stalled) · \
         baseline {baseline:?} · with-subs {with_subs:?} · stalled buffered {buffered_frames} frames"
    );
}

/// GR10 M8a — commit→global-frame delivery latency through the 250 ms poll
/// cadence (p50/p95 over 40 rounds — the poll interval dominates by design;
/// the number pins the ceiling a push-based journal `watch` would beat).
/// Persisted to the private benchmarks trend file. (M8b — the telemetry sink's
/// per-event hot-path cost — is measured in `telemetry.rs`'s ignored unit
/// test, where the crate-private ledger is constructible.)
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "GR10 measurement — run in release via `just scale-smoke`"]
async fn m8a_commit_to_frame_latency() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("kx.db");
    let journal = SqliteJournal::open(&path).unwrap();
    journal
        .append(JournalEntry::RunRegistered {
            instance_id: [7; INSTANCE_ID_LEN],
            recipe_fingerprint: [8; 32],
            ts: 1,
            seq: 0,
        })
        .unwrap();
    let reader: Arc<dyn JournalReader> =
        Arc::new(ReadOnly::new(SqliteJournal::open(&path).unwrap()));
    let (_shutdown_tx, shutdown_rx) = watch::channel(false);
    let tailer = GlobalLiveTailer::new(shutdown_rx);
    let mut stream = tailer.stream_all(reader, 0).unwrap();
    // Drain the catch-up boundary.
    let _ = timeout(Duration::from_secs(5), stream.next()).await.unwrap();

    // M8a: one commit per round, time append→frame-delivery.
    let mut lat = Vec::with_capacity(40);
    for i in 0..40u64 {
        let t = Instant::now();
        journal.append(committed(100_000 + i)).unwrap();
        loop {
            let frame = timeout(Duration::from_secs(5), stream.next())
                .await
                .expect("frame within deadline")
                .unwrap()
                .unwrap();
            if !frame.deltas.is_empty() {
                break;
            }
        }
        lat.push(t.elapsed());
    }
    lat.sort();
    let p50 = lat[lat.len() / 2];
    let p95 = lat[lat.len() * 95 / 100];

    println!("GR10 M8a commit→frame latency p50 {p50:?} p95 {p95:?} (poll cadence 250ms)");
    assert!(
        p95 < Duration::from_millis(600),
        "latency within 2 polls + noise"
    );
}
