//! The deep seed sweep — explicit, `#[ignore]`d (not run in `just ci`).
//!
//! Run it before a P3 sign-off / during a validation campaign to push the chaos gate
//! across millions of seeds:
//!
//! ```text
//! cargo test -p kx-chaos --test seed_sweep -- --ignored --nocapture
//! KX_CHAOS_SEEDS=5000000 cargo test -p kx-chaos --test seed_sweep -- --ignored --nocapture
//! ```
//!
//! It reuses the same deterministic `run_seed`, so any failure prints the exact seed to
//! drop into `kx_chaos::run_seed(<seed>)` for a one-shot reproduction.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::pedantic,
    clippy::panic
)]

/// Default depth when `KX_CHAOS_SEEDS` is unset. Overridable for a longer campaign.
const DEFAULT_DEEP_SEEDS: u64 = 1_000_000;

fn deep_seed_count() -> u64 {
    std::env::var("KX_CHAOS_SEEDS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_DEEP_SEEDS)
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "deep sweep — run explicitly before a P3 sign-off"]
async fn deep_seed_sweep() {
    let total = deep_seed_count();
    let mut proven = 0u64;
    for seed in 0..total {
        if let Err(failure) = kx_chaos::run_seed(seed).await {
            panic!("\n{failure}\n");
        }
        proven += 1;
        if proven.is_multiple_of(100_000) {
            eprintln!("kx-chaos deep sweep: {proven}/{total} seeds proven");
        }
    }
    eprintln!("kx-chaos deep sweep: {proven}/{total} seeds proven — all exactly-once");
    assert_eq!(proven, total);
}
