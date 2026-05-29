//! The P3 EXIT GATE — the product's core proof.
//!
//! Sweeps a fixed band of seeds; for **every** seed the harness kills a worker
//! mid-Mote (or races two) and proves the run upheld its exit-gate invariant:
//! exactly-once world effects, no orphaned/duplicated children after a shaper death,
//! and a correct repudiation cascade. A single failing seed fails the gate and prints
//! its own reproduction (`kx_chaos::run_seed(<seed>)`).
//!
//! This sweep runs inside `just ci` / `cargo test --workspace`. It is deterministic
//! and uses no wall-clock or filesystem, so it is fast and reproducible. The deeper
//! (millions-of-seeds) sweep lives in `seed_sweep.rs` behind `#[ignore]`.
//!
//! ## Test-plan (DoD) — every exit-gate requirement is covered by the sweep
//!
//! | Exit-gate requirement                         | Scenario                      | Asserted by                                   |
//! |-----------------------------------------------|-------------------------------|-----------------------------------------------|
//! | exactly-once (no double world mutation)       | `ExactlyOnce`                 | `net_effects == 1` across every fault + the safe-stuck refusal |
//! | recovery via re-dispatch (dedup bounds it)    | `ExactlyOnce`/`DeathBeforeCommit`/staged | committed + `dispatch_calls ≥ 2`     |
//! | P3.6c oracle refusal (no phantom re-fire)     | `ExactlyOnce`/`DeathBeforeCommit`/unstaged | `safely_stuck` + uncommitted + `net_effects == 1` |
//! | no orphaned / duplicated children             | `TopologyShaper`              | deterministic child set, `materialized == DEMO_WORKER_COUNT`, exactly-once commit |
//! | repudiation cascades correctly under chaos    | `RepudiationCascade`          | `cascade_size == 2`, all `Repudiated`, idempotent re-repudiate |

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::pedantic,
    clippy::panic
)]

/// The always-on band. 2048 seeds saturates the 3 scenarios × 3 faults × 3 WM patterns
/// grid many times over while staying sub-second (in-process, no real time).
const GATE_SEEDS: u64 = 2_048;

#[tokio::test(flavor = "current_thread")]
async fn p3_exit_gate_seed_sweep() {
    let mut proven = 0u64;
    for seed in 0..GATE_SEEDS {
        match kx_chaos::run_seed(seed).await {
            Ok(_) => proven += 1,
            Err(failure) => panic!("\n{failure}\n"),
        }
    }
    assert_eq!(proven, GATE_SEEDS, "every seed must prove its invariant");
}

/// Spot-reproduction: a handful of explicit seeds run individually, so a regression on
/// one prints in isolation (not buried in the sweep).
#[tokio::test(flavor = "current_thread")]
async fn reproduces_individual_seeds() {
    for seed in [0u64, 1, 2, 3, 7, 42, 99, 255, 1_000, 2_047] {
        kx_chaos::run_seed(seed)
            .await
            .unwrap_or_else(|f| panic!("\n{f}\n"));
    }
}

/// Non-vacuity guard: a passing sweep must actually witness each distinctive terminal
/// shape, or the gate would be green by doing nothing. Across the band we must see a
/// real re-dispatch after a death (`dispatch_calls ≥ 2`), the P3.6c safe-stuck refusal,
/// a materialized-children run, and a repudiation cascade. If a refactor ever made
/// worker death a silent no-op, this fails even though the invariants "hold".
#[tokio::test(flavor = "current_thread")]
async fn gate_exercises_every_terminal_shape() {
    let mut saw_redispatch = false;
    let mut saw_safe_stuck = false;
    let mut saw_children = false;
    let mut saw_cascade = false;
    for seed in 0..GATE_SEEDS {
        let o = kx_chaos::run_seed(seed)
            .await
            .unwrap_or_else(|f| panic!("\n{f}\n"));
        saw_redispatch |= o.dispatch_calls >= 2;
        saw_safe_stuck |= o.safely_stuck;
        saw_children |= o.materialized_children > 0;
        saw_cascade |= o.cascade_size == Some(2);
    }
    assert!(
        saw_redispatch,
        "no run re-dispatched after a death (death path not exercised)"
    );
    assert!(saw_safe_stuck, "no run hit the P3.6c safe-stuck refusal");
    assert!(saw_children, "no run materialized shaper children");
    assert!(saw_cascade, "no run exercised a repudiation cascade");
}
