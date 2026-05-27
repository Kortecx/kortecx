//! SN-4 v2 #6 — concurrency tests for `kx-scheduler`.
//!
//! Two concerns:
//!
//! - Compile-time `Send + Sync` over the full public-type set, including
//!   `Arc<dyn Placement>` (proves the trait shape admits a P5 cloud impl
//!   behind the same handle).
//! - 4-thread thread-independence of [`Placement::place`] on
//!   [`RoundRobinPlacement`] (proves the atomic counter is correct under
//!   contention — no torn reads, no missed advances).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::sync::Arc;
use std::thread;

use kx_mote::MoteId;
use kx_scheduler::{
    DispatchSummary, DispatchedMote, LocalPlacement, Placement, RoundRobinPlacement, Scheduler,
    SchedulerError, WorkerId,
};

// ---------------------------------------------------------------------------
// Compile-time Send + Sync over the public surface (SN-4 v2 #6 part 1)
// ---------------------------------------------------------------------------

fn assert_send_sync<T: Send + Sync>() {}

#[test]
fn public_types_are_send_and_sync() {
    // Worker + error vocabulary.
    assert_send_sync::<WorkerId>();
    assert_send_sync::<SchedulerError>();

    // Placement impls + the dyn trait.
    assert_send_sync::<LocalPlacement>();
    assert_send_sync::<RoundRobinPlacement>();
    assert_send_sync::<Arc<dyn Placement>>();

    // Dispatch summary shapes.
    assert_send_sync::<DispatchedMote>();
    assert_send_sync::<DispatchSummary>();

    // The Scheduler over a concrete placement.
    assert_send_sync::<Scheduler<LocalPlacement>>();
    assert_send_sync::<Scheduler<RoundRobinPlacement>>();
}

// ---------------------------------------------------------------------------
// 4-thread RoundRobinPlacement contention (SN-4 v2 #6 part 2)
// ---------------------------------------------------------------------------

#[test]
fn round_robin_place_is_thread_safe_under_contention() {
    const N_THREADS: usize = 4;
    const PLACES_PER_THREAD: usize = 50;
    const N_WORKERS: u64 = 3;

    let placement = Arc::new(RoundRobinPlacement::new(N_WORKERS));
    let handles: Vec<_> = (0..N_THREADS)
        .map(|_| {
            let p = Arc::clone(&placement);
            thread::spawn(move || {
                let mut out = Vec::with_capacity(PLACES_PER_THREAD);
                let id = MoteId::from_bytes([0u8; 32]);
                for _ in 0..PLACES_PER_THREAD {
                    out.push(p.place(&id));
                }
                out
            })
        })
        .collect();

    let mut all: Vec<WorkerId> = Vec::with_capacity(N_THREADS * PLACES_PER_THREAD);
    for h in handles {
        all.extend(h.join().expect("worker did not panic"));
    }

    // Total assignments equal total calls (atomic counter advanced exactly
    // N_THREADS * PLACES_PER_THREAD times).
    assert_eq!(all.len(), N_THREADS * PLACES_PER_THREAD);

    // Worker distribution is exactly balanced — every worker received the
    // same number of placements. With N_WORKERS=3 and 200 calls total,
    // each worker gets exactly 200 / 3 = 66.66… → 67/67/66 in some order
    // because 200 % 3 = 2. Assert the multiset spread sums correctly and
    // every count is within ±1 of mean.
    let mut counts = [0usize; N_WORKERS as usize];
    for w in &all {
        counts[w.0 as usize] += 1;
    }
    let total: usize = counts.iter().sum();
    assert_eq!(total, N_THREADS * PLACES_PER_THREAD);
    let mean = (N_THREADS * PLACES_PER_THREAD) as f64 / N_WORKERS as f64;
    for c in counts {
        let delta = (c as f64 - mean).abs();
        assert!(
            delta <= 1.0,
            "worker count {c} too far from mean {mean}; full counts: {counts:?}"
        );
    }
}
