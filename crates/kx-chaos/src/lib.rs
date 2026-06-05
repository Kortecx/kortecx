//! # kx-chaos — the P3 exit-gate failure-injection harness (P3.7)
//!
//! > **Phase: distributed (P2/P3) — test harness.** Drives the coordinator/worker
//! > layer under injected faults; not part of the runtime you deploy, and not
//! > needed to build, run, or understand single-node kortecx. See the README (How it works).
//!
//! This crate is kortecx's **product core proof**: a *seed-deterministic* engine
//! that kills workers mid-Mote across a sweep of seeds and proves, for **every**
//! seed, the three guarantees the P3 exit gate names:
//!
//! 1. **Exactly-once** — a worker killed mid-WORLD-MUTATING Mote never produces a
//!    double world effect. The replacement either re-fires an *idempotent* effect
//!    (≤1 net effect, dedup at the tool boundary) or — when the recovery oracle
//!    cannot prove safety (P3.6c) — refuses to re-dispatch and leaves the Mote
//!    *safely stuck* rather than risk a double mutation.
//! 2. **No orphaned / duplicated children** — a topology shaper killed before it
//!    commits its decision commits *exactly once* (dedup, first-wins), and the
//!    children re-derived from that one committed decision are byte-identical to a
//!    clean run (child identity is a pure function of the committed decision).
//! 3. **Repudiation cascades correctly under chaos** — repudiating a root marks
//!    its entire committed downstream lineage, even when that lineage was assembled
//!    by a *replacement* worker after a death.
//!
//! ## Why deterministic, not autonomous
//!
//! The testing doctrine requires a *recorded, reproducible* seed: a failing run
//! must replay identically. Autonomous workers racing on the wall clock cannot give
//! that. So the harness derives a [`ChaosPlan`] purely from a `u64` seed (one
//! [`SplitMix64`] stream, consumed entirely at plan construction) and drives the
//! cluster through **direct, sequenced [`kx_coordinator::CoordinatorService`]
//! calls** on a single task, advancing an injected [`Clock`](kx_coordinator::Clock)
//! by hand. No `tokio::spawn` of competing workers, no real `sleep`, no gRPC. A
//! seed maps to one action+death sequence maps to one [`ChaosOutcome`].
//!
//! The frozen engine crates (`kx-scheduler` / `kx-executor` / `kx-inference`) are
//! never touched: the harness exercises the runtime only through the coordinator's
//! public surface and the real [`kx_runtime::topology::derive_child_motes`].
//!
//! ## Entry point
//!
//! [`run_seed`] is the whole API: build it a seed, get back `Ok(ChaosOutcome)` or an
//! [`ChaosFailure`] carrying the exact seed + plan + reason to reproduce.

// Unit tests inside `src/` are exempt from the safety lints (the workspace denies
// unwrap/expect/panic in library code); `tests/*.rs` carry their own per-file allows.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod assertions;
mod broker;
mod cluster;
mod plan;
mod prng;
mod scenario;
mod workflow;

pub use assertions::{ChaosFailure, ChaosOutcome};
pub use plan::{ChaosPlan, FaultPoint, ScenarioKind};
pub use prng::SplitMix64;
pub use scenario::run_seed;
